//! NyashkLimit - a true peak mastering limiter (VST3).
//!
//! Eight limiting styles over a two-stage engine (a slow release stage and a look-ahead transient
//! stage), channel linking, up to 32x linear-phase oversampling, true peak limiting, DC filter,
//! Unity Gain, Audition, external side chain, dither with noise shaping, and full level and
//! loudness metering (peak / RMS, true peak, K-System, EBU R128 Momentary, Short-term, Integrated,
//! Loudness Range).

use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

mod demo;
pub mod dither;
mod editor;
pub mod limiter;
pub mod loudness;
pub mod os;
pub mod presets;

use limiter::{Limiter, Style, MAX_CH};
use loudness::Loudness;
use os::TruePeak;

// ------------------------------------------------------------------------------------------
// Parameters
// ------------------------------------------------------------------------------------------

#[derive(Enum, PartialEq, Eq, Clone, Copy, Debug)]
pub enum StyleParam {
    #[name = "Transparent"]
    Transparent,
    #[name = "Punchy"]
    Punchy,
    #[name = "Dynamic"]
    Dynamic,
    #[name = "Allround"]
    Allround,
    #[name = "Aggressive"]
    Aggressive,
    #[name = "Modern"]
    Modern,
    #[name = "Bus"]
    Bus,
    #[name = "Safe"]
    Safe,
}

impl StyleParam {
    pub fn style(self) -> Style {
        Style::ALL[self.to_index()]
    }
    pub fn name(self) -> &'static str {
        Self::variants()[self.to_index()]
    }
    pub fn about(self) -> &'static str {
        match self {
            StyleParam::Transparent => "slow, clean gain riding; keeps the mix as it is",
            StyleParam::Punchy => "lets drums breathe and bounce a bit when pushed",
            StyleParam::Dynamic => "sharpens hits first, then limits: more snap",
            StyleParam::Allround => "middle ground for any material",
            StyleParam::Aggressive => "short and hot, soft-clips the tips: loudest",
            StyleParam::Modern => "clean default, loud, quick on transients",
            StyleParam::Bus => "slow and thick: drum groups, single tracks",
            StyleParam::Safe => "long look-ahead, never crunches: acoustic",
        }
    }
}

#[derive(Enum, PartialEq, Eq, Clone, Copy, Debug)]
pub enum OsParam {
    #[name = "Off"]
    Off,
    #[name = "2x"]
    X2,
    #[name = "4x"]
    X4,
    #[name = "8x"]
    X8,
    #[name = "16x"]
    X16,
    #[name = "32x"]
    X32,
}

impl OsParam {
    pub fn factor(self) -> usize {
        1 << self.to_index()
    }
}

#[derive(Enum, PartialEq, Eq, Clone, Copy, Debug)]
pub enum DitherParam {
    #[name = "Off"]
    Off,
    #[name = "24 bit"]
    B24,
    #[name = "22 bit"]
    B22,
    #[name = "20 bit"]
    B20,
    #[name = "18 bit"]
    B18,
    #[name = "16 bit"]
    B16,
}

impl DitherParam {
    pub fn bits(self) -> u32 {
        [0, 24, 22, 20, 18, 16][self.to_index()]
    }
}

#[derive(Enum, PartialEq, Eq, Clone, Copy, Debug)]
pub enum ShapingParam {
    #[name = "None"]
    None,
    #[name = "Basic"]
    Basic,
    #[name = "Optimized"]
    Optimized,
    #[name = "Weighted"]
    Weighted,
}

impl ShapingParam {
    pub fn shaping(self) -> dither::Shaping {
        [dither::Shaping::None, dither::Shaping::Basic, dither::Shaping::Optimized, dither::Shaping::Weighted][self.to_index()]
    }
}

