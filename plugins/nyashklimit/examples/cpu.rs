use nyashklimit::limiter::{Limiter, Params, Style};
use std::time::Instant;
fn main() {
    let fs = 48000.0;
    let n = (fs * 10.0) as usize;
    let x: Vec<f32> = (0..n).map(|i| ((i as f32 * 0.013).sin() * 1.5 + ((i * 7919) % 101) as f32 / 300.0)).collect();
    for (os, tp) in [(1, false), (1, true), (4, true), (8, true), (16, true), (32, true)] {
        let mut l = Limiter::new(fs, 2);
        l.set_params(&Params { style: Style::Modern, oversampling: os, true_peak: tp, gain_db: 6.0, ..Params::default() });
        let t = Instant::now();
        let mut acc = 0.0f32;
        for v in &x {
            let y = l.process([*v, -*v], [0.0; 2]);
            acc += y[0];
        }
        let s = t.elapsed().as_secs_f64();
        println!("OS {os:>2} TP {tp:<5}: {:5.1}% of one core (10 s stereo @48k) {acc:.0}", s / 10.0 * 100.0);
    }
}
