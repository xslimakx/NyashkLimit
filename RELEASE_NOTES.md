## NyashkLimit 1.0.1

- **A hot input keeps its shape:** when the input (with Gain) goes over 0 dB the display adds headroom above 0 (+3 / +6 / +9 ...), so the waveform is no longer a flat line at the top.
- **Gain reduction is shown where it happens:** the red band between the output and the input level is exactly what the limiter took away; the peak labels sit on its crests.
- **No more jitter:** the history scrolls smoothly and is no longer re-sampled every frame, so the waves and the GR labels stay still while they move.
- Smoother, anti-aliased curves.

## NyashkLimit 1.0.0

First release.

**Limiting**
- 8 styles.
- Look-ahead, attack, release and channel linking.
- True peak limiting with up to 32x oversampling.
- Output ceiling, Unity Gain and Audition.
- DC filter, external side chain and dither with noise shaping.

**Display and meters**
- Scrolling display with gain-reduction peak labels.
- K-System and loudness meters: M / S / I, LRA, targets.
- Compact view.

**Workflow**
- Presets, A/B, undo/redo.
