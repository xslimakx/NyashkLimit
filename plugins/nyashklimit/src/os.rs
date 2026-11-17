//! Linear-phase polyphase oversampling (1x .. 32x) and a true-peak interpolator.
//!
//! One windowed-sinc low-pass of `taps * factor + 1` taps (Kaiser window) is used both to
//! upsample (zero-stuffing + filtering, done as a polyphase filter) and to downsample (filter +
//! decimation). Each filter delays by `taps * factor / 2` samples at the high rate, so up + down
//! together delay by exactly `taps` samples at the base rate.

use std::f64::consts::PI;

fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let q = x * x / 4.0;
    for k in 1..60 {
        term *= q / (k * k) as f64;
        sum += term;
        if term < 1e-14 * sum {
            break;
        }
    }
    sum
}

/// Windowed-sinc low-pass, `n` taps (odd), cutoff `fc` as a fraction of the sample rate,
/// Kaiser `beta`. Normalised to unity DC gain.
pub fn lowpass(n: usize, fc: f64, beta: f64) -> Vec<f64> {
    let m = (n - 1) as f64 / 2.0;
    let i0b = bessel_i0(beta);
    let mut h: Vec<f64> = (0..n)
        .map(|i| {
            let t = i as f64 - m;
            let sinc = if t.abs() < 1e-12 { 2.0 * fc } else { (2.0 * PI * fc * t).sin() / (PI * t) };
            let r = t / m;
            let w = bessel_i0(beta * (1.0 - r * r).max(0.0).sqrt()) / i0b;
            sinc * w
        })
        .collect();
    let s: f64 = h.iter().sum();
    h.iter_mut().for_each(|v| *v /= s);
    h
}

/// Taps per phase for a given factor.
pub fn taps_for(factor: usize) -> usize {
    match factor {
        1 => 0,
        2 => 32,
        // the same 24 taps per phase from 4x up: the filter's transition band (in base-rate Hz)
        // stays the same, so 8x..32x keep the full audio band (flat to ~17 kHz at 48 kHz)
        _ => 24,
    }
}

/// Upsampler + downsampler for one channel.
pub struct Oversampler {
    pub factor: usize,
    taps: usize,
    /// polyphase up filter: phases[p][j] = h[p + j*L] * L
    phases: Vec<Vec<f32>>,
    down: Vec<f32>,
    hist_in: Vec<f32>, // base-rate input history (ring, doubled for contiguous reads)
    hi: usize,
    hist_os: Vec<f32>, // high-rate history for the decimator (ring, doubled)
    ho: usize,
}

impl Oversampler {
    pub fn new(factor: usize) -> Self {
        let taps = taps_for(factor);
        if factor <= 1 {
            return Self { factor: 1, taps: 0, phases: vec![], down: vec![], hist_in: vec![], hi: 0, hist_os: vec![], ho: 0 };
        }
        let n = taps * factor + 1;
        // pass band up to ~0.45 of the base rate, stop band from 0.5
        let fc = 0.475 / factor as f64;
        let h = lowpass(n, fc, 9.0);
        let mut phases = vec![Vec::new(); factor];
        for (p, ph) in phases.iter_mut().enumerate() {
            let mut j = 0;
            while p + j * factor < n {
                ph.push((h[p + j * factor] * factor as f64) as f32);
                j += 1;
            }
        }
        let down: Vec<f32> = h.iter().map(|v| *v as f32).collect();
        let lin = taps + 2;
        Self { factor, taps, phases, down, hist_in: vec![0.0; 2 * lin], hi: 0, hist_os: vec![0.0; 2 * (n + factor)], ho: 0 }
    }

    /// Latency at the base rate (up + down).
    pub fn latency(&self) -> usize {
        self.taps
    }

    pub fn reset(&mut self) {
        self.hist_in.iter_mut().for_each(|v| *v = 0.0);
        self.hist_os.iter_mut().for_each(|v| *v = 0.0);
        self.hi = 0;
        self.ho = 0;
    }

    /// One base-rate sample in, `factor` high-rate samples out.
    #[inline]
    pub fn up(&mut self, x: f32, out: &mut [f32]) {
        if self.factor == 1 {
            out[0] = x;
            return;
        }
        let lin = self.hist_in.len() / 2;
        self.hi = if self.hi == 0 { lin - 1 } else { self.hi - 1 };
        self.hist_in[self.hi] = x;
        self.hist_in[self.hi + lin] = x;
        let hist = &self.hist_in[self.hi..self.hi + lin];
        for (p, ph) in self.phases.iter().enumerate() {
            let mut acc = 0.0f32;
            for (c, v) in ph.iter().zip(hist.iter()) {
                acc += c * v;
            }
            out[p] = acc;
        }
    }

    /// `factor` high-rate samples in, one base-rate sample out.
    #[inline]
    pub fn down(&mut self, inp: &[f32]) -> f32 {
        if self.factor == 1 {
            return inp[0];
        }
        let n = self.hist_os.len() / 2;
        for &v in inp.iter() {
            self.ho = if self.ho == 0 { n - 1 } else { self.ho - 1 };
            self.hist_os[self.ho] = v;
            self.hist_os[self.ho + n] = v;
        }
        // y[k] = sum h[j] s[kL - j]: start at the first of the L new samples, so the
        // filter delay stays a whole number of base-rate samples
        let off = self.factor - 1;
        let hist = &self.hist_os[self.ho + off..self.ho + off + self.down.len()];
        let mut acc = 0.0f32;
        for (c, v) in self.down.iter().zip(hist.iter()) {
            acc += c * v;
        }
        acc
    }
}

/// True-peak estimator (ITU-R BS.1770-4 style, finer): the largest absolute value of the signal
/// interpolated 16x (8x at 88.2 / 96 kHz, 4x above). BS.1770 asks for at least 4x, but 4x can
/// miss a worst-case peak near a quarter of the sample rate by over half a dB.
pub struct TruePeak {
    phases: Vec<Vec<f32>>,
    hist: Vec<f32>,
    i: usize,
    pub delay: usize,
}

impl TruePeak {
    pub fn new(fs: f32) -> Self {
        let factor = if fs >= 176_000.0 {
            4
        } else if fs >= 88_000.0 {
            8
        } else {
            16
        };
        let taps = 32;
        let n = taps * factor + 1;
        let h = lowpass(n, 0.495 / factor as f64, 10.0);
        let mut phases = vec![Vec::new(); factor];
        for (p, ph) in phases.iter_mut().enumerate() {
            let mut j = 0;
            while p + j * factor < n {
                ph.push((h[p + j * factor] * factor as f64) as f32);
                j += 1;
            }
        }
        let lin = taps + 2;
        Self { phases, hist: vec![0.0; 2 * lin], i: 0, delay: taps / 2 }
    }

    pub fn reset(&mut self) {
        self.hist.iter_mut().for_each(|v| *v = 0.0);
    }

    /// Feed one sample, get the true peak (absolute) of the interpolated stretch it closes.
    /// The result belongs to the sample `delay` samples back.
    #[inline]
    pub fn push(&mut self, x: f32) -> f32 {
        let lin = self.hist.len() / 2;
        self.i = if self.i == 0 { lin - 1 } else { self.i - 1 };
        self.hist[self.i] = x;
        self.hist[self.i + lin] = x;
        let hist = &self.hist[self.i..self.i + lin];
        let mut m = 0.0f32;
        for ph in self.phases.iter() {
            let mut acc = 0.0f32;
            for (c, v) in ph.iter().zip(hist.iter()) {
                acc += c * v;
            }
            m = m.max(acc.abs());
        }
        m
    }
}