/// GUI settings saved with the session (not automatable).
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
pub struct UiSettings {
    /// 0 = 16 dB, 1 = 32 dB, 2 = 48 dB, 3 = K-12, 4 = K-14, 5 = K-20, 6 = Loudness
    pub scale: u8,
    /// 0 = Fast, 1 = Slow, 2 = Slow Down, 3 = Infinite, 4 = Off
    pub display: u8,
    pub tp_meter: bool,
    /// loudness target LUFS
    pub target: f32,
    /// 0 = +9 LU, 1 = +18 LU
    pub lscale: u8,
    /// 0 = Momentary, 1 = Short Term, 2 = Integrated
    pub ltime: u8,
    pub relative: bool,
    pub auto_reset: bool,
    /// show Max M (false) or Max S (true) next to Integrated
    pub max_s: bool,
    pub compact: bool,
    pub advanced: bool,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self { scale: 1, display: 0, tp_meter: true, target: -14.0, lscale: 0, ltime: 1, relative: false, auto_reset: true, max_s: false, compact: false, advanced: true }
    }
}

fn ms_range(min: f32, max: f32, skew: f32) -> FloatRange {
    FloatRange::Skewed { min, max, factor: FloatRange::skew_factor(skew) }
}

fn decimals(n: usize) -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(move |v| format!("{:.*}", n, v + 0.0))
}

fn parse_unit(units: &'static [&'static str]) -> Arc<dyn Fn(&str) -> Option<f32> + Send + Sync> {
    Arc::new(move |s| {
        let mut t = s.trim().to_lowercase();
        for u in units {
            if let Some(x) = t.strip_suffix(u) {
                t = x.trim().to_string();
            }
        }
        t.parse().ok()
    })
}

#[derive(Params)]
pub struct LimitParams {
    #[persist = "editor-state"]
    pub editor_state: Arc<EguiState>,
    #[persist = "ui"]
    pub ui: RwLock<UiSettings>,

    #[id = "bypass"]
    pub bypass: BoolParam,
    #[id = "gain"]
    pub gain: FloatParam,
    #[id = "out"]
    pub output: FloatParam,
    #[id = "style"]
    pub style: EnumParam<StyleParam>,
    #[id = "look"]
    pub lookahead: FloatParam,
    #[id = "attack"]
    pub attack: FloatParam,
    #[id = "release"]
    pub release: FloatParam,
    #[id = "linkt"]
    pub link_transient: FloatParam,
    #[id = "linkr"]
    pub link_release: FloatParam,
    #[id = "os"]
    pub oversampling: EnumParam<OsParam>,
    #[id = "tp"]
    pub true_peak: BoolParam,
    #[id = "dc"]
    pub dc_filter: BoolParam,
    #[id = "unity"]
    pub unity: BoolParam,
    #[id = "audition"]
    pub audition: BoolParam,
    #[id = "sc"]
    pub sidechain: BoolParam,
    #[id = "dither"]
    pub dither: EnumParam<DitherParam>,
    #[id = "shaping"]
    pub shaping: EnumParam<ShapingParam>,
}

