//! Robustness checks: sample rates, mono, bad input, extremes, automation, long runs.
use nyashklimit::dither::Shaping;
use nyashklimit::limiter::{Limiter, Params, Style};

struct Rng(u64);
impl Rng {
    fn f(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0
    }
}

fn program(fs: f32, secs: f32, seed: u64) -> Vec<[f32; 2]> {
    let n = (secs * fs) as usize;
    let mut r = Rng(seed);
    (0..n)
        .map(|i| {
            let t = i as f32 / fs;
            let beat = (t * 2.0).fract();
            let kick = (-(beat * 30.0)).exp() * (std::f32::consts::TAU * (50.0 + 120.0 * (-(beat * 40.0)).exp()) * t).sin();
            let n = r.f();
            let hat = 0.3 * n * if (t * 8.0).fract() < 0.03 { 1.0 } else { 0.0 };
            let hi = 0.5 * (std::f32::consts::TAU * (fs * 0.23) * t + 0.7).sin() * if (t % 1.0) > 0.8 { 1.0 } else { 0.0 };
            let b = 0.4 * (std::f32::consts::TAU * 55.0 * t).sin();
            [0.8 * kick + b + hat + hi, 0.7 * kick + 0.9 * b - hat + 0.8 * hi]
        })
        .collect()
}

fn base() -> Params {
    Params { gain_db: 12.0, out_db: -1.0, true_peak: false, oversampling: 1, ..Params::default() }
}

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-12).log10()
}

fn run_with(l: &mut Limiter, x: &[[f32; 2]], nch: usize) -> Vec<[f32; 2]> {
    let lat = l.latency();
    let mut y = Vec::with_capacity(x.len() + lat);
    for f in x.iter().copied().chain(std::iter::repeat([0.0, 0.0]).take(lat)) {
        let mut i = f;
        if nch == 1 {
            i[1] = 0.0;
        }
        y.push(l.process(i, [0.0; 2]));
    }
    y.drain(..lat);
    y
}

