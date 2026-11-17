//! The limiter engine.
//!
//! Signal path (per base-rate sample):
//!
//! ```text
//! in -> DC filter -> Gain -> (Dynamic: transient lift)
//!    -> detection: oversampled peak (or the side chain) -> dB over the ceiling
//!       -> channel linking -> release stage (attack / release envelope, slow)
//!                          -> transient stage (look-ahead: peak hold + smoothing, fast release)
//!    -> audio: delayed by the look-ahead, oversampled, gain applied per high-rate sample,
//!       (Aggressive: soft clip), downsampled
//!    -> safety stage at the base rate (short look-ahead, sample or true peak): nothing passes
//!       the ceiling, whatever the oversampling filters did
//!    -> Unity Gain / Audition / dither -> out
//! ```
//!
//! Both stages work in dB of gain reduction. The transient stage holds the largest requirement
//! over the look-ahead window and smooths it with box filters whose total length equals the
//! window, so the smoothed curve always reaches the requirement by the time the audio gets
//! there: brick-wall without clipping.

use crate::dither::{Dither, Shaping};
use crate::os::{Oversampler, TruePeak};

pub const MAX_CH: usize = 2;
pub const LOOK_MAX_MS: f32 = 5.0;
/// styles may stretch the look-ahead up to this factor (Safe)
const LOOK_STRETCH_MAX: f32 = 1.5;
/// the safety stage: look-ahead in base-rate samples
const SAFE_LA: usize = 16;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Style {
    Transparent,
    Punchy,
    Dynamic,
    Allround,
    Aggressive,
    Modern,
    Bus,
    Safe,
}

impl Style {
    pub const ALL: [Style; 8] = [Style::Transparent, Style::Punchy, Style::Dynamic, Style::Allround, Style::Aggressive, Style::Modern, Style::Bus, Style::Safe];
}

/// How each style shapes the two stages.
#[derive(Clone, Copy)]
struct Tuning {
    /// share of the (linked) gain reduction the slow release stage takes on (0..1)
    slow_share: f32,
    attack_mul: f32,
    release_mul: f32,
    /// release of the transient stage, ms
    fast_release_ms: f32,
    /// look-ahead multiplier
    look_mul: f32,
    /// smoothing boxes in the transient stage (1 = fastest, 3 = smoothest)
    boxes: usize,
    /// program-dependent release (Modern / Transparent)
    adaptive: bool,
    /// transient lift before limiting, dB (Dynamic)
    lift_db: f32,
    /// dB of the peaks left to a soft clipper instead of the look-ahead (Aggressive)
    clip_db: f32,
}

fn tuning(s: Style) -> Tuning {
    let t = Tuning { slow_share: 0.5, attack_mul: 1.0, release_mul: 1.0, fast_release_ms: 20.0, look_mul: 1.0, boxes: 1, adaptive: false, lift_db: 0.0, clip_db: 0.0 };
    match s {
        Style::Transparent => Tuning { slow_share: 0.55, fast_release_ms: 25.0, boxes: 2, adaptive: true, ..t },
        Style::Punchy => Tuning { slow_share: 0.85, release_mul: 0.45, attack_mul: 0.6, fast_release_ms: 8.0, ..t },
        Style::Dynamic => Tuning { slow_share: 0.5, fast_release_ms: 15.0, lift_db: 2.0, ..t },
        Style::Allround => Tuning { slow_share: 0.6, fast_release_ms: 30.0, boxes: 2, ..t },
        Style::Aggressive => Tuning { slow_share: 0.3, fast_release_ms: 4.0, look_mul: 0.5, clip_db: 2.0, ..t },
        Style::Modern => Tuning { slow_share: 0.6, fast_release_ms: 18.0, boxes: 2, adaptive: true, ..t },
        Style::Bus => Tuning { slow_share: 1.0, attack_mul: 2.0, release_mul: 1.4, fast_release_ms: 40.0, ..t },
        Style::Safe => Tuning { slow_share: 0.7, release_mul: 1.8, fast_release_ms: 80.0, look_mul: LOOK_STRETCH_MAX, boxes: 3, ..t },
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Params {
    pub gain_db: f32,
    /// Output Level: the ceiling, dB (dBTP with true peak limiting)
    pub out_db: f32,
    pub style: Style,
    pub lookahead_ms: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    /// channel linking, 0..1
    pub link_transient: f32,
    pub link_release: f32,
    /// 1, 2, 4, 8, 16, 32
    pub oversampling: usize,
    pub true_peak: bool,
    pub dc_filter: bool,
    pub unity_gain: bool,
    pub audition: bool,
    pub sidechain: bool,
    /// 0 = off, else 16..24
    pub dither_bits: u32,
    pub shaping: Shaping,
    pub bypass: bool,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            gain_db: 0.0,
            out_db: -1.0,
            style: Style::Modern,
            lookahead_ms: 2.0,
            attack_ms: 30.0,
            release_ms: 120.0,
            link_transient: 0.75,
            link_release: 1.0,
            oversampling: 4,
            true_peak: true,
            dc_filter: false,
            unity_gain: false,
            audition: false,
            sidechain: false,
            dither_bits: 0,
            shaping: Shaping::Optimized,
            bypass: false,
        }
    }
}

