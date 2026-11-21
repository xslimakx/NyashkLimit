//! Developer helpers for screenshots. With `NYASHK_DEMO=1` the standalone build replaces the
//! input with a loud, music-like loop (kick, bass, snare, hats) so the display has something to
//! draw; `NYASHK_DEMO_UI="scale=6,display=1,compact=1,gain=8"` presets the view.
use crate::UiSettings;
use nih_plug::prelude::Buffer;
use std::sync::Mutex;

struct State {
    n: u64,
    rng: u32,
    on: bool,
}
static ST: Mutex<Option<State>> = Mutex::new(None);

pub fn fill(buffer: &mut Buffer, fs: f32) {
    // lock-free check first: the audio threads of several instances must never wait on each other
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if !*ON.get_or_init(|| std::env::var("NYASHK_DEMO").is_ok()) {
        return;
    }
    let Ok(mut g) = ST.try_lock() else { return };
    let st = g.get_or_insert_with(|| State { n: 0, rng: 777, on: std::env::var("NYASHK_DEMO").is_ok() });
    if !st.on {
        return;
    }
    let tau = std::f32::consts::TAU;
    for mut frame in buffer.iter_samples() {
        let t = st.n as f32 / fs;
        st.n += 1;
        let bar = (t / 2.0) as u32;
        // a section that gets louder every two bars, so the GR trace moves
        let lift = [0.75, 0.9, 1.1, 1.3][(bar % 4) as usize];
        let beat = (t * 2.0).fract();
        let kick = (-beat * 18.0).exp() * (tau * (48.0 + 110.0 * (-beat * 35.0).exp()) * t).sin();
        let sn_ph = (t * 2.0 + 0.5).fract();
        let bass = 0.35 * (tau * 55.0 * t).sin() * (0.6 + 0.4 * (-beat * 3.0).exp());
        let pad = 0.08 * ((tau * 220.0 * t).sin() + (tau * 277.2 * t).sin() + (tau * 329.6 * t).sin());
        for (c, s) in frame.iter_mut().enumerate() {
            st.rng = st.rng.wrapping_mul(1664525).wrapping_add(1013904223);
            let white = (st.rng >> 8) as f32 / 8388608.0 - 1.0;
            let snare = if sn_ph < 0.25 { 0.55 * white * (-sn_ph * 14.0).exp() } else { 0.0 };
            let hp = (t * 8.0 + 0.5).fract();
            let hat = 0.12 * white * (-hp * 60.0).exp();
            let x = 0.85 * kick + bass + snare + hat + pad * if c == 0 { 1.0 } else { 0.9 };
            *s = x * 0.55 * lift;
        }
    }
}

pub fn ui(mut u: UiSettings) -> UiSettings {
    let Ok(spec) = std::env::var("NYASHK_DEMO_UI") else { return u };
    for kv in spec.split(',') {
        let mut it = kv.split('=');
        let (Some(k), Some(v)) = (it.next(), it.next()) else { continue };
        let n: f32 = v.trim().parse().unwrap_or(0.0);
        match k.trim() {
            "scale" => u.scale = n as u8,
            "display" => u.display = n as u8,
            "compact" => u.compact = n != 0.0,
            "advanced" => u.advanced = n != 0.0,
            "ltime" => u.ltime = n as u8,
            "lscale" => u.lscale = n as u8,
            "relative" => u.relative = n != 0.0,
            "target" => u.target = n,
            _ => {}
        }
    }
    u
}

pub fn gain() -> Option<f32> {
    let spec = std::env::var("NYASHK_DEMO_UI").ok()?;
    spec.split(',').find_map(|kv| kv.strip_prefix("gain=").and_then(|v| v.parse().ok()))
}

/// a boolean parameter default for screenshots: `NYASHK_DEMO_UI="unity=1,bypass=1"`
pub fn flag(name: &str, default: bool) -> bool {
    let Ok(spec) = std::env::var("NYASHK_DEMO_UI") else { return default };
    spec.split(',').find_map(|kv| kv.strip_prefix(name).and_then(|v| v.strip_prefix('=')).map(|v| v == "1")).unwrap_or(default)
}