/// cheap true-peak check: 8x windowed-sinc interpolation
fn tp8(x: &[f32]) -> f32 {
    let l = 8;
    let taps = 64;
    let h = nyashklimit::os::lowpass(taps * l + 1, 0.5 / l as f64, 9.0);
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

#[test]
fn all_sample_rates_mono_stereo() {
    for fs in [44100.0f32, 48000.0, 88200.0, 96000.0, 192000.0] {
        let x = program(fs, 1.5, 3);
        for nch in [1usize, 2] {
            for style in Style::ALL {
                for (os, tp) in [(1usize, false), (1, true), (2, false), (32, true)] {
                    if fs > 100_000.0 && os == 32 {
                        continue;
                    }
                    let mut l = Limiter::new(fs, nch);
                    l.set_params(&Params { style, oversampling: os, true_peak: tp, ..base() });
                    let y = run_with(&mut l, &x, nch);
                    for c in 0..nch {
                        let ch: Vec<f32> = y.iter().map(|f| f[c]).collect();
                        assert!(ch.iter().all(|v| v.is_finite()), "{fs} {nch}ch {style:?} os{os}: not finite");
                        let sp = db(ch.iter().fold(0.0f32, |a, v| a.max(v.abs())));
                        assert!(sp <= -1.0 + 0.01, "{fs} {nch}ch {style:?} os{os} tp{tp}: sample peak {sp:.3}");
                        if tp && style == Style::Modern {
                            let t = db(tp8(&ch));
                            assert!(t <= -1.0 + 0.1, "{fs} {nch}ch os{os}: true peak {t:.3}");
                        }
                        // the limiter must actually work: it gets loud
                        assert!(sp > -2.5, "{fs} {nch}ch {style:?} os{os}: too quiet {sp:.2}");
                    }
                    if nch == 1 {
                        assert!(y.iter().all(|f| f[1] == 0.0), "mono: second channel must stay silent");
                    }
                }
            }
        }
    }
}

#[test]
fn silence_stays_silent_and_tiny_input_is_fast() {
    let mut l = Limiter::new(48000.0, 2);
    l.set_params(&Params { oversampling: 8, true_peak: true, ..base() });
    let y = run_with(&mut l, &vec![[0.0; 2]; 48000], 2);
    assert!(y.iter().all(|f| f[0] == 0.0 && f[1] == 0.0), "silence in, silence out");
    // denormal-range input must not slow the limiter down a lot
    let tiny = vec![[1e-38f32, -1e-38]; 480000];
    let t0 = std::time::Instant::now();
    let _ = run_with(&mut l, &tiny, 2);
    let slow = t0.elapsed().as_secs_f64();
    let norm = vec![[0.1f32, -0.1]; 480000];
    let t1 = std::time::Instant::now();
    let _ = run_with(&mut l, &norm, 2);
    let fast = t1.elapsed().as_secs_f64();
    eprintln!("tiny {slow:.3}s normal {fast:.3}s");
    assert!(slow < fast * 3.0, "denormals: {slow:.3}s vs {fast:.3}s");
}

#[test]
fn recovers_from_nan_and_inf() {
    let x = program(48000.0, 2.0, 5);
    let mut l = Limiter::new(48000.0, 2);
    l.set_params(&Params { oversampling: 4, true_peak: true, ..base() });
    let mut bad = x.clone();
    for i in 1000..1010 {
        bad[i] = [f32::NAN, f32::INFINITY];
    }
    let y = run_with(&mut l, &bad, 2);
    // after one second everything must be fine again
    let tail = &y[48000..];
    assert!(tail.iter().all(|f| f[0].is_finite() && f[1].is_finite()), "NaN poisoned the state");
    assert!(tail.iter().any(|f| f[0].abs() > 0.1), "output dead after NaN");
}

#[test]
fn extremes_are_safe() {
    let x = program(48000.0, 2.0, 9);
    let mut cases = vec![];
    for look in [0.0f32, 5.0] {
        for att in [0.0f32, 1000.0] {
            for rel in [1.0f32, 5000.0] {
                for link in [0.0f32, 1.0] {
                    cases.push(Params { gain_db: 30.0, out_db: -30.0, lookahead_ms: look, attack_ms: att, release_ms: rel, link_transient: link, link_release: 1.0 - link, ..base() });
                    cases.push(Params { gain_db: 30.0, out_db: 0.0, lookahead_ms: look, attack_ms: att, release_ms: rel, link_transient: link, link_release: link, oversampling: 4, true_peak: true, ..base() });
                }
            }
        }
    }
    for p in cases {
        for style in Style::ALL {
            let p = Params { style, ..p };
            let mut l = Limiter::new(48000.0, 2);
            l.set_params(&p);
            let y = run_with(&mut l, &x, 2);
            let sp = y.iter().fold(0.0f32, |a, f| a.max(f[0].abs()).max(f[1].abs()));
            assert!(sp.is_finite() && db(sp) <= p.out_db + 0.01, "{style:?} {p:?}: peak {:.3}", db(sp));
        }
    }
}

#[test]
fn automation_mid_stream() {
    // every parameter is changed every 50 ms while music plays
    let fs = 48000.0;
    let x = program(fs, 6.0, 13);
    let mut l = Limiter::new(fs, 2);
    let mut r = Rng(99);
    let mut p = base();
    l.set_params(&p);
    let mut y = vec![];
    let mut worst = 0.0f32;
    for (i, f) in x.iter().enumerate() {
        if i % 2400 == 0 {
            p.style = Style::ALL[((r.f() * 0.5 + 0.5) * 7.99) as usize];
            p.lookahead_ms = (r.f() * 0.5 + 0.5) * 5.0;
            p.attack_ms = (r.f() * 0.5 + 0.5) * 200.0;
            p.release_ms = 1.0 + (r.f() * 0.5 + 0.5) * 1000.0;
            p.gain_db = 6.0 + (r.f() * 0.5 + 0.5) * 12.0;
            p.link_transient = r.f() * 0.5 + 0.5;
            p.link_release = r.f() * 0.5 + 0.5;
            if i % 24000 == 0 {
                p.oversampling = [1, 2, 4, 8][((r.f() * 0.5 + 0.5) * 3.99) as usize];
                p.true_peak = r.f() > 0.0;
            }
            l.set_params(&p);
        }
        let o = l.process(*f, [0.0; 2]);
        worst = worst.max(o[0].abs()).max(o[1].abs());
        y.push(o);
    }
    assert!(y.iter().all(|f| f[0].is_finite() && f[1].is_finite()));
    assert!(db(worst) <= -1.0 + 0.01, "peak during automation {:.3}", db(worst));
}

#[test]
fn bypass_and_oversampling_switch_without_clicks() {
    let fs = 48000.0;
    // a low sine: any jump bigger than its slope is a click
    let n = (fs * 3.0) as usize;
    let x: Vec<[f32; 2]> = (0..n).map(|i| {
        let v = 0.5 * (std::f32::consts::TAU * 100.0 * i as f32 / fs).sin();
        [v, v]
    }).collect();
    let max_step = 0.5 * std::f32::consts::TAU * 100.0 / fs;
    for what in ["bypass", "os", "tp", "style", "dither"] {
        let mut l = Limiter::new(fs, 2);
        let mut p = Params { gain_db: 3.0, out_db: -1.0, ..base() };
        l.set_params(&p);
        let mut prev = 0.0f32;
        let mut worst = 0.0f32;
        for (i, f) in x.iter().enumerate() {
            if i > 0 && i % 24000 == 0 {
                match what {
                    "bypass" => p.bypass = !p.bypass,
                    "os" => p.oversampling = if p.oversampling == 1 { 8 } else { 1 },
                    "tp" => p.true_peak = !p.true_peak,
                    "style" => p.style = if p.style == Style::Modern { Style::Safe } else { Style::Modern },
                    _ => p.dither_bits = if p.dither_bits == 0 { 16 } else { 0 },
                }
                l.set_params(&p);
            }
            let o = l.process(*f, [0.0; 2])[0];
            if i > 48000 / 10 {
                worst = worst.max((o - prev).abs());
            }
            prev = o;
        }
        eprintln!("{what}: worst step {worst:.4} (sine slope {max_step:.4})");
        let allowed = 1.6;
        assert!(worst < max_step * allowed, "{what}: click, step {worst:.4} vs {max_step:.4}");
    }
}

#[test]
fn long_run_is_stable() {
    // 10 minutes of a steady loud sine: the gain reduction must not drift
    let fs = 48000.0;
    let mut l = Limiter::new(fs, 2);
    l.set_params(&Params { style: Style::Transparent, ..base() });
    let w = std::f32::consts::TAU * 220.0 / fs;
    let mut first = 0.0f32;
    let mut last = 0.0f32;
    let n = (fs * 600.0) as usize;
    let mut ph = 0.0f32;
    for i in 0..n {
        ph += w;
        if ph > std::f32::consts::TAU {
            ph -= std::f32::consts::TAU;
        }
        let v = 0.8 * ph.sin();
        let o = l.process([v, v], [0.0; 2])[0].abs();
        if i > (fs * 5.0) as usize && i < (fs * 10.0) as usize {
            first = first.max(o);
        }
        if i > n - (fs * 5.0) as usize {
            last = last.max(o);
        }
    }
    eprintln!("start {:.4} dB end {:.4} dB", db(first), db(last));
    assert!((db(first) - db(last)).abs() < 0.05, "drift: {:.3} -> {:.3}", db(first), db(last));
}

#[test]
fn dither_never_breaks_the_ceiling() {
    let x = program(48000.0, 2.0, 21);
    for bits in [16u32, 18, 20, 24] {
        for sh in [Shaping::None, Shaping::Basic, Shaping::Optimized, Shaping::Weighted] {
            let mut l = Limiter::new(48000.0, 2);
            l.set_params(&Params { out_db: -0.1, dither_bits: bits, shaping: sh, ..base() });
            let y = run_with(&mut l, &x, 2);
            let sp = y.iter().fold(0.0f32, |a, f| a.max(f[0].abs()).max(f[1].abs()));
            assert!(sp <= 1.0, "{bits} {sh:?}: {sp}");
        }
    }
}

#[test]
fn recovers_after_nan_or_inf_from_the_host() {
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        for tp in [false, true] {
            let mut l = Limiter::new(48000.0, 2);
            l.set_params(&Params { true_peak: tp, ..base() });
            for n in 0..96000 {
                let v = if (100..110).contains(&n) { bad } else { 0.5 * (n as f32 * 0.05).sin() };
                let sc = if (200..205).contains(&n) { bad } else { 0.0 };
                let y = l.process([v, v], [sc, sc]);
                if n > 48000 {
                    assert!(y[0].is_finite() && y[1].is_finite() && y[0].abs() < 1.0, "{bad} tp {tp}: n {n} {y:?}");
                }
            }
        }
    }
}