#[inline]
fn db(x: f32) -> f32 {
    20.0 * x.max(1e-12).log10()
}

#[inline]
fn undb(d: f32) -> f32 {
    10f32.powf(d / 20.0)
}

/// Sliding maximum over the last `w` values (monotonic queue in a fixed ring).
struct SlidingMax {
    idx: Vec<u64>,
    val: Vec<f32>,
    head: usize,
    len: usize,
    t: u64,
}

impl SlidingMax {
    fn new(cap: usize) -> Self {
        Self { idx: vec![0; cap], val: vec![0.0; cap], head: 0, len: 0, t: 0 }
    }
    fn reset(&mut self) {
        self.head = 0;
        self.len = 0;
        self.t = 0;
    }
    #[inline]
    fn push(&mut self, v: f32, w: usize) -> f32 {
        let cap = self.idx.len();
        // drop smaller values from the back
        while self.len > 0 {
            let b = (self.head + self.len - 1) % cap;
            if self.val[b] <= v {
                self.len -= 1;
            } else {
                break;
            }
        }
        let b = (self.head + self.len) % cap;
        self.idx[b] = self.t;
        self.val[b] = v;
        self.len += 1;
        // drop expired values from the front
        while self.idx[self.head] + w as u64 <= self.t {
            self.head = (self.head + 1) % cap;
            self.len -= 1;
        }
        self.t += 1;
        self.val[self.head]
    }
}

/// Running average over the last `w` values.
struct BoxAvg {
    ring: Vec<f32>,
    i: usize,
    sum: f64,
    w: usize,
    n: u32,
}

impl BoxAvg {
    fn new(cap: usize) -> Self {
        Self { ring: vec![0.0; cap], i: 0, sum: 0.0, w: 1, n: 0 }
    }
    fn reset(&mut self) {
        self.ring.iter_mut().for_each(|v| *v = 0.0);
        self.sum = 0.0;
        self.i = 0;
    }
    fn set_len(&mut self, w: usize) {
        let w = w.clamp(1, self.ring.len());
        if w != self.w {
            self.w = w;
            self.recompute();
        }
    }
    fn recompute(&mut self) {
        let cap = self.ring.len();
        self.sum = (0..self.w).map(|k| self.ring[(self.i + cap - 1 - k) % cap] as f64).sum();
    }
    #[inline]
    fn push(&mut self, v: f32) -> f32 {
        let cap = self.ring.len();
        let old = self.ring[(self.i + cap - self.w) % cap];
        self.ring[self.i] = v;
        self.i = (self.i + 1) % cap;
        self.sum += v as f64 - old as f64;
        self.n = self.n.wrapping_add(1);
        if self.n % 65536 == 0 {
            self.recompute(); // no drift
        }
        (self.sum / self.w as f64) as f32
    }
}