impl Default for LimitParams {
    fn default() -> Self {
        Self {
            editor_state: EguiState::from_size(980, 600),
            ui: RwLock::new(demo::ui(UiSettings::default())),
            bypass: BoolParam::new("Bypass", demo::flag("bypass", false)).make_bypass(),
            gain: FloatParam::new("Gain", demo::gain().unwrap_or(0.0), FloatRange::Linear { min: 0.0, max: 30.0 })
                .with_unit(" dB")
                .with_step_size(0.01)
                .with_value_to_string(decimals(2))
                .with_string_to_value(parse_unit(&["db"])),
            output: FloatParam::new("Output Level", -1.0, FloatRange::Linear { min: -30.0, max: 0.0 })
                .with_unit(" dB")
                .with_step_size(0.01)
                .with_value_to_string(decimals(2))
                .with_string_to_value(parse_unit(&["dbtp", "db"])),
            style: EnumParam::new("Style", StyleParam::Modern),
            lookahead: FloatParam::new("Lookahead", 2.0, FloatRange::Linear { min: 0.0, max: limiter::LOOK_MAX_MS })
                .with_unit(" ms")
                .with_step_size(0.01)
                .with_value_to_string(decimals(2))
                .with_string_to_value(parse_unit(&["ms"])),
            attack: FloatParam::new("Attack", 30.0, ms_range(0.0, 1000.0, -2.0))
                .with_unit(" ms")
                .with_value_to_string(decimals(1))
                .with_string_to_value(parse_unit(&["ms"])),
            release: FloatParam::new("Release", 120.0, ms_range(1.0, 5000.0, -2.0))
                .with_unit(" ms")
                .with_value_to_string(decimals(0))
                .with_string_to_value(parse_unit(&["ms"])),
            link_transient: FloatParam::new("Channel Link Transients", 75.0, FloatRange::Linear { min: 0.0, max: 100.0 })
                .with_unit(" %")
                .with_step_size(1.0)
                .with_value_to_string(decimals(0))
                .with_string_to_value(parse_unit(&["%"])),
            link_release: FloatParam::new("Channel Link Release", 100.0, FloatRange::Linear { min: 0.0, max: 100.0 })
                .with_unit(" %")
                .with_step_size(1.0)
                .with_value_to_string(decimals(0))
                .with_string_to_value(parse_unit(&["%"])),
            oversampling: EnumParam::new("Oversampling", OsParam::X4),
            true_peak: BoolParam::new("True Peak Limiting", true),
            dc_filter: BoolParam::new("Filter DC Offset", demo::flag("dc", false)),
            unity: BoolParam::new("Unity Gain", demo::flag("unity", false)),
            audition: BoolParam::new("Audition Limiting", demo::flag("audition", false)),
            sidechain: BoolParam::new("External Side Chain", false),
            dither: EnumParam::new("Dither", DitherParam::Off),
            shaping: EnumParam::new("Noise Shaping", ShapingParam::Optimized),
        }
    }
}

impl LimitParams {
    pub fn dsp(&self) -> limiter::Params {
        limiter::Params {
            gain_db: self.gain.value(),
            out_db: self.output.value(),
            style: self.style.value().style(),
            lookahead_ms: self.lookahead.value(),
            attack_ms: self.attack.value(),
            release_ms: self.release.value(),
            link_transient: self.link_transient.value() / 100.0,
            link_release: self.link_release.value() / 100.0,
            oversampling: self.oversampling.value().factor(),
            true_peak: self.true_peak.value(),
            dc_filter: self.dc_filter.value(),
            unity_gain: self.unity.value(),
            audition: self.audition.value(),
            sidechain: self.sidechain.value(),
            dither_bits: self.dither.value().bits(),
            shaping: self.shaping.value().shaping(),
            bypass: self.bypass.value(),
        }
    }
}

// ------------------------------------------------------------------------------------------
// Shared with the GUI
// ------------------------------------------------------------------------------------------

/// One column of the scrolling display (10 ms of audio).
#[derive(Clone, Copy, Default, Debug)]
pub struct Column {
    /// input peak after Gain (dBFS)
    pub inp: f32,
    /// output peak (dBFS)
    pub out: f32,
    /// largest gain reduction (dB, positive)
    pub gr: f32,
    /// Momentary loudness (LUFS)
    pub loud: f32,
}

pub const COL_SECS: f32 = 0.01;
pub const HIST_CAP: usize = 1 << 19; // ~87 minutes

pub struct History {
    pub cols: Vec<Column>,
    /// total columns written since the last reset
    pub written: u64,
}

impl History {
    pub fn get(&self, i: u64) -> Column {
        self.cols[(i % HIST_CAP as u64) as usize]
    }
}

