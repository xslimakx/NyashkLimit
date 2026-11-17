//! Loudness metering after ITU-R BS.1770-4 / EBU R128 / EBU Tech 3342:
//! K-weighting, Momentary (400 ms), Short-term (3 s), gated Integrated, Loudness Range,
//! maximum Momentary / Short-term.

use std::f64::consts::PI;

#[derive(Clone, Copy, Default)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

impl Biquad {
    /// The two K-weighting stages for any sample rate (the analog prototypes of BS.1770,
    /// bilinear-transformed; at 48 kHz they match the coefficients printed in the standard).
    fn k_shelf(fs: f64) -> Self {
        let (f0, g, q) = (1681.974450955533, 3.999843853973347, 0.7071752369554196);
        let k = (PI * f0 / fs).tan();
        let vh = 10f64.powf(g / 20.0);
        let vb = vh.powf(0.4996667741545416);
        let a0 = 1.0 + k / q + k * k;
        Self {
            b0: (vh + vb * k / q + k * k) / a0,
            b1: 2.0 * (k * k - vh) / a0,
            b2: (vh - vb * k / q + k * k) / a0,
            a1: 2.0 * (k * k - 1.0) / a0,
            a2: (1.0 - k / q + k * k) / a0,
        }
    }

    fn k_highpass(fs: f64) -> Self {
        let (f0, q) = (38.13547087602444, 0.5003270373238773);
        let k = (PI * f0 / fs).tan();
        let a0 = 1.0 + k / q + k * k;
        Self { b0: 1.0, b1: -2.0, b2: 1.0, a1: 2.0 * (k * k - 1.0) / a0, a2: (1.0 - k / q + k * k) / a0 }
    }
}