/// Fixed delay line.
struct Delay {
    buf: Vec<f32>,
    i: usize,
}

impl Delay {
    fn new(n: usize) -> Self {
        Self { buf: vec![0.0; n.max(1)], i: 0 }
    }
    fn reset(&mut self) {
        self.buf.iter_mut().for_each(|v| *v = 0.0);
    }
    /// push `x`, get the value from `len` samples ago
    #[inline]
    fn tick(&mut self, x: f32) -> f32 {
        let y = self.buf[self.i];
        self.buf[self.i] = x;
        self.i = (self.i + 1) % self.buf.len();
        y
    }
}

/// One transient stage (the look-ahead part) for one channel.
struct Transient {
    hold: SlidingMax,
    boxes: [BoxAvg; 3],
    rel: f32,
}

/// Per-block measurements handed to the GUI.
#[derive(Clone, Copy, Default, Debug)]
pub struct BlockStats {
    pub in_peak: [f32; MAX_CH],
    pub out_peak: [f32; MAX_CH],
    pub gr_max_db: f32,
}

pub struct Limiter {
    fs: f32,
    p: Params,
    nch: usize,
    factor: usize,
    // filters / delays rebuilt when the oversampling changes
    up_det: Vec<Oversampler>,
    up_sc: Vec<Oversampler>,
    up_aud: Vec<Oversampler>,
    down: Vec<Oversampler>,
    d_max: usize,
    aud_delay: Vec<Delay>,
    gr_delay: Vec<Delay>,
    slow_delay: Vec<Delay>,
    fast: Vec<Transient>,
    slow: [f32; MAX_CH],
    adapt_mean: [f32; MAX_CH],
    adapt_peak: [f32; MAX_CH],
    gr_prev: [f32; MAX_CH],
    // transient lift (Dynamic)
    env_fast: f32,
    env_slow: f32,
    // DC filter
    dc_x: [f32; MAX_CH],
    dc_y: [f32; MAX_CH],
    // safety stage
    safe_tp: Vec<TruePeak>,
    safe_hold: Vec<SlidingMax>,
    safe_box: Vec<BoxAvg>,
    safe_aud: Vec<Delay>,
    // latency-matched dry path (bypass / audition)
    dry: Vec<Delay>,
    dry_gain: Vec<Delay>,
    bypass_x: f32,
    dither: Dither,
    safe_pk: Vec<Delay>,
    latency: usize,
    pub stats: BlockStats,
    /// oversampling change waiting for the output to fade out
    pending: Option<usize>,
    /// output fade for click-free oversampling changes (0..1)
    fade: f32,
    /// samples left until the rebuilt delay lines carry signal again
    mute_left: usize,
    /// nothing processed since new() / reset(): changes can apply at once
    fresh: bool,
    /// current total gain reduction (dB, max over channels), for the meters
    pub gr_now: f32,
}

impl Limiter {
    pub fn new(fs: f32, nch: usize) -> Self {
        let mut l = Self {
            fs,
            p: Params::default(),
            nch: nch.clamp(1, MAX_CH),
            factor: 0,
            up_det: vec![],
            up_sc: vec![],
            up_aud: vec![],
            down: vec![],
            d_max: 0,
            aud_delay: vec![],
            gr_delay: vec![],
            slow_delay: vec![],
            fast: vec![],
            slow: [0.0; MAX_CH],
            adapt_mean: [0.0; MAX_CH],
            adapt_peak: [0.0; MAX_CH],
            gr_prev: [0.0; MAX_CH],
            env_fast: 0.0,
            env_slow: 0.0,
            dc_x: [0.0; MAX_CH],
            dc_y: [0.0; MAX_CH],
            safe_tp: (0..MAX_CH).map(|_| TruePeak::new(fs)).collect(),
            safe_hold: (0..MAX_CH).map(|_| SlidingMax::new(SAFE_LA + 4)).collect(),
            safe_box: (0..MAX_CH).map(|_| BoxAvg::new(SAFE_LA + 4)).collect(),
            safe_aud: vec![],
            dry: vec![],
            dry_gain: vec![],
            bypass_x: 0.0,
            dither: Dither::new(0x9E37_79B9_7F4A_7C15),
            safe_pk: vec![],
            latency: 0,
            stats: BlockStats::default(),
            pending: None,
            fade: 1.0,
            mute_left: 0,
            fresh: true,
            gr_now: 0.0,
        };
        l.rebuild(4);
        l
    }

