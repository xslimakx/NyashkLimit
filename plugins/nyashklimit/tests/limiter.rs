//! The limiter must be a brick wall (sample peak and true peak), transparent below the ceiling,
//! report its latency exactly, and every option must do what it says.

use nyashklimit::dither::Shaping;
use nyashklimit::limiter::{Limiter, Params, Style};
use nyashklimit::loudness::Loudness;
use nyashklimit::os::lowpass;

const FS: f32 = 48000.0;

struct Rng(u64);
impl Rng {
    fn f(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f32 / (1u64 << 53) as f32 * 2.0 - 1.0
    }
}

/// A hard test signal: drum-like hits, a bass line, bright noise bursts and a full-scale square
/// burst (inter-sample peaks), stereo with different content per side.
fn program(secs: f32, seed: u64) -> Vec<[f32; 2]> {
    let n = (secs * FS) as usize;
    let mut r = Rng(seed);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / FS;
        let beat = (t * 2.0).fract();
        let kick = (-(beat * 30.0)).exp() * (2.0 * std::f32::consts::PI * (50.0 + 120.0 * (-(beat * 40.0)).exp()) * t).sin();
        let snare = if (t * 2.0 + 0.5).fract() < 0.08 { r.f() * (1.0 - (t * 2.0 + 0.5).fract() / 0.08) } else { 0.0 };
        let bass = 0.4 * (2.0 * std::f32::consts::PI * 55.0 * t).sin();
        let hat = 0.2 * r.f() * if (t * 8.0).fract() < 0.03 { 1.0 } else { 0.0 };
        // a full-scale sine at fs/4 with a phase offset: its samples are 0.707 but the true peak is 1
        let isp = if (t % 3.0) > 2.5 { (2.0 * std::f32::consts::PI * 12000.0 * t + 0.785).sin() } else { 0.0 };
        let l = 0.7 * kick + 0.6 * snare + bass + hat + 0.8 * isp;
        let rr = 0.7 * kick + 0.4 * snare + 0.8 * bass + 1.2 * hat + 0.8 * isp;
        out.push([l * 0.9, rr * 0.9]);
    }
    out
}

fn run(p: &Params, x: &[[f32; 2]]) -> (Vec<[f32; 2]>, usize) {
    let mut l = Limiter::new(FS, 2);
    l.set_params(p);
    let lat = l.latency();
    let mut y = Vec::with_capacity(x.len() + lat);
    for f in x.iter().copied().chain(std::iter::repeat([0.0, 0.0]).take(lat)) {
        y.push(l.process(f, [0.0, 0.0]));
    }
    (y[lat..].to_vec(), lat)
}

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-12).log10()
}

/// independent true-peak measurement: 16x interpolation with a long filter
fn true_peak(x: &[f32]) -> f32 {
    let l = 16;
    let taps = 32;
    let h = lowpass(taps * l + 1, 0.49 / l as f64, 10.0);
    let mut m = 0.0f32;
    for n in taps..x.len() {
        for ph in 0..l {
            let mut acc = 0.0f64;
            let mut j = 0;
            while ph + j * l < h.len() {
                acc += h[ph + j * l] * l as f64 * x[n - j] as f64;
                j += 1;
            }
            m = m.max(acc.abs() as f32);
        }
    }
    m
}

fn base() -> Params {
    Params { gain_db: 12.0, out_db: -1.0, true_peak: false, oversampling: 1, ..Params::default() }
}

#[test]
fn sample_peak_never_over_the_ceiling() {
    let x = program(4.0, 7);
    for style in Style::ALL {
        for os in [1usize, 4, 32] {
            for look in [0.0f32, 1.0, 5.0] {
                let p = Params { style, oversampling: os, lookahead_ms: look, ..base() };
                let (y, _) = run(&p, &x);
                let pk = y.iter().flat_map(|f| f.iter()).fold(0.0f32, |a, v| a.max(v.abs()));
                assert!(db(pk) <= -1.0 + 0.001, "{style:?} os {os} look {look}: peak {:.3} dB", db(pk));
            }
        }
    }
}

#[test]
fn true_peak_never_over_the_ceiling() {
    let x = program(4.0, 11);
    for style in Style::ALL {
        for os in [1usize, 4, 8] {
            let p = Params { style, oversampling: os, true_peak: true, ..base() };
            let (y, _) = run(&p, &x);
            for c in 0..2 {
                let ch: Vec<f32> = y.iter().map(|f| f[c]).collect();
                let tp = db(true_peak(&ch));
                eprintln!("{style:?} os {os} ch {c}: true peak {tp:.3} dBTP");
                assert!(tp <= -1.0 + 0.1, "{style:?} os {os}: true peak {tp:.3} dBTP over -1");
            }
        }
    }
}

