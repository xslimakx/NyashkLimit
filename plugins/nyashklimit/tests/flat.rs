//! Below the ceiling the limiter must leave the top end alone at every oversampling setting.
use nyashklimit::limiter::{Limiter, Params};
#[test]
fn flat_to_20k_at_every_oversampling() {
    for fs in [44100.0f32, 48000.0] {
        for os in [1usize, 2, 4, 8, 16, 32] {
            for tp in [false, true] {
                print!("fs {fs} os {os} tp {tp}: ");
                for f in [1000.0f32, 15000.0, 18000.0, 19000.0, 20000.0] {
                    let mut l = Limiter::new(fs, 2);
                    l.set_params(&Params { gain_db: 0.0, out_db: 0.0, oversampling: os, true_peak: tp, dc_filter: false, dither_bits: 0, ..Params::default() });
                    let n = (fs * 0.5) as usize;
                    let lat = l.latency();
                    let (mut ei, mut eo) = (0.0f64, 0.0f64);
                    for i in 0..n + lat {
                        let s = if i < n { 0.05 * (2.0 * std::f32::consts::PI * f * i as f32 / fs).sin() } else { 0.0 };
                        let y = l.process([s, s], [0.0; 2]);
                        if i > n / 2 && i < n { ei += (s * s) as f64; eo += (y[0] * y[0]) as f64; }
                    }
                    let g = 10.0 * (eo / ei).log10();
                    print!("{f}: {g:.2}  ");
                    assert!(g.abs() < 0.05, "fs {fs} os {os} tp {tp} f {f}: {g} dB");
                }
                println!();
            }
        }
    }
}