    /// The effective oversampling: true peak limiting needs at least 4x.
    fn effective_factor(p: &Params) -> usize {
        let f = p.oversampling.clamp(1, 32).next_power_of_two();
        if p.true_peak {
            f.max(4)
        } else {
            f
        }
    }

    fn rebuild(&mut self, factor: usize) {
        self.factor = factor;
        self.up_det = (0..MAX_CH).map(|_| Oversampler::new(factor)).collect();
        self.up_sc = (0..MAX_CH).map(|_| Oversampler::new(factor)).collect();
        self.up_aud = (0..MAX_CH).map(|_| Oversampler::new(factor)).collect();
        self.down = (0..MAX_CH).map(|_| Oversampler::new(factor)).collect();
        // the look-ahead delay is fixed (max look-ahead) so the latency never changes with it
        self.d_max = ((LOOK_MAX_MS * LOOK_STRETCH_MAX / 1000.0 * self.fs).ceil() as usize).max(2) + 1;
        let d = self.d_max;
        self.aud_delay = (0..MAX_CH).map(|_| Delay::new(d)).collect();
        self.slow_delay = (0..MAX_CH).map(|_| Delay::new(d)).collect();
        self.gr_delay = (0..MAX_CH).map(|_| Delay::new(1)).collect(); // sized in set_params
        self.fast = (0..MAX_CH)
            .map(|_| Transient { hold: SlidingMax::new(d + 4), boxes: [BoxAvg::new(d + 4), BoxAvg::new(d + 4), BoxAvg::new(d + 4)], rel: 0.0 })
            .collect();
        let os_lat = self.up_aud[0].latency(); // up + down
        let tp_lat = self.safe_tp[0].delay;
        self.safe_aud = (0..MAX_CH).map(|_| Delay::new(SAFE_LA + tp_lat)).collect();
        self.safe_pk = (0..MAX_CH).map(|_| Delay::new(tp_lat)).collect();
        self.latency = d + os_lat + SAFE_LA + tp_lat;
        self.dry = (0..MAX_CH).map(|_| Delay::new(self.latency)).collect();
        self.dry_gain = (0..MAX_CH).map(|_| Delay::new(self.latency)).collect();
        self.apply_window();
        self.reset_state();
    }

    pub fn latency(&self) -> usize {
        self.latency
    }

    fn window(&self) -> usize {
        let t = tuning(self.p.style);
        let w = (self.p.lookahead_ms.clamp(0.0, LOOK_MAX_MS) * t.look_mul / 1000.0 * self.fs).round() as usize;
        w.clamp(1, self.d_max - 1)
    }

    fn apply_window(&mut self) {
        let w = self.window();
        let t = tuning(self.p.style);
        let nb = t.boxes.clamp(1, 3);
        for f in self.fast.iter_mut() {
            let mut left = w;
            for (k, b) in f.boxes.iter_mut().enumerate() {
                let len = if k < nb { (w / nb).max(1) } else { 1 };
                let len = if k == nb - 1 { left.max(1) } else { len };
                left = left.saturating_sub(len);
                b.set_len(len);
            }
        }
        // the GR stream is delayed so the transient stage lines up with the delayed audio
        let gd = self.d_max - w;
        for g in self.gr_delay.iter_mut() {
            if g.buf.len() != gd.max(1) {
                *g = Delay::new(gd.max(1));
            }
        }
    }