#[derive(Default)]
pub struct Shared {
    pub out_peak: [AtomicF32; MAX_CH],
    pub out_rms: [AtomicF32; MAX_CH],
    pub out_rms_sum: AtomicF32,
    /// max output peak since reset (sample peak, dBFS)
    pub max_peak: AtomicF32,
    /// max true peak since reset (dBTP)
    pub max_tp: AtomicF32,
    pub max_gr: AtomicF32,
    pub gr_now: AtomicF32,
    pub momentary: AtomicF32,
    pub short_term: AtomicF32,
    pub integrated: AtomicF32,
    pub lra: AtomicF32,
    pub max_m: AtomicF32,
    pub max_s: AtomicF32,
    pub channels: AtomicU32,
    pub sample_rate: AtomicF32,
    pub latency: AtomicU32,
    pub reset_peaks: AtomicBool,
    pub reset_loudness: AtomicBool,
    pub pause_loudness: AtomicBool,
    pub reset_history: AtomicBool,
}

pub struct Meters {
    pub s: Shared,
    pub hist: Mutex<History>,
}

impl Default for Meters {
    fn default() -> Self {
        let s = Shared::default();
        s.max_peak.store(-200.0, Ordering::Relaxed);
        s.max_tp.store(-200.0, Ordering::Relaxed);
        Self { s, hist: Mutex::new(History { cols: vec![Column { inp: -200.0, out: -200.0, gr: 0.0, loud: -200.0 }; HIST_CAP], written: 0 }) }
    }
}

// ------------------------------------------------------------------------------------------
// Plugin
// ------------------------------------------------------------------------------------------

pub struct NyashkLimit {
    params: Arc<LimitParams>,
    meters: Arc<Meters>,
    lim: Option<Limiter>,
    loud: Option<Loudness>,
    loud_ch: Vec<Loudness>,
    tp: Vec<TruePeak>,
    fs: f32,
    nch: usize,
    col_n: usize,
    col_len: usize,
    pending: Vec<Column>,
    was_playing: bool,
    rms_win: [f64; MAX_CH],
}

impl Default for NyashkLimit {
    fn default() -> Self {
        Self {
            params: Arc::new(LimitParams::default()),
            meters: Arc::new(Meters::default()),
            lim: None,
            loud: None,
            loud_ch: vec![],
            tp: vec![],
            fs: 48000.0,
            nch: 2,
            col_n: 0,
            col_len: 480,
            pending: Vec::with_capacity(256),
            was_playing: false,
            rms_win: [0.0; MAX_CH],
        }
    }
}

fn dbf(x: f32) -> f32 {
    20.0 * x.max(1e-10).log10()
}