#[derive(Clone, Copy, Default)]
struct BqState {
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl BqState {
    #[inline]
    fn tick(&mut self, c: &Biquad, x: f64) -> f64 {
        let y = c.b0 * x + c.b1 * self.x1 + c.b2 * self.x2 - c.a1 * self.y1 - c.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

const BINS: usize = 1600;
const BIN_LO: f64 = -70.0;
const BIN_W: f64 = 0.05;

fn bin_of(l: f64) -> Option<usize> {
    if l <= BIN_LO {
        return None;
    }
    Some((((l - BIN_LO) / BIN_W) as usize).min(BINS - 1))
}

fn bin_lufs(i: usize) -> f64 {
    BIN_LO + (i as f64 + 0.5) * BIN_W
}

pub fn lufs(power: f64) -> f64 {
    if power <= 1e-20 {
        -200.0
    } else {
        -0.691 + 10.0 * power.log10()
    }
}

pub struct Loudness {
    #[allow(dead_code)]
    fs: f64,
    shelf: Biquad,
    hp: Biquad,
    st: [[BqState; 2]; 2],
    /// sum of K-weighted squares (both channels) in the current 100 ms hop
    hop_acc: f64,
    hop_n: usize,
    hop_len: usize,
    /// the last 30 hop powers (3 s) for Momentary (last 4) and Short-term (all 30)
    hops: [f64; 30],
    hop_i: usize,
    hops_seen: usize,
    /// Integrated: 400 ms block powers kept as a histogram (0.05 LU bins from -70 to +10 LUFS:
    /// count and power sum per bin), so hours of audio cost no memory and no allocation
    blk_n: Vec<u32>,
    blk_p: Vec<f64>,
    /// LRA: Short-term values (every 100 ms), histogram counts
    st_n: Vec<u32>,
    pub momentary: f64,
    pub short_term: f64,
    pub integrated: f64,
    pub lra: f64,
    pub max_m: f64,
    pub max_s: f64,
    pub paused: bool,
}

impl Loudness {
    pub fn new(fs: f32) -> Self {
        let fs = fs as f64;
        Self {
            fs,
            shelf: Biquad::k_shelf(fs),
            hp: Biquad::k_highpass(fs),
            st: [[BqState::default(); 2]; 2],
            hop_acc: 0.0,
            hop_n: 0,
            hop_len: (fs * 0.1).round() as usize,
            hops: [0.0; 30],
            hop_i: 0,
            hops_seen: 0,
            blk_n: vec![0; BINS],
            blk_p: vec![0.0; BINS],
            st_n: vec![0; BINS],
            momentary: -200.0,
            short_term: -200.0,
            integrated: -200.0,
            lra: 0.0,
            max_m: -200.0,
            max_s: -200.0,
            paused: false,
        }
    }

    /// Clear every measurement (no allocation: safe on the audio thread).
    pub fn reset(&mut self) {
        self.st = [[BqState::default(); 2]; 2];
        self.hop_acc = 0.0;
        self.hop_n = 0;
        self.hops = [0.0; 30];
        self.hop_i = 0;
        self.hops_seen = 0;
        self.blk_n.iter_mut().for_each(|v| *v = 0);
        self.blk_p.iter_mut().for_each(|v| *v = 0.0);
        self.st_n.iter_mut().for_each(|v| *v = 0);
        self.momentary = -200.0;
        self.short_term = -200.0;
        self.integrated = -200.0;
        self.lra = 0.0;
        self.max_m = -200.0;
        self.max_s = -200.0;
    }

    /// One stereo (or mono: r = l, `channels` = 1) sample. Returns true when a 100 ms hop closed.
    #[inline]
    pub fn push(&mut self, l: f32, r: f32, channels: usize) -> bool {
        let mut acc = 0.0;
        for (c, x) in [l, r].iter().enumerate().take(channels.clamp(1, 2)) {
            let s = &mut self.st[c];
            let a = s[0].tick(&self.shelf, *x as f64);
            let y = s[1].tick(&self.hp, a);
            acc += y * y;
        }
        self.hop_acc += acc;
        self.hop_n += 1;
        if self.hop_n < self.hop_len {
            return false;
        }
        let p = self.hop_acc / self.hop_n as f64;
        self.hop_acc = 0.0;
        self.hop_n = 0;
        self.hops[self.hop_i] = p;
        self.hop_i = (self.hop_i + 1) % 30;
        self.hops_seen += 1;
        let mean_last = |n: usize| {
            let n = n.min(self.hops_seen).max(1);
            let mut s = 0.0;
            for k in 0..n {
                s += self.hops[(self.hop_i + 30 - 1 - k) % 30];
            }
            s / n as f64
        };
        let pm = mean_last(4);
        let ps = mean_last(30);
        self.momentary = lufs(pm);
        self.short_term = lufs(ps);
        if self.paused {
            return true;
        }
        if self.hops_seen >= 4 {
            self.max_m = self.max_m.max(self.momentary);
            if let Some(b) = bin_of(self.momentary) {
                self.blk_n[b] += 1;
                self.blk_p[b] += pm;
            }
            self.update_integrated();
        }
        if self.hops_seen >= 30 {
            self.max_s = self.max_s.max(self.short_term);
            if let Some(b) = bin_of(self.short_term) {
                self.st_n[b] += 1;
            }
            if self.hops_seen % 5 == 0 {
                self.update_lra();
            }
        }
        true
    }

    fn update_integrated(&mut self) {
        // absolute gate -70 LUFS (the histogram starts there), then a relative gate 10 LU below
        // the mean of the absolutely-gated blocks
        let (mut s, mut n) = (0.0, 0u64);
        for i in 0..BINS {
            s += self.blk_p[i];
            n += self.blk_n[i] as u64;
        }
        if n == 0 {
            self.integrated = -200.0;
            return;
        }
        let rel = lufs(s / n as f64) - 10.0;
        let (mut s2, mut n2) = (0.0, 0u64);
        for i in 0..BINS {
            if bin_lufs(i) > rel {
                s2 += self.blk_p[i];
                n2 += self.blk_n[i] as u64;
            }
        }
        self.integrated = if n2 > 0 { lufs(s2 / n2 as f64) } else { -200.0 };
    }

    fn update_lra(&mut self) {
        // EBU Tech 3342: short-term values, absolute gate -70, relative gate 20 LU below their
        // power mean; LRA = 95th - 10th percentile
        let (mut s, mut n) = (0.0, 0u64);
        for i in 0..BINS {
            let c = self.st_n[i] as u64;
            if c > 0 {
                s += c as f64 * 10f64.powf((bin_lufs(i) + 0.691) / 10.0);
                n += c;
            }
        }
        if n < 2 {
            self.lra = 0.0;
            return;
        }
        let rel = lufs(s / n as f64) - 20.0;
        let first = bin_of(rel).unwrap_or(0);
        let total: u64 = (first..BINS).map(|i| self.st_n[i] as u64).sum();
        if total < 2 {
            self.lra = 0.0;
            return;
        }
        let pct = |p: f64| {
            let target = p * total as f64;
            let mut acc = 0u64;
            for i in first..BINS {
                acc += self.st_n[i] as u64;
                if acc as f64 >= target {
                    return bin_lufs(i);
                }
            }
            bin_lufs(BINS - 1)
        };
        self.lra = pct(0.95) - pct(0.10);
    }
}
