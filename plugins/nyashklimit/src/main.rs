use nih_plug::prelude::*;
use nyashklimit::NyashkLimit;

fn main() {
    nih_export_standalone::<NyashkLimit>();
}