#[test]
fn transparent_below_the_ceiling() {
    // a quiet signal never touches the limiter: the output is the input, delayed. (Content up to
    // 16 kHz: the oversampling filters roll off above ~20 kHz, like any oversampled limiter.)
    let x: Vec<[f32; 2]> = (0..(2.0 * FS) as usize)
        .map(|i| {
            let t = i as f32 / FS;
            let v = [55.0f32, 220.0, 1234.0, 5000.0, 9100.0, 15800.0].iter().enumerate().map(|(k, f)| (2.0 * std::f32::consts::PI * f * t + k as f32).sin()).sum::<f32>() * 0.05;
            [v, -v]
        })
        .collect();
    for os in [1usize, 4, 32] {
        let p = Params { gain_db: 0.0, out_db: 0.0, oversampling: os, true_peak: false, ..Params::default() };
        let (y, _) = run(&p, &x);
        let (mut e, mut s) = (0.0f64, 0.0f64);
        for (a, b) in x.iter().zip(y.iter()).skip(4800) {
            for c in 0..2 {
                e += ((a[c] - b[c]) as f64).powi(2);
                s += (a[c] as f64).powi(2);
            }
        }
        let snr = 10.0 * (s / e.max(1e-30)).log10();
        eprintln!("os {os}: transparency {snr:.1} dB");
        assert!(snr > 55.0, "os {os}: only {snr:.1} dB");
    }
}

#[test]
fn latency_is_exact() {
    for (os, tp) in [(1usize, false), (4, false), (32, false), (1, true), (16, true)] {
        let p = Params { gain_db: 0.0, out_db: 0.0, oversampling: os, true_peak: tp, ..Params::default() };
        let mut l = Limiter::new(FS, 2);
        l.set_params(&p);
        let lat = l.latency();
        let mut best = (0usize, 0.0f32);
        for i in 0..lat + 200 {
            let v = if i == 0 { 0.5 } else { 0.0 };
            let y = l.process([v, v], [0.0, 0.0]);
            if y[0].abs() > best.1 {
                best = (i, y[0].abs());
            }
        }
        assert_eq!(best.0, lat, "os {os} tp {tp}: impulse at {} not at the reported {lat}", best.0);
    }
}

#[test]
fn styles_differ_and_aggressive_is_loudest() {
    let x = program(4.0, 5);
    let rms = |y: &[[f32; 2]]| (y.iter().map(|f| (f[0] as f64).powi(2)).sum::<f64>() / y.len() as f64).sqrt();
    let mut vals = vec![];
    for style in Style::ALL {
        let (y, _) = run(&Params { style, gain_db: 10.0, ..base() }, &x);
        vals.push((style, 20.0 * rms(&y).log10()));
    }
    for (s, v) in &vals {
        eprintln!("{s:?}: {v:.2} dB RMS");
    }
    let agg = vals.iter().find(|v| v.0 == Style::Aggressive).unwrap().1;
    let safe = vals.iter().find(|v| v.0 == Style::Safe).unwrap().1;
    assert!(agg > safe + 0.3, "aggressive {agg:.2} vs safe {safe:.2}");
    // all different
    for i in 0..vals.len() {
        for j in i + 1..vals.len() {
            assert!((vals[i].1 - vals[j].1).abs() > 0.005, "{:?} and {:?} sound the same", vals[i].0, vals[j].0);
        }
    }
}

#[test]
fn unity_gain_and_audition() {
    let x = program(2.0, 9);
    // Unity: the ceiling becomes -Gain
    let (y, _) = run(&Params { unity_gain: true, gain_db: 6.0, ..base() }, &x);
    let pk = y.iter().flat_map(|f| f.iter()).fold(0.0f32, |a, v| a.max(v.abs()));
    assert!(db(pk) <= -6.0 + 0.001, "unity: peak {:.2}", db(pk));
    // Audition: what the limiter removes, at the source level: (dry*gain - out) / gain
    let p = Params { gain_db: 6.0, ..base() };
    let (y_out, _) = run(&p, &x);
    let (y_aud, _) = run(&Params { audition: true, ..p }, &x);
    let g = 10f32.powf(6.0 / 20.0);
    let mut err = 0.0f32;
    for i in 0..x.len() {
        err = err.max((y_aud[i][0] - (x[i][0] * g - y_out[i][0]) / g).abs());
    }
    assert!(err < 1e-4, "audition is not (dry*gain - out)/gain: {err}");
}