impl Plugin for NyashkLimit {
    const NAME: &'static str = "NyashkLimit";
    const VENDOR: &'static str = "Nyashk";
    const URL: &'static str = "https://github.com/xslimakx/NyashkLimit";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(2),
            main_output_channels: NonZeroU32::new(2),
            aux_input_ports: &[new_nonzero_u32(2)],
            names: PortNames { layout: Some("Stereo"), main_input: None, main_output: None, aux_inputs: &["Side Chain"], aux_outputs: &[] },
            ..AudioIOLayout::const_default()
        },
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(1),
            main_output_channels: NonZeroU32::new(1),
            aux_input_ports: &[new_nonzero_u32(1)],
            names: PortNames { layout: Some("Mono"), main_input: None, main_output: None, aux_inputs: &["Side Chain"], aux_outputs: &[] },
            ..AudioIOLayout::const_default()
        },
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(2),
            main_output_channels: NonZeroU32::new(2),
            aux_input_ports: &[new_nonzero_u32(1)],
            names: PortNames { layout: Some("Stereo, mono side chain"), main_input: None, main_output: None, aux_inputs: &["Side Chain"], aux_outputs: &[] },
            ..AudioIOLayout::const_default()
        },
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(1),
            main_output_channels: NonZeroU32::new(1),
            aux_input_ports: &[new_nonzero_u32(2)],
            names: PortNames { layout: Some("Mono, stereo side chain"), main_input: None, main_output: None, aux_inputs: &["Side Chain"], aux_outputs: &[] },
            ..AudioIOLayout::const_default()
        },
    ];

    const SAMPLE_ACCURATE_AUTOMATION: bool = false;
    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::create(self.params.clone(), self.meters.clone())
    }

    fn initialize(&mut self, layout: &AudioIOLayout, config: &BufferConfig, context: &mut impl InitContext<Self>) -> bool {
        self.fs = config.sample_rate;
        self.nch = layout.main_output_channels.map(|n| n.get() as usize).unwrap_or(2).clamp(1, MAX_CH);
        let mut lim = Limiter::new(self.fs, self.nch);
        lim.set_params(&self.params.dsp());
        context.set_latency_samples(lim.latency() as u32);
        self.meters.s.latency.store(lim.latency() as u32, Ordering::Relaxed);
        self.meters.s.channels.store(self.nch as u32, Ordering::Relaxed);
        self.meters.s.sample_rate.store(self.fs, Ordering::Relaxed);
        self.lim = Some(lim);
        self.loud = Some(Loudness::new(self.fs));
        self.loud_ch = (0..MAX_CH).map(|_| Loudness::new(self.fs)).collect();
        self.tp = (0..MAX_CH).map(|_| TruePeak::new(self.fs)).collect();
        self.col_len = ((COL_SECS * self.fs).round() as usize).max(1);
        true
    }

    fn reset(&mut self) {
        if let Some(l) = self.lim.as_mut() {
            l.reset();
        }
        self.tp.iter_mut().for_each(|t| t.reset());
    }

    fn process(&mut self, buffer: &mut Buffer, aux: &mut AuxiliaryBuffers, context: &mut impl ProcessContext<Self>) -> ProcessStatus {
        demo::fill(buffer, self.fs as f32);
        let Some(lim) = self.lim.as_mut() else { return ProcessStatus::Normal };
        let p = self.params.dsp();
        let old_lat = lim.latency();
        lim.set_params(&p);
        if lim.latency() != old_lat {
            context.set_latency_samples(lim.latency() as u32);
            self.meters.s.latency.store(lim.latency() as u32, Ordering::Relaxed);
        }
        let s = &self.meters.s;
        let ui = *self.params.ui.read();

        // resets requested by the GUI, and Auto-Reset when the host starts playing
        let playing = context.transport().playing;
        let auto = ui.auto_reset && playing && !self.was_playing;
        self.was_playing = playing;
        let loud = self.loud.as_mut().unwrap();
        if s.reset_loudness.swap(false, Ordering::Relaxed) || auto {
            loud.reset();
            s.max_peak.store(-200.0, Ordering::Relaxed);
            s.max_tp.store(-200.0, Ordering::Relaxed);
            s.max_gr.store(0.0, Ordering::Relaxed);
            s.reset_history.store(true, Ordering::Relaxed);
        }
        if s.reset_peaks.swap(false, Ordering::Relaxed) {
            s.max_peak.store(-200.0, Ordering::Relaxed);
            s.max_tp.store(-200.0, Ordering::Relaxed);
            s.max_gr.store(0.0, Ordering::Relaxed);
        }
        loud.paused = s.pause_loudness.load(Ordering::Relaxed);

        let nch = self.nch;
        let sc_buf = aux.inputs.first_mut();
        let sc_slices: Option<&[&mut [f32]]> = sc_buf.map(|b| &*b.as_slice());
        let n = buffer.samples();
        let mut max_peak = s.max_peak.load(Ordering::Relaxed);
        let mut max_tp = s.max_tp.load(Ordering::Relaxed);
        let mut opk = [0.0f32; MAX_CH];
        let mut gr_blk = 0.0f32;
        let a_rms = 1.0 - (-1.0 / (0.4 * self.fs as f64)).exp();
        let chans = buffer.as_slice();
        for i in 0..n {
            let mut x = [0.0f32; MAX_CH];
            let mut sc = [0.0f32; MAX_CH];
            for c in 0..nch {
                x[c] = chans[c][i];
                if let Some(scs) = sc_slices {
                    if let Some(ch) = scs.get(c.min(scs.len().saturating_sub(1))) {
                        sc[c] = ch[i];
                    }
                }
            }
            let y = lim.process(x, sc);
            gr_blk = gr_blk.max(lim.gr_now);
            for c in 0..nch {
                chans[c][i] = y[c];
                let a = y[c].abs();
                opk[c] = opk[c].max(a);
                let t = self.tp[c].push(y[c]);
                max_tp = max_tp.max(dbf(t));
                max_peak = max_peak.max(dbf(a));
                self.rms_win[c] += (y[c] as f64 * y[c] as f64 - self.rms_win[c]) * a_rms;
                self.loud_ch[c].push(y[c], 0.0, 1);
            }
            let r = if nch == 2 { y[1] } else { y[0] };
            loud.push(y[0], r, nch);
            // display column
            self.col_n += 1;
            if self.col_n >= self.col_len {
                self.col_n = 0;
                let st = lim.take_stats();
                let inp = (0..nch).map(|c| st.in_peak[c]).fold(0.0f32, f32::max);
                let out = (0..nch).map(|c| st.out_peak[c]).fold(0.0f32, f32::max);
                let col = Column { inp: dbf(inp), out: dbf(out), gr: st.gr_max_db, loud: loud.momentary as f32 };
                if self.pending.len() < self.pending.capacity() {
                    self.pending.push(col);
                }
            }
        }
        // hand the columns over (skip a block if the GUI holds the lock; nothing is lost)
        if !self.pending.is_empty() {
            if let Some(mut h) = self.meters.hist.try_lock() {
                if s.reset_history.swap(false, Ordering::Relaxed) {
                    h.written = 0;
                }
                for col in self.pending.drain(..) {
                    let w = h.written;
                    h.cols[(w % HIST_CAP as u64) as usize] = col;
                    h.written += 1;
                }
            }
        }
        for c in 0..nch {
            let prev = s.out_peak[c].load(Ordering::Relaxed);
            s.out_peak[c].store(prev.max(opk[c]), Ordering::Relaxed);
            s.out_rms[c].store(self.loud_ch[c].momentary as f32, Ordering::Relaxed);
        }
        s.out_rms_sum.store(loud.momentary as f32, Ordering::Relaxed);
        s.max_peak.store(max_peak, Ordering::Relaxed);
        s.max_tp.store(max_tp, Ordering::Relaxed);
        let prev_gr = s.gr_now.load(Ordering::Relaxed);
        s.gr_now.store(prev_gr.max(gr_blk), Ordering::Relaxed);
        let mg = s.max_gr.load(Ordering::Relaxed);
        s.max_gr.store(mg.max(gr_blk), Ordering::Relaxed);
        s.momentary.store(loud.momentary as f32, Ordering::Relaxed);
        s.short_term.store(loud.short_term as f32, Ordering::Relaxed);
        s.integrated.store(loud.integrated as f32, Ordering::Relaxed);
        s.lra.store(loud.lra as f32, Ordering::Relaxed);
        s.max_m.store(loud.max_m as f32, Ordering::Relaxed);
        s.max_s.store(loud.max_s as f32, Ordering::Relaxed);
        ProcessStatus::Normal
    }
}

impl Vst3Plugin for NyashkLimit {
    const VST3_CLASS_ID: [u8; 16] = *b"NyashkLimitVST3!";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Fx, Vst3SubCategory::Dynamics, Vst3SubCategory::Mastering];
}

nih_export_vst3!(NyashkLimit);
