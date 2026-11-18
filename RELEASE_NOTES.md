## NyashkLimit 1.0.2

- Верх більше не приглушується: з оверсемплінгом (2x–32x) або True Peak на 44.1 кГц було ≈ −6 дБ на 20 кГц і −0.7 дБ на 18 кГц. Тепер рівно до 20 кГц на всіх режимах.
- True Peak точніший: детектор читає і найвищі частоти, міжсемпловий пік враховується для обох сусідніх семплів, на 176–192 кГц 8x замість 4x.
- Фільтри оверсемплінгу векторизовані: з довшими (кращими) фільтрами все одно швидше ніж було (4x: 22x реального часу замість 21x, 32x: 6x замість 5x на одне ядро).
- Захист від NaN / inf у вході і сайдчейні. Затримка трохи більша (на 1–2 мс), вона повідомляється хосту.

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