    pub fn set_params(&mut self, p: &Params) {
        let old = self.p;
        self.p = *p;
        let f = Self::effective_factor(p);
        if f != self.factor {
            if self.fresh {
                self.rebuild(f);
                self.pending = None;
            } else {
                // fade out first, rebuild when silent (see process)
                self.pending = Some(f);
            }
        } else if self.pending.is_some() {
            self.pending = None;
        } else if old.lookahead_ms != p.lookahead_ms || old.style != p.style {
            self.apply_window();
        }
    }

    pub fn reset(&mut self) {
        let f = self.pending.take().unwrap_or(self.factor);
        self.rebuild(f);
        self.fade = 1.0;
        self.mute_left = 0;
        self.fresh = true;
    }

    fn reset_state(&mut self) {
        for o in self.up_det.iter_mut().chain(self.up_sc.iter_mut()).chain(self.up_aud.iter_mut()).chain(self.down.iter_mut()) {
            o.reset();
        }
        for d in self.aud_delay.iter_mut().chain(self.gr_delay.iter_mut()).chain(self.slow_delay.iter_mut()).chain(self.safe_aud.iter_mut()).chain(self.safe_pk.iter_mut()).chain(self.dry.iter_mut()).chain(self.dry_gain.iter_mut()) {
            d.reset();
        }
        for f in self.fast.iter_mut() {
            f.hold.reset();
            f.boxes.iter_mut().for_each(|b| b.reset());
            f.rel = 0.0;
        }
        for t in self.safe_tp.iter_mut() {
            t.reset();
        }
        self.safe_hold.iter_mut().for_each(|h| h.reset());
        self.safe_box.iter_mut().for_each(|b| {
            b.reset();
            b.set_len(SAFE_LA);
        });
        self.slow = [0.0; MAX_CH];
        self.adapt_mean = [0.0; MAX_CH];
        self.adapt_peak = [0.0; MAX_CH];
        self.gr_prev = [0.0; MAX_CH];
        self.env_fast = 0.0;
        self.env_slow = 0.0;
        self.dc_x = [0.0; MAX_CH];
        self.dc_y = [0.0; MAX_CH];
        self.dither.reset();
    }

    pub fn ceiling_db(&self) -> f32 {
        if self.p.unity_gain {
            -self.p.gain_db.max(0.0)
        } else {
            self.p.out_db
        }
    }

    pub fn take_stats(&mut self) -> BlockStats {
        std::mem::take(&mut self.stats)
    }

