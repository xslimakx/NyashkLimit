//! Factory presets, grouped by the kind of material. Each one sets the limiter's sound
//! (style, timing, linking, gain); the output section (ceiling, true peak, oversampling, dither)
//! only changes when the preset is about a delivery standard.

pub struct Preset {
    pub category: &'static str,
    pub name: &'static str,
    /// (parameter id, plain value). Enums take their index, switches 0 / 1.
    pub values: &'static [(&'static str, f32)],
    /// loudness target for the meter, if the preset is about a standard
    pub target: Option<f32>,
}

// style indices: 0 Transparent, 1 Punchy, 2 Dynamic, 3 Allround, 4 Aggressive, 5 Modern, 6 Bus, 7 Safe

pub const PRESETS: &[Preset] = &[
    Preset { category: "Mastering", name: "Default", values: &[("style", 5.0), ("gain", 0.0), ("look", 2.0), ("attack", 30.0), ("release", 120.0), ("linkt", 75.0), ("linkr", 100.0)], target: None },
    Preset { category: "Mastering", name: "Clean and loud", values: &[("style", 5.0), ("gain", 6.0), ("look", 1.5), ("attack", 20.0), ("release", 100.0), ("linkt", 60.0), ("linkr", 100.0)], target: None },
    Preset { category: "Mastering", name: "Transparent polish", values: &[("style", 0.0), ("gain", 3.0), ("look", 3.0), ("attack", 40.0), ("release", 180.0), ("linkt", 80.0), ("linkr", 100.0)], target: None },
    Preset { category: "Mastering", name: "Allround master", values: &[("style", 3.0), ("gain", 4.5), ("look", 2.5), ("attack", 25.0), ("release", 150.0), ("linkt", 75.0), ("linkr", 100.0)], target: None },
    Preset { category: "Mastering", name: "Punch keeper", values: &[("style", 2.0), ("gain", 5.0), ("look", 1.0), ("attack", 30.0), ("release", 120.0), ("linkt", 50.0), ("linkr", 100.0)], target: None },
    Preset { category: "Mastering", name: "Gentle safety", values: &[("style", 7.0), ("gain", 1.5), ("look", 5.0), ("attack", 50.0), ("release", 300.0), ("linkt", 100.0), ("linkr", 100.0)], target: None },
    Preset { category: "Loud genres", name: "EDM brickwall", values: &[("style", 4.0), ("gain", 9.0), ("look", 0.5), ("attack", 10.0), ("release", 60.0), ("linkt", 50.0), ("linkr", 100.0)], target: None },
    Preset { category: "Loud genres", name: "Rock wall", values: &[("style", 4.0), ("gain", 7.0), ("look", 1.0), ("attack", 15.0), ("release", 80.0), ("linkt", 70.0), ("linkr", 100.0)], target: None },
    Preset { category: "Loud genres", name: "Trap low end", values: &[("style", 5.0), ("gain", 7.0), ("look", 3.0), ("attack", 40.0), ("release", 250.0), ("linkt", 90.0), ("linkr", 100.0)], target: None },
    Preset { category: "Acoustic", name: "Classical safety", values: &[("style", 7.0), ("gain", 0.0), ("look", 5.0), ("attack", 80.0), ("release", 500.0), ("linkt", 100.0), ("linkr", 100.0)], target: None },
    Preset { category: "Acoustic", name: "Jazz / folk", values: &[("style", 0.0), ("gain", 2.0), ("look", 4.0), ("attack", 50.0), ("release", 300.0), ("linkt", 90.0), ("linkr", 100.0)], target: None },
    Preset { category: "Tracks", name: "Vocal leveler", values: &[("style", 1.0), ("gain", 6.0), ("look", 2.0), ("attack", 10.0), ("release", 80.0), ("linkt", 100.0), ("linkr", 100.0)], target: None },
    Preset { category: "Tracks", name: "Bass control", values: &[("style", 5.0), ("gain", 4.0), ("look", 5.0), ("attack", 50.0), ("release", 200.0), ("linkt", 100.0), ("linkr", 100.0)], target: None },
    Preset { category: "Tracks", name: "Drum bus glue", values: &[("style", 6.0), ("gain", 5.0), ("look", 1.0), ("attack", 20.0), ("release", 150.0), ("linkt", 100.0), ("linkr", 100.0)], target: None },
    Preset { category: "Tracks", name: "Drum smash", values: &[("style", 6.0), ("gain", 12.0), ("look", 0.3), ("attack", 5.0), ("release", 60.0), ("linkt", 100.0), ("linkr", 100.0)], target: None },
    Preset { category: "Tracks", name: "Guitar punch", values: &[("style", 1.0), ("gain", 5.0), ("look", 1.0), ("attack", 15.0), ("release", 90.0), ("linkt", 80.0), ("linkr", 100.0)], target: None },
    Preset { category: "Delivery", name: "Streaming -14 LUFS / -1 dBTP", values: &[("out", -1.0), ("tp", 1.0), ("os", 2.0)], target: Some(-14.0) },
    Preset { category: "Delivery", name: "EBU R128 -23 LUFS / -1 dBTP", values: &[("out", -1.0), ("tp", 1.0), ("os", 2.0)], target: Some(-23.0) },
    Preset { category: "Delivery", name: "ATSC A/85 -24 LUFS / -2 dBTP", values: &[("out", -2.0), ("tp", 1.0), ("os", 2.0)], target: Some(-24.0) },
    Preset { category: "Delivery", name: "CD -0.1 dBTP, 16 bit dither", values: &[("out", -0.1), ("tp", 1.0), ("os", 2.0), ("dither", 5.0), ("shaping", 2.0)], target: Some(-9.0) },
];
