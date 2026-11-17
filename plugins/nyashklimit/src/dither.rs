//! Final word-length reduction: TPDF dither with error-feedback noise shaping.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shaping {
    /// no shaping: flat TPDF noise
    None,
    /// mild 2nd-order: a few dB less noise in the low and mid range, more above ~6 kHz
    Basic,
    /// Lipshitz 5-tap E-weighted: lower overall noise, strong rise above ~10 kHz
    Optimized,
    /// Wannamaker 9-tap F-weighted (ear sensitivity at low levels), designed for 44.1 kHz
    Weighted,
}

impl Shaping {
    fn coefs(self) -> &'static [f64] {
        match self {
            Shaping::None => &[],
            Shaping::Basic => &[1.0, -0.5],
            Shaping::Optimized => &[2.033, -2.165, 1.959, -1.590, 0.6149],
            Shaping::Weighted => &[2.412, -3.370, 3.937, -4.174, 3.353, -2.205, 1.281, -0.569, 0.0847],
        }
    }
}

pub struct Dither {
    err: [[f64; 9]; 2],
    rng: u64,
}

impl Dither {
    pub fn new(seed: u64) -> Self {
        Self { err: [[0.0; 9]; 2], rng: seed | 1 }
    }

    pub fn reset(&mut self) {
        self.err = [[0.0; 9]; 2];
    }

    #[inline]
    fn uniform(&mut self) -> f64 {
        // xorshift64*
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        let v = self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D);
        (v >> 11) as f64 / (1u64 << 53) as f64 - 0.5
    }

    /// Quantise `x` to `bits` with TPDF dither (+-1 LSB) and the chosen noise shaping.
    #[inline]
    pub fn process(&mut self, ch: usize, x: f32, bits: u32, shaping: Shaping) -> f32 {
        let q = 2f64.powi(-(bits as i32 - 1));
        let c = shaping.coefs();
        let e = &self.err[ch];
        let mut fb = 0.0;
        for (k, ck) in c.iter().enumerate() {
            fb += ck * e[k];
        }
        let w = x as f64 - fb;
        let d = (self.uniform() + self.uniform()) * q;
        let y = ((w + d) / q).round() * q;
        let y = y.clamp(-1.0, 1.0 - q);
        let err = &mut self.err[ch];
        err.copy_within(0..8, 1);
        err[0] = y - w;
        y as f32
    }
}