#[test]
fn sidechain_drives_the_limiter() {
    let n = (FS * 1.0) as usize;
    let mut l = Limiter::new(FS, 2);
    l.set_params(&Params { sidechain: true, gain_db: 0.0, out_db: -6.0, ..base() });
    let mut late = 0.0f32;
    for i in 0..n {
        let v = 0.1 * (i as f32 * 0.05).sin(); // quiet main signal
        let sc = if i > n / 2 { 0.9 } else { 0.0 }; // loud key in the second half
        let y = l.process([v, v], [sc, sc]);
        if i > n * 3 / 4 {
            late = late.max(y[0].abs());
        }
    }
    assert!(late < 0.1 * 0.7, "the side chain did not pull the quiet signal down: {late}");
}

#[test]
fn dither_quantises_and_shapes() {
    let n = 1 << 16;
    let x: Vec<[f32; 2]> = (0..n).map(|i| [0.25 * (i as f32 * 0.013).sin(); 2]).collect();
    let mut levels = vec![];
    for sh in [Shaping::None, Shaping::Optimized] {
        let p = Params { gain_db: 0.0, out_db: 0.0, dither_bits: 16, shaping: sh, oversampling: 1, true_peak: false, ..Params::default() };
        let (y, lat) = run(&p, &x);
        let _ = lat;
        // on the 16-bit grid
        for f in y.iter().take(5000) {
            let q = f[0] * 32768.0;
            assert!((q - q.round()).abs() < 1e-3, "not on the 16 bit grid: {}", f[0]);
        }
        // error energy below 4 kHz (DFT bins) - shaping must move it up
        let err: Vec<f64> = y.iter().zip(x.iter()).map(|(a, b)| (a[0] - b[0]) as f64).collect();
        let m = 8192;
        let mut lo = 0.0;
        for k in 1..(4000.0 / FS * m as f32) as usize {
            let (mut re, mut im) = (0.0, 0.0);
            for (i, e) in err.iter().skip(10000).take(m).enumerate() {
                let ph = 2.0 * std::f64::consts::PI * (k * i) as f64 / m as f64;
                re += e * ph.cos();
                im += e * ph.sin();
            }
            lo += re * re + im * im;
        }
        levels.push(lo);
    }
    assert!(levels[1] < levels[0] * 0.5, "noise shaping did not lower the low/mid noise: {levels:?}");
}

#[test]
fn loudness_matches_bs1770() {
    // 997 Hz sine at -20 dBFS in both channels = -20.0 LUFS (BS.1770 / EBU Tech 3341)
    let mut ld = Loudness::new(FS);
    for i in 0..(FS * 10.0) as usize {
        let v = 0.1 * (2.0 * std::f32::consts::PI * 997.0 * i as f32 / FS).sin();
        ld.push(v, v, 2);
    }
    eprintln!("M {:.2} S {:.2} I {:.2}", ld.momentary, ld.short_term, ld.integrated);
    for v in [ld.momentary, ld.short_term, ld.integrated] {
        assert!((v + 20.0).abs() < 0.1, "{v:.2} LUFS, want -20.0");
    }
    // silence after it does not pull Integrated down (absolute gate)
    for _ in 0..(FS * 10.0) as usize {
        ld.push(0.0, 0.0, 2);
    }
    assert!((ld.integrated + 20.0).abs() < 0.1, "gating: {:.2}", ld.integrated);
}

#[test]
fn loudness_range_matches_tech3342() {
    // EBU Tech 3343 case: 20 s at -20 dBFS then 20 s at -30 dBFS (1 kHz, stereo) -> LRA 10 LU
    let mut ld = Loudness::new(FS);
    for i in 0..(FS * 40.0) as usize {
        let a = if i < (FS * 20.0) as usize { 0.1 } else { 0.0316 };
        let v = a * (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / FS).sin();
        ld.push(v, v, 2);
    }
    eprintln!("LRA {:.2}", ld.lra);
    assert!((ld.lra - 10.0).abs() < 1.0, "LRA {:.2}, want 10 +- 1", ld.lra);
}