    /// Process one frame. `sc` is the external side chain (used when `sidechain` is on).
    #[inline]
    pub fn process(&mut self, x: [f32; MAX_CH], sc: [f32; MAX_CH]) -> [f32; MAX_CH] {
        self.fresh = false;
        // an oversampling change: fade out (5 ms), rebuild, wait for the new delay lines to
        // fill, fade in (5 ms)
        let fade_step = 1.0 / (0.005 * self.fs);
        if let Some(f) = self.pending {
            self.fade -= fade_step;
            if self.fade <= 0.0 {
                self.fade = 0.0;
                self.pending = None;
                self.rebuild(f);
                self.mute_left = self.latency;
            }
        } else if self.mute_left > 0 {
            self.mute_left -= 1;
        } else if self.fade < 1.0 {
            self.fade = (self.fade + fade_step).min(1.0);
        }
        // bad host input (NaN / inf) would poison every filter state: treat it as silence;
        // values below -300 dB are flushed so nothing inside goes denormal
        let clean = |v: f32| if v.is_finite() && v.abs() > 1e-15 { v } else { 0.0 };
        let x = [clean(x[0]), clean(x[1])];
        let sc = [clean(sc[0]), clean(sc[1])];
        let nch = self.nch;
        let p = self.p;
        let t = tuning(p.style);
        let l = self.factor;
        let fs = self.fs;
        let g = undb(p.gain_db);
        let ceil_db = self.ceiling_db();
        let ceil = undb(ceil_db);
        // the main stages aim a hair under the ceiling, the safety stage catches the rest
        let target_db = ceil_db - 0.05;

        // ---- input conditioning
        let mut xin = [0.0f32; MAX_CH];
        let mut scin = [0.0f32; MAX_CH];
        let dc_a = 1.0 - 2.0 * std::f32::consts::PI * 5.0 / fs;
        for c in 0..nch {
            let mut v = x[c];
            if p.dc_filter {
                let y = v - self.dc_x[c] + dc_a * self.dc_y[c];
                self.dc_x[c] = v;
                self.dc_y[c] = y;
                v = y;
            }
            xin[c] = v * g;
            scin[c] = sc[c] * g;
            self.stats.in_peak[c] = self.stats.in_peak[c].max(xin[c].abs());
        }

        // ---- Dynamic: lift the transients a little before they are limited
        if t.lift_db > 0.0 {
            let m = (0..nch).map(|c| xin[c].abs()).fold(0.0f32, f32::max);
            let a = |ms: f32| 1.0 - (-1.0 / (ms / 1000.0 * fs)).exp();
            let ef = if m > self.env_fast { a(0.5) } else { a(30.0) };
            self.env_fast += (m - self.env_fast) * ef;
            let es = if m > self.env_slow { a(40.0) } else { a(250.0) };
            self.env_slow += (m - self.env_slow) * es;
            let diff = db(self.env_fast) - db(self.env_slow.max(1e-6));
            let lift = undb((diff / 6.0).clamp(0.0, 1.0) * t.lift_db);
            for v in xin.iter_mut().take(nch) {
                *v *= lift;
            }
        }

        // ---- detection: the oversampled peak of this base sample, dB over the target
        let mut req = [0.0f32; MAX_CH];
        for c in 0..nch {
            let mut buf = [0.0f32; 32];
            if p.sidechain {
                self.up_sc[c].up(scin[c], &mut buf[..l]);
            } else {
                self.up_det[c].up(xin[c], &mut buf[..l]);
            }
            let pk = buf[..l].iter().fold(0.0f32, |a, v| a.max(v.abs()));
            req[c] = (db(pk) - target_db - t.clip_db).max(0.0);
        }
        let rmax = req[..nch].iter().cloned().fold(0.0f32, f32::max);

        // ---- release stage (slow): attack / release envelope on the linked requirement
        let mut slow_now = [0.0f32; MAX_CH];
        for c in 0..nch {
            let r = p.link_release * rmax + (1.0 - p.link_release) * req[c];
            let mut rel_ms = p.release_ms * t.release_mul;
            if t.adaptive {
                // program-dependent: short release after isolated peaks, longer on dense material
                let a_m = 1.0 - (-1.0 / (0.3 * fs)).exp();
                self.adapt_mean[c] += (r - self.adapt_mean[c]) * a_m;
                self.adapt_peak[c] = (self.adapt_peak[c] * (1.0 - a_m * 0.5)).max(r);
                let density = (self.adapt_mean[c] / self.adapt_peak[c].max(1e-3)).clamp(0.0, 1.0);
                rel_ms *= 0.35 + 1.3 * density;
            }
            let att_ms = (p.attack_ms * t.attack_mul).max(0.01);
            let target = r * t.slow_share;
            let a = if target > self.slow[c] { 1.0 - (-1.0 / (att_ms / 1000.0 * fs)).exp() } else { 1.0 - (-1.0 / (rel_ms.max(1.0) / 1000.0 * fs)).exp() };
            self.slow[c] += (target - self.slow[c]) * a;
            slow_now[c] = self.slow[c];
        }

        // ---- transient stage (look-ahead): whatever the slow stage does not cover yet
        let w = self.window();
        let fr = (-1.0 / (t.fast_release_ms / 1000.0 * fs)).exp();
        let mut gr = [0.0f32; MAX_CH];
        for c in 0..nch {
            let rt = p.link_transient * rmax + (1.0 - p.link_transient) * req[c];
            let need = (rt - slow_now[c]).max(0.0);
            let f = &mut self.fast[c];
            let mut v = f.hold.push(need, w + 1);
            for b in f.boxes.iter_mut() {
                v = b.push(v); // unused boxes have length 1: they pass the value through
            }
            // release of the transient stage (never less than the smoothed requirement)
            f.rel = (f.rel * fr).max(v);
            // line up: the slow stage value from the same moment, the transient value delayed
            let s_al = self.slow_delay[c].tick(slow_now[c]);
            let f_al = self.gr_delay[c].tick(f.rel);
            gr[c] = s_al + f_al;
        }
        if nch == 1 {
            gr[1] = gr[0];
        }

        // ---- audio: delayed, oversampled, gain per high-rate sample, back down
        let mut y = [0.0f32; MAX_CH];
        let mut grm = 0.0f32;
        for c in 0..nch {
            let xd = self.aud_delay[c].tick(xin[c]);
            let mut buf = [0.0f32; 32];
            self.up_aud[c].up(xd, &mut buf[..l]);
            let g0 = self.gr_prev[c];
            let g1 = gr[c];
            for (k, v) in buf[..l].iter_mut().enumerate() {
                let gdb = g0 + (g1 - g0) * ((k + 1) as f32 / l as f32);
                *v *= undb(-gdb);
                if t.clip_db > 0.0 {
                    // Aggressive: the last dB or two of the peaks go into a soft clipper
                    let th = ceil * undb(-t.clip_db);
                    let a = v.abs();
                    if a > th {
                        let k2 = ceil - th;
                        *v = v.signum() * (th + k2 * ((a - th) / k2).tanh());
                    }
                }
            }
            y[c] = self.down[c].down(&buf[..l]);
            self.gr_prev[c] = g1;
            grm = grm.max(g1);
        }

        // ---- safety stage: sample peak (or true peak) never above the ceiling
        let mut out = [0.0f32; MAX_CH];
        let mut req_s = [0.0f32; MAX_CH];
        for c in 0..nch {
            // both detectors report the sample `tp_lat` back, so the audio delay is the same
            let tp = self.safe_tp[c].push(y[c]);
            let sp = self.safe_pk[c].tick(y[c]).abs();
            let pk = if p.true_peak { tp.max(sp) } else { sp };
            req_s[c] = (db(pk) - ceil_db).max(0.0);
        }
        let rs_max = req_s[..nch].iter().cloned().fold(0.0f32, f32::max);
        for c in 0..nch {
            let r = p.link_transient * rs_max + (1.0 - p.link_transient) * req_s[c];
            let h = self.safe_hold[c].push(r, SAFE_LA + 1);
            let s = self.safe_box[c].push(h);
            let yd = self.safe_aud[c].tick(y[c]);
            // last resort: a numeric hair over the ceiling is cut
            let v = (yd * undb(-s)).clamp(-ceil, ceil);
            out[c] = v;
            grm = grm.max(s + gr[c]);
        }
        self.gr_now = grm;
        self.stats.gr_max_db = self.stats.gr_max_db.max(grm);

        // ---- dry path (bypass, audition) and the output options
        let mut res = [0.0f32; MAX_CH];
        let bx_t = if p.bypass { 1.0 } else { 0.0 };
        self.bypass_x += (bx_t - self.bypass_x) * (1.0 - (-1.0 / (0.01 * fs)).exp());
        if (self.bypass_x - bx_t).abs() < 1e-4 {
            self.bypass_x = bx_t;
        }
        for c in 0..nch {
            let dry = self.dry[c].tick(x[c]);
            let dryg = self.dry_gain[c].tick(xin[c]);
            // Audition: what the limiter takes away, played back at the level of the source (divided
            // by the gain) so it never blasts out louder than the track itself
            let mut v = if p.audition { (dryg - out[c]) / g } else { out[c] };
            if p.dither_bits > 0 && !p.audition {
                v = self.dither.process(c, v, p.dither_bits, p.shaping);
            }
            v += (dry - v) * self.bypass_x;
            v *= self.fade;
            res[c] = v;
            self.stats.out_peak[c] = self.stats.out_peak[c].max(v.abs());
        }
        res
    }
}
