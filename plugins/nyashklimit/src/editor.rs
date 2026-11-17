//! NyashkLimit GUI: a big real-time display (input, output, gain reduction from the top, the
//! loudness curve, gain-reduction peak labels) with the Gain fader on its left and the level or
//! loudness meters on its right; the style, timing, linking and output section below.

use nih_plug::prelude::*;
use nih_plug_egui::{
    create_egui_editor,
    egui::{self, epaint::PathStroke, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2},
};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::presets::PRESETS;
use crate::{Column, DitherParam, LimitParams, Meters, OsParam, ShapingParam, StyleParam, UiSettings, COL_SECS};

// palette: night plum, lilac, rose, crimson
const BG: Color32 = Color32::from_rgb(0x12, 0x0c, 0x14);
const PANEL: Color32 = Color32::from_rgb(0x1a, 0x11, 0x1d);
const PANEL2: Color32 = Color32::from_rgb(0x26, 0x19, 0x2b);
const WHITE: Color32 = Color32::from_rgb(0xf0, 0xe6, 0xf5);
const DIM: Color32 = Color32::from_rgb(0xae, 0x9a, 0xb6);
const LILAC: Color32 = Color32::from_rgb(0xb4, 0x8a, 0xc9);
const ROSE: Color32 = Color32::from_rgb(0xe0, 0x5a, 0x8a);
const CRIMSON: Color32 = Color32::from_rgb(0xc7, 0x3a, 0x5e);
const AMBER: Color32 = Color32::from_rgb(0xe8, 0xb0, 0x5a);


const BASE_W: f32 = 980.0;
const BASE_H: f32 = 600.0;

/// UI font: a clean proportional sans (DejaVu Sans), slightly larger than the old mono
fn mono(size: f32) -> FontId {
    FontId::proportional(size + 0.5)
}
fn set_norm<P: Param>(setter: &ParamSetter, p: &P, v: f32) {
    setter.begin_set_parameter(p);
    setter.set_parameter_normalized(p, v.clamp(0.0, 1.0));
    setter.end_set_parameter(p);
}
fn set_b(setter: &ParamSetter, p: &BoolParam, v: bool) {
    setter.begin_set_parameter(p);
    setter.set_parameter(p, v);
    setter.end_set_parameter(p);
}
fn set_e<E: Enum + PartialEq>(setter: &ParamSetter, p: &EnumParam<E>, v: E) {
    setter.begin_set_parameter(p);
    setter.set_parameter(p, v);
    setter.end_set_parameter(p);
}
/// ♯ and ♭ are not in egui's built-in fonts: DejaVu Sans as a fall-back after the primary font.
fn install_fonts(ctx: &egui::Context) {
    use egui::{FontData, FontFamily};
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert("dejavu".into(), Arc::new(FontData::from_static(include_bytes!("fonts/DejaVuSans.ttf"))));
    for fam in [FontFamily::Monospace, FontFamily::Proportional] {
        if let Some(list) = fonts.families.get_mut(&fam) {
            list.insert(0, "dejavu".into());
        }
    }
    ctx.set_fonts(fonts);
}

fn enum_combo<E: Enum + PartialEq + 'static>(ui: &mut Ui, id: &str, rect: Rect, cur: E, shown: &str) -> Option<E> {
    let names = E::variants();
    let idx = cur.to_index();
    let mut pick = None;
    let mut btn = rect;
    ui.allocate_new_ui(UiBuilder::new().max_rect(rect), |ui| {
        btn = egui::ComboBox::from_id_salt(id)
            .width(rect.width() - 22.0)
            .selected_text(egui::RichText::new(shown).font(mono(12.0)))
            .show_ui(ui, |ui| {
                for (i, n) in names.iter().enumerate() {
                    if ui.selectable_label(i == idx, egui::RichText::new(*n).font(mono(12.0))).clicked() {
                        pick = Some(E::from_index(i));
                    }
                }
            })
            .response
            .rect;
    });
    specular_rim(ui.painter(), btn, (btn.height() * 0.5).min(10.0), 0.7);
    pick
}

/// Knob: drag (shift = fine), wheel, double-click = default, click the value to type.
/// Returns the normalised change made this frame.
#[allow(clippy::too_many_arguments)]
fn knob<P: Param>(ui: &mut Ui, setter: &ParamSetter, p: &P, label: &str, id: &str, c: Pos2, r: f32, tooltip: &str, bipolar: bool) -> f32 {
    let hit = Rect::from_center_size(c, Vec2::splat(2.0 * r + 8.0));
    let resp = ui.allocate_rect(hit, Sense::click_and_drag());
    let resp = if tooltip.is_empty() { resp } else { resp.on_hover_text(tooltip) };
    let norm = p.unmodulated_normalized_value();
    let mut delta = 0.0f32;
    let raw_id = egui::Id::new(("knob_raw", id));
    if resp.drag_started() {
        setter.begin_set_parameter(p);
        ui.data_mut(|d| d.insert_temp(raw_id, norm));
    }
    if resp.dragged() {
        let fine = ui.input(|i| i.modifiers.shift);
        let d = -resp.drag_delta().y / if fine { 1200.0 } else { 220.0 };
        let raw = ui.data(|dd| dd.get_temp::<f32>(raw_id)).unwrap_or(norm) + d;
        ui.data_mut(|dd| dd.insert_temp(raw_id, raw));
        let new = raw.clamp(0.0, 1.0);
        delta = new - norm;
        setter.set_parameter_normalized(p, new);
    }
    if resp.drag_stopped() {
        setter.end_set_parameter(p);
    }
    // double-click or ctrl-click: back to the default
    if resp.double_clicked() || (resp.clicked() && ui.input(|i| i.modifiers.command)) {
        delta = p.default_normalized_value() - norm;
        set_norm(setter, p, p.default_normalized_value());
    }
    if resp.hovered() {
        let sc = ui.input(|i| i.smooth_scroll_delta.y);
        if sc != 0.0 {
            let new = (norm + sc / 1500.0).clamp(0.0, 1.0);
            delta = new - norm;
            set_norm(setter, p, new);
        }
    }
    let norm = p.unmodulated_normalized_value();
    let painter = ui.painter();
    let a0 = std::f32::consts::PI * 0.75;
    let sweep = std::f32::consts::PI * 1.5;
    let ring = r - 2.5;
    let pt = |rad: f32, t: f32| -> Pos2 {
        let a = a0 + t * sweep;
        Pos2::new(c.x + rad * a.cos(), c.y + rad * a.sin())
    };
    let arc = |from: f32, to: f32| -> Vec<Pos2> {
        let n = (((to - from).abs() / sweep) * 120.0).ceil().max(2.0) as usize;
        (0..=n).map(|i| pt(ring, from + (to - from) * i as f32 / n as f32)).collect()
    };
    let w: f32 = if r > 34.0 { 4.0 } else { 3.2 };
    painter.add(egui::Shape::line(arc(0.0, 1.0), PathStroke::new(w, Color32::from_white_alpha(18))));
    let from = if bipolar { 0.5 } else { 0.0 };
    if (norm - from).abs() > 0.002 {
        painter.add(egui::Shape::line(arc(from, norm), PathStroke::new(w, ROSE)));
    }
    let body = r * 0.72;
    // soft shadow, then a gently lit body (lighter towards the top)
    for k in 0..4 {
        painter.circle_filled(c + Vec2::new(0.0, 3.0), body + 1.0 + k as f32 * 1.6, Color32::from_black_alpha(26 - k * 6));
    }
    painter.circle_filled(c, body, Color32::from_rgb(0x3a, 0x29, 0x42));
    painter.circle_filled(c + Vec2::new(0.0, 1.2), body * 0.93, Color32::from_rgb(0x2b, 0x1d, 0x32));
    painter.circle_filled(c - Vec2::new(0.0, body * 0.35), body * 0.45, Color32::from_white_alpha(5));
    painter.circle_filled(pt(body * 0.62, norm), 2.6, WHITE);

    // value box (click to type) + label
    let val = p.normalized_value_to_string(norm, true);
    let vr = Rect::from_center_size(Pos2::new(c.x, c.y + r + 14.0), Vec2::new(64.0, 18.0));
    let edit_id = egui::Id::new(("knob_edit", id));
    let editing: Option<String> = ui.data(|d| d.get_temp::<String>(edit_id));
    ui.painter().text(Pos2::new(c.x, vr.bottom() + 10.0), Align2::CENTER_CENTER, label, mono(10.0), DIM);
    if let Some(mut buf) = editing {
        let te = egui::TextEdit::singleline(&mut buf).font(mono(11.0)).horizontal_align(egui::Align::Center).desired_width(vr.width()).margin(egui::Margin::symmetric(2, 1));
        let r_te = ui.put(vr, te);
        if !r_te.has_focus() && !r_te.lost_focus() {
            r_te.request_focus();
        }
        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
        let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if esc {
            ui.data_mut(|d| d.remove::<String>(edit_id));
        } else if enter || r_te.lost_focus() {
            let txt = buf.trim().replace(',', ".");
            if let Some(nv) = p.string_to_normalized_value(&txt) {
                delta = nv - norm;
                set_norm(setter, p, nv);
            }
            ui.data_mut(|d| d.remove::<String>(edit_id));
        } else {
            ui.data_mut(|d| d.insert_temp(edit_id, buf));
        }
    } else {
        let vresp = ui.allocate_rect(vr, Sense::click()).on_hover_text("Click to type a value");
        let painter = ui.painter();
        glass_pill(painter, vr, vresp.hovered());
        let gw = ui.fonts(|f| f.layout_no_wrap(val.clone(), mono(11.0), WHITE).size().x);
        let size = if gw > vr.width() - 6.0 { (11.0 * (vr.width() - 6.0) / gw).max(7.5) } else { 11.0 };
        painter.text(vr.center(), Align2::CENTER_CENTER, &val, mono(size), WHITE);
        if vresp.clicked() {
            let first = val.trim().split(' ').next().unwrap_or("").to_string();
            let same = p.string_to_normalized_value(&first).map(|m| p.normalized_value_to_string(m, true)).as_deref() == Some(val.as_str());
            let plain = if same { first } else { val.trim().to_string() };
            ui.data_mut(|d| d.insert_temp(edit_id, plain));
        }
    }
    delta
}


// ------------------------------------------------------------------------------------------
// meter scales
// ------------------------------------------------------------------------------------------

pub const SCALE_NAMES: [&str; 7] = ["16 dB", "32 dB", "48 dB", "K-12", "K-14", "K-20", "Loudness"];
const DISPLAY_NAMES: [&str; 5] = ["Fast", "Slow", "Slow Down", "Infinite", "Off"];
const TARGETS: [(f32, &str); 4] = [(-9.0, "-9 LUFS CD"), (-14.0, "-14 LUFS web"), (-23.0, "-23 LUFS EBU"), (-24.0, "-24 LUFS ATSC")];

/// dB value -> 0 (bottom) .. 1 (top) for the current scale. The plain scales spend half the
/// height on their top part (where limiting happens) and the rest on what is below.
fn scale_pos(scale: u8, db: f32) -> f32 {
    let (range, top_part) = match scale {
        0 => (16.0, 5.0),
        1 => (32.0, 10.0),
        2 => (48.0, 16.0),
        3 => (36.0, 0.0),
        4 => (38.0, 0.0),
        5 => (44.0, 0.0),
        _ => (48.0, 0.0),
    };
    if top_part > 0.0 {
        if db >= -top_part {
            0.5 + 0.5 * (1.0 + db / top_part).clamp(0.0, 1.0)
        } else {
            (0.5 * (1.0 + (db + top_part) / (range - top_part))).clamp(0.0, 0.5)
        }
    } else {
        ((db + range) / range).clamp(0.0, 1.0)
    }
}

fn scale_ticks(scale: u8) -> Vec<f32> {
    match scale {
        0 => vec![0.0, -1.0, -2.0, -3.0, -4.0, -5.0, -8.0, -12.0, -16.0],
        1 => vec![0.0, -2.0, -4.0, -6.0, -8.0, -10.0, -15.0, -20.0, -25.0, -32.0],
        2 => vec![0.0, -3.0, -6.0, -9.0, -12.0, -16.0, -24.0, -32.0, -40.0, -48.0],
        3 => vec![0.0, -4.0, -8.0, -12.0, -16.0, -20.0, -24.0, -30.0, -36.0],
        4 => vec![0.0, -4.0, -8.0, -14.0, -18.0, -22.0, -26.0, -32.0, -38.0],
        5 => vec![0.0, -4.0, -10.0, -14.0, -20.0, -24.0, -30.0, -38.0, -44.0],
        _ => vec![0.0, -6.0, -12.0, -18.0, -24.0, -30.0, -36.0, -42.0, -48.0],
    }
}

/// K-System label for a dBFS tick (0 VU = -K dBFS)
fn k_label(scale: u8, db: f32) -> Option<String> {
    let k = match scale {
        3 => 12.0,
        4 => 14.0,
        5 => 20.0,
        _ => return None,
    };
    let v = db + k;
    Some(if v > 0.0 { format!("+{v:.0}") } else { format!("{v:.0}") })
}

fn dbs(v: f32) -> String {
    if v <= -150.0 {
        "-inf".into()
    } else {
        format!("{v:.1}")
    }
}

// ------------------------------------------------------------------------------------------
// state
// ------------------------------------------------------------------------------------------

struct Gui {
    out_lv: [f32; 2],
    out_hold: [f32; 2],
    gr_lv: f32,
    hold_t: [f64; 2],
    /// undo / redo: snapshots of all parameters (normalised)
    undo: Vec<Vec<f32>>,
    redo: Vec<Vec<f32>>,
    last_snap: Vec<f32>,
    /// A/B
    ab: [Option<Vec<f32>>; 2],
    ab_cur: usize,
    preset: usize,
    gain_raw: f32,
    custom_target: String,
    /// smooth scrolling: when the history last grew
    hist_written: u64,
    hist_t: Instant,
    /// headroom shown above 0 dB (eases with the input level)
    head_sm: f32,
}

fn snapshot(params: &LimitParams) -> Vec<f32> {
    params.param_map().iter().map(|(_, ptr, _)| unsafe { ptr.unmodulated_normalized_value() }).collect()
}

fn restore(params: &LimitParams, setter: &ParamSetter, snap: &[f32]) {
    for ((_, ptr, _), v) in params.param_map().iter().zip(snap.iter()) {
        unsafe {
            if (ptr.unmodulated_normalized_value() - v).abs() > 1e-7 {
                setter.raw_context.raw_begin_set_parameter(*ptr);
                setter.raw_context.raw_set_parameter_normalized(*ptr, *v);
                setter.raw_context.raw_end_set_parameter(*ptr);
            }
        }
    }
}

fn apply_preset(params: &LimitParams, setter: &ParamSetter, i: usize) {
    let pr = &PRESETS[i];
    let map = params.param_map();
    // a preset is a whole setting: everything it does not mention goes back to the default
    // (so the dither of a delivery preset does not stick to the next one); bypass stays
    for (id, ptr, _) in map.iter() {
        if id == "bypass" || pr.values.iter().any(|(pid, _)| pid == id) {
            continue;
        }
        unsafe {
            let n = ptr.default_normalized_value();
            if (ptr.unmodulated_normalized_value() - n).abs() > 1e-6 {
                setter.raw_context.raw_begin_set_parameter(*ptr);
                setter.raw_context.raw_set_parameter_normalized(*ptr, n);
                setter.raw_context.raw_end_set_parameter(*ptr);
            }
        }
    }
    for (id, plain) in pr.values {
        if let Some((_, ptr, _)) = map.iter().find(|(pid, _, _)| pid == id) {
            unsafe {
                let n = ptr.preview_normalized(*plain);
                setter.raw_context.raw_begin_set_parameter(*ptr);
                setter.raw_context.raw_set_parameter_normalized(*ptr, n);
                setter.raw_context.raw_end_set_parameter(*ptr);
            }
        }
    }
    if let Some(t) = pr.target {
        params.ui.write().target = t;
    }
}

/// 0 at the left edge of the history, 1 from 48 px in: the oldest part fades in softly
fn edge_fade(px: usize) -> f32 {
    let t = (px as f32 / 48.0).min(1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A polyline whose first 48 px fade in (per-segment colour), the rest drawn in one go.
fn faded_line(painter: &egui::Painter, x0: f32, pts: Vec<Pos2>, st: Stroke) {
    let split = pts.iter().position(|p| p.x - x0 >= 48.0).unwrap_or(pts.len());
    for w in pts[..(split + 1).min(pts.len())].windows(2) {
        let f = edge_fade((w[0].x - x0).max(0.0) as usize);
        painter.line_segment([w[0], w[1]], Stroke::new(st.width, st.color.gamma_multiply(f)));
    }
    if split < pts.len() && pts.len() - split > 1 {
        painter.add(egui::Shape::line(pts[split..].to_vec(), PathStroke::new(st.width, st.color)));
    }
}


/// Soft round glow (radial gradient to transparent), for the backdrop behind the glass.
fn glow(painter: &egui::Painter, c: Pos2, r: f32, col: Color32) {
    let mut mesh = egui::Mesh::default();
    let uv = egui::epaint::WHITE_UV;
    mesh.vertices.push(egui::epaint::Vertex { pos: c, uv, color: col });
    let n = 48;
    for k in 0..=n {
        let a = k as f32 / n as f32 * std::f32::consts::TAU;
        mesh.vertices.push(egui::epaint::Vertex { pos: c + Vec2::new(a.cos(), a.sin()) * r, uv, color: Color32::TRANSPARENT });
    }
    for k in 1..=n as u32 {
        mesh.indices.extend_from_slice(&[0, k, k + 1]);
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// Vertical gradient in a rectangle (top colour to bottom colour).
fn vgrad(painter: &egui::Painter, r: Rect, top: Color32, bottom: Color32) {
    let mut mesh = egui::Mesh::default();
    let uv = egui::epaint::WHITE_UV;
    for (p, c) in [(r.left_top(), top), (r.right_top(), top), (r.right_bottom(), bottom), (r.left_bottom(), bottom)] {
        mesh.vertices.push(egui::epaint::Vertex { pos: p, uv, color: c });
    }
    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
    painter.add(egui::Shape::mesh(mesh));
}

/// A vertical fade inside a rounded rectangle: `top` at the top edge, transparent `fade` px
/// lower. Built from thin horizontal strips cut to the rounded outline, so no square corners.
fn rounded_sheen(painter: &egui::Painter, r: Rect, rad: f32, top: Color32, fade: f32) {
    let mut mesh = egui::Mesh::default();
    let uv = egui::epaint::WHITE_UV;
    let inset = |y: f32| -> f32 {
        // horizontal inset of the rounded outline at height y
        let dy = if y < r.top() + rad { r.top() + rad - y } else { 0.0 };
        rad - (rad * rad - dy * dy).max(0.0).sqrt()
    };
    let steps = 40;
    for k in 0..=steps {
        let t = k as f32 / steps as f32;
        // denser near the top, where the corner curves
        let y = r.top() + fade * t * t;
        let a = (1.0 - (y - r.top()) / fade).clamp(0.0, 1.0);
        let c = top.gamma_multiply(a * a);
        let i = inset(y) + 0.5;
        mesh.vertices.push(egui::epaint::Vertex { pos: Pos2::new(r.left() + i, y), uv, color: c });
        mesh.vertices.push(egui::epaint::Vertex { pos: Pos2::new(r.right() - i, y), uv, color: c });
        if k > 0 {
            let b = (k as u32) * 2;
            mesh.indices.extend_from_slice(&[b - 2, b - 1, b + 1, b - 2, b + 1, b]);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// A soft card (iOS-like): no hard frame, a large radius, a wide soft shadow, a gently lighter
/// top. `dark` is the recessed display well.
fn glass(painter: &egui::Painter, r: Rect, dark: bool) {
    let rad = 20.0;
    // wide, faint shadow in layers
    for k in 0..6 {
        let e = 2.0 + k as f32 * 3.0;
        painter.rect_filled(r.translate(Vec2::new(0.0, 7.0)).expand(e), CornerRadius::same((rad + e) as u8), Color32::from_black_alpha(15 - k as u8 * 2));
    }
    let cr = CornerRadius::same(rad as u8);
    // the glass body: a tinted, see-through pane (the coloured backdrop shows through)
    if dark {
        painter.rect_filled(r, cr, Color32::from_rgba_unmultiplied(8, 5, 12, 205));
    } else {
        painter.rect_filled(r, cr, Color32::from_rgba_unmultiplied(40, 24, 50, 118));
    }
    // light passing through: brighter at the top, following the rounded corners
    rounded_sheen(painter, r, rad, Color32::from_white_alpha(if dark { 7 } else { 16 }), (r.height() * 0.55).min(110.0));
    // refraction at the edge: a soft bright band just inside the outline
    for k in 0..4 {
        let inset = 1.0 + k as f32 * 1.5;
        let a = [12u8, 7, 4, 2][k];
        painter.rect_stroke(r.shrink(inset), CornerRadius::same((rad - inset) as u8), Stroke::new(1.5_f32, Color32::from_white_alpha(if dark { a / 2 } else { a })), StrokeKind::Inside);
    }
    // specular rim: strong where the edge faces the light (top-left), a weaker bounce opposite
    specular_rim(painter, r, rad, if dark { 0.55 } else { 1.0 });
}

/// The outline of a rounded rectangle as points with outward normals (clockwise from top-left).
fn rounded_outline(r: Rect, rad: f32) -> Vec<(Pos2, Vec2)> {
    let mut pts = vec![];
    let corners = [
        (Pos2::new(r.left() + rad, r.top() + rad), std::f32::consts::PI),
        (Pos2::new(r.right() - rad, r.top() + rad), std::f32::consts::PI * 1.5),
        (Pos2::new(r.right() - rad, r.bottom() - rad), 0.0),
        (Pos2::new(r.left() + rad, r.bottom() - rad), std::f32::consts::PI * 0.5),
    ];
    for (c, a0) in corners {
        let n = 12;
        for k in 0..=n {
            let a = a0 + k as f32 / n as f32 * std::f32::consts::FRAC_PI_2;
            let d = Vec2::new(a.cos(), a.sin());
            pts.push((c + d * rad, d));
        }
    }
    pts
}

/// Glass edge highlight: each outline segment is lit by how much it faces the light.
fn specular_rim(painter: &egui::Painter, r: Rect, rad: f32, strength: f32) {
    let light = Vec2::new(-0.55, -0.83);
    let pts = rounded_outline(r.shrink(0.5), rad - 0.5);
    let n = pts.len();
    for k in 0..n {
        let (a, na) = pts[k];
        let (b, nb) = pts[(k + 1) % n];
        let nrm = (na + nb).normalized();
        let d = nrm.dot(light);
        let lit = d.max(0.0).powf(1.6) * 150.0 + (-d).max(0.0).powf(2.0) * 50.0 + 16.0;
        let alpha = (lit * strength).min(255.0) as u8;
        painter.line_segment([a, b], Stroke::new(1.1_f32, Color32::from_white_alpha(alpha)));
    }
}

/// A small glass control (pill): see-through fill, top sheen, specular edge.
fn glass_pill(painter: &egui::Painter, r: Rect, hover: bool) {
    let rad = (r.height() * 0.5).min(12.0);
    let cr = CornerRadius::same(rad as u8);
    painter.rect_filled(r.translate(Vec2::new(0.0, 1.5)), cr, Color32::from_black_alpha(30));
    painter.rect_filled(r, cr, Color32::from_rgba_unmultiplied(255, 236, 250, if hover { 30 } else { 16 }));
    rounded_sheen(painter, r, rad, Color32::from_white_alpha(14), r.height() * 0.6);
    specular_rim(painter, r, rad, 0.75);
}

/// gain reduction as text: "-3.2", and a plain "0.0" when there is none
fn gr_txt(v: f32) -> String {
    if v < 0.05 {
        "0.0".to_string()
    } else {
        format!("-{v:.1}")
    }
}

/// small text button
fn tbtn(ui: &mut Ui, rect: Rect, text: &str, on: bool, tip: &str) -> bool {
    let resp = ui.allocate_rect(rect, Sense::click());
    let resp = if tip.is_empty() { resp } else { resp.on_hover_text(tip) };
    let p = ui.painter();
    let hov = resp.hovered();
    let cr = CornerRadius::same((rect.height() * 0.5) as u8);
    if on {
        // lit rose glass with a soft glow around it
        for k in 1..4 {
            p.rect_filled(rect.expand(k as f32 * 1.8), CornerRadius::same((rect.height() * 0.5 + k as f32 * 1.8) as u8), ROSE.gamma_multiply(0.07));
        }
        p.rect_filled(rect, cr, ROSE.gamma_multiply(if hov { 1.0 } else { 0.9 }));
        rounded_sheen(p, rect, rect.height() * 0.5, Color32::from_white_alpha(40), rect.height() * 0.65);
        specular_rim(p, rect, rect.height() * 0.5, 0.9);
    } else {
        glass_pill(p, rect, hov);
    }
    p.text(rect.center(), Align2::CENTER_CENTER, text, mono(10.5), if on { WHITE } else { Color32::from_rgb(0xe6, 0xd8, 0xee) });
    resp.clicked()
}

/// a little combo for a list of names
fn list_combo(ui: &mut Ui, id: &str, rect: Rect, names: &[&str], cur: usize) -> Option<usize> {
    let mut pick = None;
    let mut btn = rect;
    ui.allocate_new_ui(UiBuilder::new().max_rect(rect), |ui| {
        btn = egui::ComboBox::from_id_salt(id)
            .width(rect.width() - 22.0)
            .selected_text(egui::RichText::new(names.get(cur).copied().unwrap_or("")).font(mono(10.5)))
            .show_ui(ui, |ui| {
                for (i, n) in names.iter().enumerate() {
                    if ui.selectable_label(i == cur, egui::RichText::new(*n).font(mono(11.0))).clicked() {
                        pick = Some(i);
                    }
                }
            })
            .response
            .rect;
    });
    specular_rim(ui.painter(), btn, (btn.height() * 0.5).min(10.0), 0.7);
    pick
}

// ------------------------------------------------------------------------------------------

pub fn create(params: Arc<LimitParams>, meters: Arc<Meters>) -> Option<Box<dyn Editor>> {
    let editor_state = params.editor_state.clone();
    let size_state = editor_state.clone();
    create_egui_editor(
        editor_state,
        Gui { gr_lv: 0.0, out_lv: [0.0; 2], out_hold: [-200.0; 2], hold_t: [0.0; 2], undo: vec![], redo: vec![], last_snap: vec![], ab: [None, None], ab_cur: 0, preset: 0, gain_raw: 0.0, custom_target: String::new(), hist_written: 0, hist_t: Instant::now(), head_sm: 0.0 },
        |ctx, _| {
            let mut v = egui::Visuals::dark();
            v.panel_fill = BG;
            v.window_fill = PANEL;
            v.extreme_bg_color = PANEL;
            v.override_text_color = Some(WHITE);
            v.window_stroke = Stroke::new(1.0_f32, Color32::from_white_alpha(14));
            v.window_corner_radius = CornerRadius::same(14);
            v.menu_corner_radius = CornerRadius::same(14);
            v.window_fill = Color32::from_rgb(0x24, 0x18, 0x2b);
            v.selection.bg_fill = Color32::from_rgb(0x5a, 0x2a, 0x45);
            v.selection.stroke = Stroke::new(1.0_f32, WHITE);
            for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
                w.corner_radius = CornerRadius::same(10);
                w.bg_stroke = Stroke::NONE;
            }
            v.widgets.inactive.weak_bg_fill = Color32::from_rgba_unmultiplied(255, 236, 250, 11);
            v.widgets.inactive.bg_fill = Color32::from_rgba_unmultiplied(255, 236, 250, 11);
            v.widgets.hovered.weak_bg_fill = PANEL2;
            v.widgets.hovered.bg_fill = PANEL2;
            v.widgets.active.weak_bg_fill = PANEL2;
            v.widgets.open.weak_bg_fill = PANEL2;
            v.widgets.hovered.bg_stroke = Stroke::NONE;
            v.popup_shadow = egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(110) };
            v.window_shadow = egui::Shadow::NONE;
            ctx.set_visuals(v);
            install_fonts(ctx);
        },
        move |ctx, setter, st| {
            ctx.request_repaint_after(Duration::from_millis(33));
            let p = &*params;
            let s = &meters.s;
            let now = ctx.input(|i| i.time);
            let mut ui_set: UiSettings = *p.ui.read();
            let ui_before = ui_set;

            // ---- undo history: a new step whenever the parameters settle on something new
            let snap = snapshot(p);
            let mouse_down = ctx.input(|i| i.pointer.any_down());
            if st.last_snap.is_empty() {
                st.last_snap = snap.clone();
            } else if !mouse_down && snap != st.last_snap {
                st.undo.push(std::mem::replace(&mut st.last_snap, snap.clone()));
                if st.undo.len() > 100 {
                    st.undo.remove(0);
                }
                st.redo.clear();
            }

            // ---- meters
            let nch = s.channels.load(Ordering::Relaxed).clamp(1, 2) as usize;
            {
                let g = s.gr_now.swap(0.0, Ordering::Relaxed);
                st.gr_lv = if g > st.gr_lv { g } else { (st.gr_lv - 0.6).max(g) };
            }
            for c in 0..2 {
                let v = s.out_peak[c].swap(0.0, Ordering::Relaxed);
                let d = 20.0 * v.max(1e-10).log10();
                st.out_lv[c] = if d > st.out_lv[c] { d } else { st.out_lv[c] - 1.2 };
                if d >= st.out_hold[c] || now - st.hold_t[c] > 1.5 {
                    st.out_hold[c] = d;
                    st.hold_t[c] = now;
                }
            }

            egui::CentralPanel::default().frame(egui::Frame::NONE.fill(BG)).show(ctx, |ui| {
                let full = ui.max_rect();
                let o = full.min;
                let at = |x: f32, y: f32| Pos2::new(o.x + x, o.y + y);
                let rect = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(at(x, y), Vec2::new(w, h));
                let painter = ui.painter().clone();
                // backdrop for the glass: a deep gradient with soft coloured glows
                vgrad(&painter, full, Color32::from_rgb(0x1c, 0x10, 0x22), Color32::from_rgb(0x0c, 0x08, 0x10));
                let sx = full.width() / BASE_W;
                glow(&painter, at(60.0 * sx, 80.0 * sx), 340.0 * sx, Color32::from_rgba_unmultiplied(0xe0, 0x4f, 0x8a, 60));
                glow(&painter, at(900.0 * sx, 560.0 * sx), 440.0 * sx, Color32::from_rgba_unmultiplied(0x7c, 0x52, 0xe0, 60));
                glow(&painter, at(560.0 * sx, 0.0), 300.0 * sx, Color32::from_rgba_unmultiplied(0xff, 0x9e, 0xc8, 26));
                glow(&painter, at(330.0 * sx, 620.0 * sx), 300.0 * sx, Color32::from_rgba_unmultiplied(0xc0, 0x4a, 0xb0, 40));
                let byp = p.bypass.value();

                // ================= header
                // logo, laid out by measured widths (proportional font)
                let r1 = painter.text(at(16.0, 24.0), Align2::LEFT_CENTER, "Nyashk", mono(19.0), WHITE);
                let r2 = painter.text(Pos2::new(r1.right() + 1.0, o.y + 24.0), Align2::LEFT_CENTER, "Limit", mono(19.0), LILAC);
                let r3 = painter.text(Pos2::new(r2.right() + 5.0, o.y + 23.0), Align2::LEFT_CENTER, "✦", mono(13.0), ROSE);
                painter.text(Pos2::new(r3.right() + 8.0, o.y + 25.0), Align2::LEFT_CENTER, "TRUE PEAK LIMITER", mono(8.5), DIM);
                // presets
                let px = 300.0;
                ui.allocate_new_ui(UiBuilder::new().max_rect(rect(px, 12.0, 230.0, 24.0)), |ui| {
                    // one list, grouped by category (a combo box closes reliably on outside clicks)
                    let pr = egui::ComboBox::from_id_salt("presets")
                        .width(208.0)
                        .height(560.0)
                        .selected_text(egui::RichText::new(PRESETS[st.preset.min(PRESETS.len() - 1)].name).font(mono(11.0)))
                        .show_ui(ui, |ui| {
                            let mut last_cat = "";
                            for (i, pr) in PRESETS.iter().enumerate() {
                                if pr.category != last_cat {
                                    last_cat = pr.category;
                                    ui.add_space(3.0);
                                    ui.label(egui::RichText::new(pr.category.to_uppercase()).font(mono(9.0)).color(ROSE));
                                }
                                if ui.selectable_label(i == st.preset, egui::RichText::new(format!("  {}", pr.name)).font(mono(11.0))).clicked() {
                                    st.preset = i;
                                    apply_preset(p, setter, i);
                                }
                            }
                        });
                    specular_rim(ui.painter(), pr.response.rect, (pr.response.rect.height() * 0.5).min(10.0), 0.7);
                });
                if tbtn(ui, rect(px + 236.0, 12.0, 24.0, 22.0), "‹", false, "Previous preset") {
                    st.preset = (st.preset + PRESETS.len() - 1) % PRESETS.len();
                    apply_preset(p, setter, st.preset);
                }
                if tbtn(ui, rect(px + 262.0, 12.0, 24.0, 22.0), "›", false, "Next preset") {
                    st.preset = (st.preset + 1) % PRESETS.len();
                    apply_preset(p, setter, st.preset);
                }
                // undo / redo
                if tbtn(ui, rect(px + 300.0, 12.0, 24.0, 22.0), "↶", false, "Undo") {
                    if let Some(prev) = st.undo.pop() {
                        st.redo.push(snap.clone());
                        restore(p, setter, &prev);
                        st.last_snap = prev;
                    }
                }
                if tbtn(ui, rect(px + 326.0, 12.0, 24.0, 22.0), "↷", false, "Redo") {
                    if let Some(next) = st.redo.pop() {
                        st.undo.push(snap.clone());
                        restore(p, setter, &next);
                        st.last_snap = next;
                    }
                }
                // A/B
                for (k, name) in ["A", "B"].iter().enumerate() {
                    if tbtn(ui, rect(px + 362.0 + k as f32 * 26.0, 12.0, 24.0, 22.0), name, st.ab_cur == k, "A/B: two settings to compare") && st.ab_cur != k {
                        st.ab[st.ab_cur] = Some(snap.clone());
                        if let Some(other) = st.ab[k].clone() {
                            restore(p, setter, &other);
                        }
                        st.ab_cur = k;
                    }
                }
                if tbtn(ui, rect(px + 416.0, 12.0, 46.0, 22.0), "copy", false, "Copy the current setting to the other slot") {
                    st.ab[1 - st.ab_cur] = Some(snap.clone());
                }
                // right side
                if tbtn(ui, rect(BASE_W - 16.0 - 72.0, 12.0, 72.0, 22.0), "BYPASS", byp, "Bypass (latency-compensated, click-free)") {
                    set_b(setter, &p.bypass, !byp);
                }
                if tbtn(ui, rect(BASE_W - 16.0 - 72.0 - 8.0 - 76.0, 12.0, 76.0, 22.0), if ui_set.compact { "FULL" } else { "COMPACT" }, false, "Compact: no display, big meters") {
                    ui_set.compact = !ui_set.compact;
                }
                let lat = s.latency.load(Ordering::Relaxed);
                let fs = s.sample_rate.load(Ordering::Relaxed).max(1.0);
                let lat_txt = format!("latency {:.1} ms", lat as f32 / fs * 1000.0);

                // ================= main area
                let top = 46.0;
                let main_h = 412.0;
                // ---- Gain fader (left)
                let fr = rect(14.0, top, 56.0, main_h);
                glass(&painter, fr, false);
                painter.text(Pos2::new(fr.center().x, fr.top() + 12.0), Align2::CENTER_CENTER, "GAIN", mono(9.5), LILAC);
                let track = Rect::from_min_max(Pos2::new(fr.center().x - 4.0, fr.top() + 48.0), Pos2::new(fr.center().x + 4.0, fr.bottom() - 16.0));
                let gnorm = p.gain.unmodulated_normalized_value();
                let gy = track.bottom() - track.height() * gnorm;
                painter.rect_filled(track, CornerRadius::same(4), Color32::from_black_alpha(90));
                painter.rect_filled(Rect::from_min_max(Pos2::new(track.left(), gy), track.max), CornerRadius::same(4), ROSE);
                for k in 0..=6 {
                    let y = track.bottom() - track.height() * k as f32 / 6.0;
                    painter.hline(track.right() + 6.0..=track.right() + 9.0, y, Stroke::new(1.0_f32, Color32::from_white_alpha(22)));
                    painter.text(Pos2::new(track.left() - 6.0, y), Align2::RIGHT_CENTER, format!("{}", k * 5), mono(8.0), DIM);
                }
                let handle = Rect::from_center_size(Pos2::new(track.center().x, gy), Vec2::new(30.0, 12.0));
                let gresp = ui.allocate_rect(Rect::from_min_max(Pos2::new(fr.left(), track.top() - 8.0), Pos2::new(fr.right(), track.bottom() + 8.0)), Sense::click_and_drag()).on_hover_text("Gain: drives the limiter (drag, shift = fine, double-click = 0, alt-drag also lowers the Output Level by the same amount)");
                for k in 0..3 {
                    painter.rect_filled(handle.translate(Vec2::new(0.0, 2.0)).expand(1.0 + k as f32 * 1.5), CornerRadius::same(9), Color32::from_black_alpha(40 - k * 12));
                }
                painter.rect_filled(handle, CornerRadius::same(6), if gresp.hovered() || gresp.dragged() { WHITE } else { Color32::from_rgb(0xd8, 0xc8, 0xe0) });
                painter.hline(handle.left() + 6.0..=handle.right() - 6.0, handle.center().y, Stroke::new(1.0_f32, BG));
                if gresp.drag_started() {
                    setter.begin_set_parameter(&p.gain);
                    setter.begin_set_parameter(&p.output);
                    st.gain_raw = gnorm;
                }
                if gresp.dragged() {
                    let fine = ui.input(|i| i.modifiers.shift);
                    let alt = ui.input(|i| i.modifiers.alt);
                    let d = -gresp.drag_delta().y / if fine { 1600.0 } else { track.height() };
                    st.gain_raw += d;
                    let before = p.gain.value();
                    setter.set_parameter_normalized(&p.gain, st.gain_raw.clamp(0.0, 1.0));
                    if alt {
                        let dd = p.gain.value() - before;
                        let out = (p.output.value() - dd).clamp(-30.0, 0.0);
                        setter.set_parameter(&p.output, out);
                    }
                }
                if gresp.drag_stopped() {
                    setter.end_set_parameter(&p.gain);
                    setter.end_set_parameter(&p.output);
                }
                if gresp.double_clicked() {
                    set_norm(setter, &p.gain, p.gain.default_normalized_value());
                }
                if gresp.hovered() {
                    let sc = ui.input(|i| i.smooth_scroll_delta.y);
                    if sc != 0.0 {
                        set_norm(setter, &p.gain, gnorm + sc / 2000.0);
                    }
                }
                painter.text(Pos2::new(fr.center().x, fr.top() + 30.0), Align2::CENTER_CENTER, format!("{:.1}", p.gain.value()), mono(12.0), WHITE);

                // ---- right: meters or loudness panel
                let mx = BASE_W - 14.0 - 190.0;
                let mr = rect(mx, top, 190.0, main_h);
                glass(&painter, mr, false);

                // ---- display
                let dx = 80.0;
                let dw = mx - 10.0 - dx;
                if ui_set.compact {
                    // compact: big numbers instead of the scrolling display
                    let dr = rect(dx, top, dw, main_h);
                    glass(&painter, dr, false);
                    let lufs = |v: f32| if v > -100.0 { format!("{v:.1}") } else { "-inf".to_string() };
                    let tgt = ui_set.target;
                    let near = |v: f32| if v < -100.0 { DIM } else if v > tgt + 1.0 { ROSE } else if v > tgt - 1.0 { WHITE } else { AMBER };
                    let st_v = s.short_term.load(Ordering::Relaxed);
                    let in_v = s.integrated.load(Ordering::Relaxed);
                    let mo_v = s.momentary.load(Ordering::Relaxed);
                    let tiles: [(&str, String, &str, Color32); 6] = [
                        ("SHORT TERM", lufs(st_v), "LUFS", near(st_v)),
                        ("INTEGRATED", lufs(in_v), "LUFS", near(in_v)),
                        ("MOMENTARY", lufs(mo_v), "LUFS", near(mo_v)),
                        ("LOUDNESS RANGE", format!("{:.1}", s.lra.load(Ordering::Relaxed)), "LU", WHITE),
                        ("MAX TRUE PEAK", dbs(s.max_tp.load(Ordering::Relaxed)), "dBTP", LILAC),
                        ("GAIN REDUCTION", gr_txt(st.gr_lv), "dB", ROSE),
                    ];
                    let tw = (dw - 16.0 * 4.0) / 3.0;
                    let th = (main_h - 16.0 * 3.0) / 2.0;
                    for (k, (name, val, unit, col)) in tiles.iter().enumerate() {
                        let tx = dx + 16.0 + (k % 3) as f32 * (tw + 16.0);
                        let ty = top + 16.0 + (k / 3) as f32 * (th + 16.0);
                        let tr = rect(tx, ty, tw, th);
                        glass(&painter, tr, true);
                        painter.text(Pos2::new(tr.center().x, tr.top() + 22.0), Align2::CENTER_CENTER, *name, mono(10.0), DIM);
                        painter.text(Pos2::new(tr.center().x, tr.center().y + 4.0), Align2::CENTER_CENTER, val, mono(44.0), *col);
                        painter.text(Pos2::new(tr.center().x, tr.bottom() - 26.0), Align2::CENTER_CENTER, *unit, mono(11.0), DIM);
                    }
                    painter.text(Pos2::new(dr.right() - 12.0, top + main_h - 10.0), Align2::RIGHT_CENTER, format!("target {:.1} LUFS", tgt), mono(9.0), DIM);
                }
                let scale = ui_set.scale;
                let ceil_db = if p.unity.value() { -p.gain.value() } else { p.output.value() };
                if !ui_set.compact {
                    let dr = rect(dx, top, dw, main_h);
                    glass(&painter, dr, true);
                    let g = Rect::from_min_max(Pos2::new(dr.left() + 46.0, dr.top() + 30.0), Pos2::new(dr.right() - 16.0, dr.bottom() - 30.0));
                    // the scale, plus headroom above 0 dB when the input (with Gain) goes over it, so a
                    // hot input keeps its real shape instead of a flat line at the top
                    let k_top = scale_pos(scale, 0.0) - scale_pos(scale, -1.0);
                    let head = st.head_sm;
                    let base_h = g.height() / (1.0 + head * k_top);
                    let ypos = |db: f32| {
                        if db <= 0.0 {
                            g.bottom() - base_h * scale_pos(scale, db)
                        } else {
                            g.bottom() - base_h * (1.0 + db.min(head) * k_top)
                        }
                    };
                    let disp_on = ui_set.display < 4;
                    let mut ticks = scale_ticks(scale);
                    let mut hd = 3.0;
                    while hd <= head + 0.01 {
                        ticks.push(hd);
                        hd += if head > 12.0 { 6.0 } else { 3.0 };
                    }
                    for t in ticks.into_iter().filter(|_| disp_on) {
                        let y = ypos(t);
                        painter.hline(g.left()..=g.right(), y, Stroke::new(1.0_f32, Color32::from_rgb(0x1f, 0x15, 0x23)));
                        let lab = if t > 0.0 { format!("+{t:.0}") } else { k_label(scale, t).unwrap_or_else(|| format!("{t:.0}")) };
                        painter.text(Pos2::new(g.left() - 6.0, y), Align2::RIGHT_CENTER, lab, mono(9.0), DIM);
                    }
                    // the ceiling
                    let cy = ypos(ceil_db);
                    let mut x = g.left();
                    while disp_on && x < g.right() {
                        painter.hline(x..=(x + 5.0).min(g.right()), cy, Stroke::new(1.0_f32, ROSE.gamma_multiply(0.7)));
                        x += 9.0;
                    }
                    let ceil_txt = format!("ceiling {:.1}{}", ceil_db, if p.true_peak.value() { " dBTP" } else { " dB" });

                    // history: columns anchored to their absolute index (groups of 2^k columns
                    // where they are narrower than ~1.5 px), placed at fractional x and scrolled
                    // smoothly between audio blocks - the picture moves, it is not re-sampled
                    // every frame (that was the jitter of the waves and the GR labels)
                    let mode = ui_set.display;
                    if mode < 4 {
                        if let Some(h) = meters.hist.try_lock() {
                            let written = h.written;
                            if written != st.hist_written {
                                st.hist_written = written;
                                st.hist_t = Instant::now();
                            }
                            let frac = (st.hist_t.elapsed().as_secs_f32() / COL_SECS).min(1.0);
                            let avail = written.min(crate::HIST_CAP as u64) as i64;
                            let w = g.width();
                            let per_s = 1.0 / COL_SECS;
                            let span = match mode {
                                0 => 4.0 * per_s,
                                1 => 12.0 * per_s,
                                2 => 20.0 * per_s,
                                _ => (avail as f32).max(4.0 * per_s),
                            };
                            let to_x = |age: f32| -> f32 {
                                let t = (age / span).clamp(0.0, 1.0);
                                let t = if mode == 2 { t.powf(1.0 / 2.2) } else { t };
                                g.right() - t * w
                            };
                            // newest first
                            let mut pts: Vec<(f32, Column)> = Vec::with_capacity(1024);
                            let newest = written as i64 - 1;
                            let oldest = written as i64 - avail;
                            let mut idx = newest;
                            while idx >= oldest && idx >= 0 {
                                let age = (newest - idx) as f32 + frac;
                                if age > span + 1.0 {
                                    break;
                                }
                                let ppc = (to_x(age) - to_x(age + 1.0)).max(1e-4);
                                let kk = (1.5 / ppc).log2().ceil().clamp(0.0, 20.0) as u32;
                                let gs = ((idx >> kk) << kk).max(oldest);
                                let mut c = Column { inp: -200.0, out: -200.0, gr: 0.0, loud: -200.0 };
                                for j in gs..=idx {
                                    let col = h.get(j as u64);
                                    c.inp = c.inp.max(col.inp);
                                    c.out = c.out.max(col.out);
                                    c.gr = c.gr.max(col.gr);
                                    c.loud = c.loud.max(col.loud);
                                }
                                let mid = (newest as f32 - (gs + idx) as f32 * 0.5) + frac;
                                pts.push((to_x(mid), c));
                                idx = gs - 1;
                            }
                            drop(h);
                            pts.reverse();
                            let n = pts.len();
                            // headroom for the next frame: the loudest input on screen, in 3 dB
                            // steps (up quickly, down slowly)
                            let max_in = pts.iter().map(|(_, c)| c.inp).fold(-200.0f32, f32::max);
                            let want_head = ((max_in.max(0.0) + 0.5) / 3.0).ceil().min(8.0) * 3.0;
                            let want_head = if max_in > 0.0 { want_head } else { 0.0 };
                            let dtf = ui.input(|i| i.stable_dt).min(0.1);
                            let tau = if want_head > st.head_sm { 0.15 } else { 2.0 };
                            st.head_sm += (want_head - st.head_sm) * (1.0 - (-dtf / tau).exp());
                            if n > 1 {
                                // a light [1 2 1] smoothing of the levels (dB) for clean curves
                                let sm = |f: &dyn Fn(&Column) -> f32, floor: f32| -> Vec<f32> {
                                    (0..n)
                                        .map(|i| {
                                            let v = f(&pts[i].1);
                                            if v <= floor {
                                                return v;
                                            }
                                            let a = f(&pts[i.saturating_sub(1)].1).max(floor);
                                            let b = f(&pts[(i + 1).min(n - 1)].1).max(floor);
                                            (a + 2.0 * v + b) * 0.25
                                        })
                                        .collect()
                                };
                                let xs: Vec<f32> = pts.iter().map(|(x, _)| *x).collect();
                                let v_in = sm(&|c| c.inp, -150.0);
                                let v_out = sm(&|c| c.out, -150.0);
                                let v_loud = sm(&|c| c.loud, -60.0);
                                let floor_y = g.bottom();
                                let to_y = |v: f32| ypos(v).clamp(g.top(), floor_y);
                                let fade = |x: f32| edge_fade((x - g.left()).max(0.0) as usize);
                                let uv = egui::epaint::WHITE_UV;
                                // a filled band between two curves (top / bottom per point)
                                let band = |tops: &[f32], bots: &[f32], on: &dyn Fn(usize) -> bool, ct: Color32, cb: Color32| {
                                    let mut mesh = egui::Mesh::default();
                                    let mut prev = false;
                                    for i in 0..n {
                                        if !on(i) {
                                            prev = false;
                                            continue;
                                        }
                                        let f = fade(xs[i]);
                                        let b = mesh.vertices.len() as u32;
                                        mesh.vertices.push(egui::epaint::Vertex { pos: Pos2::new(xs[i], tops[i]), uv, color: ct.gamma_multiply(f) });
                                        mesh.vertices.push(egui::epaint::Vertex { pos: Pos2::new(xs[i], bots[i]), uv, color: cb.gamma_multiply(f) });
                                        if prev {
                                            mesh.indices.extend_from_slice(&[b - 2, b, b + 1, b - 2, b + 1, b - 1]);
                                        }
                                        prev = true;
                                    }
                                    painter.add(egui::Shape::mesh(mesh));
                                };
                                let rim = |ys: &[f32], on: &dyn Fn(usize) -> bool, st_: Stroke| {
                                    let mut line: Vec<Pos2> = vec![];
                                    for i in 0..n {
                                        if on(i) {
                                            line.push(Pos2::new(xs[i], ys[i]));
                                        } else if line.len() > 1 {
                                            faded_line(&painter, g.left(), std::mem::take(&mut line), st_);
                                        } else {
                                            line.clear();
                                        }
                                    }
                                    if line.len() > 1 {
                                        faded_line(&painter, g.left(), line, st_);
                                    }
                                };
                                let y_in: Vec<f32> = v_in.iter().map(|v| to_y(*v)).collect();
                                let y_out: Vec<f32> = v_out.iter().map(|v| to_y(*v)).collect();
                                let bottoms = vec![floor_y; n];
                                let has_in = |i: usize| v_in[i] > -150.0;
                                let has_out = |i: usize| v_out[i] > -150.0;
                                // input (with Gain): a soft veil behind
                                let col_in = Color32::from_rgb(0x9a, 0x6c, 0xb4);
                                band(&y_in, &bottoms, &has_in, col_in.gamma_multiply(0.30), col_in.gamma_multiply(0.04));
                                // output: lit at its edge, fading to the floor
                                let col_out = Color32::from_rgb(0xd2, 0xb6, 0xe2);
                                band(&y_out, &bottoms, &has_out, col_out.gamma_multiply(0.42), Color32::from_rgb(0x2e, 0x1e, 0x38).gamma_multiply(0.10));
                                // what the limiter took away: the red band between output and input
                                let cut = |i: usize| has_in(i) && has_out(i) && y_out[i] - y_in[i] > 0.6;
                                band(&y_in, &y_out, &cut, Color32::from_rgb(0xff, 0x5c, 0x8e).gamma_multiply(0.30), Color32::from_rgb(0xff, 0x4a, 0x7c).gamma_multiply(0.75));
                                rim(&y_in, &has_in, Stroke::new(1.0_f32, col_in.gamma_multiply(0.85)));
                                rim(&y_in, &cut, Stroke::new(1.3_f32, ROSE));
                                rim(&y_out, &has_out, Stroke::new(1.4_f32, Color32::from_rgb(0xf4, 0xe6, 0xfa)));
                                // loudness
                                let lp: Vec<Pos2> = (0..n).filter(|i| v_loud[*i] > -60.0).map(|i| Pos2::new(xs[i], to_y(v_loud[i]))).collect();
                                if lp.len() > 1 {
                                    faded_line(&painter, g.left(), lp, Stroke::new(1.5_f32, AMBER));
                                }
                                // gain-reduction peak labels: the largest GR within +-40 px, at least
                                // 64 px apart, sitting on the input crest
                                let mut last_x = -1000.0f32;
                                for i in 0..n {
                                    let v = pts[i].1.gr;
                                    if v < 0.5 || xs[i] < g.left() + 12.0 {
                                        continue;
                                    }
                                    let x = xs[i];
                                    let is_max = (0..n).filter(|j| (xs[*j] - x).abs() <= 40.0).all(|j| pts[j].1.gr < v || (pts[j].1.gr == v && j >= i));
                                    if is_max && x - last_x > 64.0 {
                                        let y = (y_in[i] - 12.0).clamp(g.top() + 8.0, g.bottom() - 8.0);
                                        let tr = Rect::from_center_size(Pos2::new(x.clamp(g.left() + 20.0, g.right() - 20.0), y), Vec2::new(38.0, 15.0));
                                        painter.rect_filled(tr, CornerRadius::same(8), ROSE.gamma_multiply(0.92 * fade(x)));
                                        painter.text(tr.center(), Align2::CENTER_CENTER, format!("-{:.1}", v), mono(9.0), Color32::from_rgb(0x22, 0x0a, 0x14).gamma_multiply(fade(x)));
                                        last_x = x;
                                    }
                                }
                            }
                        }
                    } else {
                        painter.text(g.center(), Align2::CENTER_CENTER, "display off", mono(11.0), DIM);
                    }
                    if disp_on {
                        // the ceiling as a rose tag on the scale, left of the graph
                        let lr = Rect::from_min_max(Pos2::new(dr.left() + 4.0, cy - 7.0), Pos2::new(g.left() - 2.0, cy + 7.0));
                        let resp = ui.allocate_rect(lr, Sense::hover());
                        painter.rect_filled(lr, CornerRadius::same(7), ROSE);
                        painter.text(lr.center(), Align2::CENTER_CENTER, format!("{:.1}", ceil_db), mono(9.0), Color32::from_rgb(0x22, 0x0a, 0x14));
                        resp.on_hover_text(ceil_txt);
                    }
                    // display mode + scale selectors on the display
                    if let Some(m) = list_combo(ui, "dmode", rect(dx + 10.0, top + 6.0, 112.0, 20.0), &DISPLAY_NAMES, ui_set.display as usize) {
                        ui_set.display = m as u8;
                        if m == 3 {
                            s.reset_history.store(true, Ordering::Relaxed);
                        }
                    }
                    // legend
                    // legend under the graph, aligned with it; latency at the right
                    let ly = dr.bottom() - 14.0;
                    let mut lx = dr.left() + 46.0;
                    for (name, c) in [("in", Color32::from_rgb(0x9a, 0x6c, 0xb4)), ("out", Color32::from_rgb(0xd2, 0xb6, 0xe2)), ("gain reduction", ROSE), ("loudness", AMBER)] {
                        painter.circle_filled(Pos2::new(lx + 3.5, ly), 3.5, c);
                        let tr = painter.text(Pos2::new(lx + 11.0, ly), Align2::LEFT_CENTER, name, mono(9.0), DIM);
                        lx = tr.right() + 16.0;
                    }
                    painter.text(Pos2::new(dr.right() - 16.0, ly), Align2::RIGHT_CENTER, &lat_txt, mono(9.0), DIM);
                }

                // ---- meters panel
                let unity = p.unity.value();
                if let Some(sc) = list_combo(ui, "scale", rect(mx + 8.0, top + 6.0, 96.0, 20.0), &SCALE_NAMES, scale as usize) {
                    ui_set.scale = sc as u8;
                }
                if tbtn(ui, rect(mx + 190.0 - 8.0 - 34.0, top + 6.0, 34.0, 20.0), "TP", ui_set.tp_meter, "True peak metering (BS.1770, oversampled)") {
                    ui_set.tp_meter = !ui_set.tp_meter;
                }
                // readouts: max peak and max gain reduction (click to reset)
                let max_pk = if ui_set.tp_meter { s.max_tp.load(Ordering::Relaxed) } else { s.max_peak.load(Ordering::Relaxed) };
                let max_gr = s.max_gr.load(Ordering::Relaxed);
                let pk_col = if max_pk > 0.0 {
                    CRIMSON
                } else if max_pk > ceil_db + 0.05 {
                    AMBER
                } else if ui_set.tp_meter {
                    LILAC
                } else {
                    WHITE
                };
                let r_pk = rect(mx + 8.0, top + 32.0, 84.0, 34.0);
                let r_gr = rect(mx + 98.0, top + 32.0, 84.0, 34.0);
                for (r, label, val, col) in [(r_pk, if ui_set.tp_meter { "PEAK dBTP" } else { "PEAK dB" }, dbs(max_pk), pk_col), (r_gr, "GR dB", gr_txt(max_gr), ROSE)] {
                    let resp = ui.allocate_rect(r, Sense::click()).on_hover_text("Click to reset");
                    painter.rect_filled(r, CornerRadius::same(12), Color32::from_black_alpha(if resp.hovered() { 40 } else { 70 }));
                    specular_rim(&painter, r, 12.0, 0.45);
                    painter.text(Pos2::new(r.center().x, r.top() + 9.0), Align2::CENTER_CENTER, label, mono(8.5), DIM);
                    painter.text(Pos2::new(r.center().x, r.top() + 23.0), Align2::CENTER_CENTER, val, mono(13.0), col);
                    if resp.clicked() {
                        s.reset_peaks.store(true, Ordering::Relaxed);
                    }
                }
                let mtop = top + 76.0;
                let mbot = top + main_h - 26.0;
                if scale != 6 {
                    // level meters: L / R peak (+ momentary RMS inside) and the gain reduction
                    let mh = mbot - mtop;
                    let ypos = |db: f32| mbot - mh * scale_pos(scale, db);
                    let bw = if ui_set.compact { 40.0 } else { 26.0 };
                    let x0 = mx + 30.0;
                    for t in scale_ticks(scale) {
                        let lab = k_label(scale, t).unwrap_or_else(|| format!("{t:.0}"));
                        painter.text(Pos2::new(x0 - 6.0, ypos(t)), Align2::RIGHT_CENTER, lab, mono(8.5), DIM);
                    }
                    for c in 0..nch {
                        let bx = x0 + c as f32 * (bw + 6.0);
                        let br = Rect::from_min_max(Pos2::new(bx, mtop), Pos2::new(bx + bw, mbot));
                        painter.rect_filled(br, CornerRadius::same(7), Color32::from_black_alpha(90));
                        let pk = st.out_lv[c];
                        let col = if unity {
                            AMBER
                        } else if pk > -0.1 {
                            CRIMSON
                        } else {
                            LILAC
                        };
                        painter.rect_filled(Rect::from_min_max(Pos2::new(bx, ypos(pk)), Pos2::new(bx + bw, mbot)), CornerRadius::same(7), col.gamma_multiply(0.85));
                        let rms = s.out_rms[c].load(Ordering::Relaxed);
                        // K-System: above 0 VU the RMS bar turns rose
                        let k_zone = match scale {
                            3 => -12.0,
                            4 => -14.0,
                            5 => -20.0,
                            _ => 1.0,
                        };
                        let rcol = if rms > k_zone { ROSE } else { WHITE };
                        painter.rect_filled(Rect::from_min_max(Pos2::new(bx + bw * 0.3, ypos(rms)), Pos2::new(bx + bw * 0.7, mbot)), CornerRadius::same(4), rcol);
                        painter.hline(bx..=bx + bw, ypos(st.out_hold[c]), Stroke::new(2.0_f32, WHITE));
                        painter.text(Pos2::new(bx + bw * 0.5, mbot + 8.0), Align2::CENTER_CENTER, if nch == 1 { "M" } else if c == 0 { "L" } else { "R" }, mono(8.5), DIM);
                    }
                    // gain reduction meter (from the top)
                    let gx = x0 + nch as f32 * (bw + 6.0) + 10.0;
                    let gr_r = Rect::from_min_max(Pos2::new(gx, mtop), Pos2::new(gx + 16.0, mbot));
                    painter.rect_filled(gr_r, CornerRadius::same(7), Color32::from_black_alpha(90));
                    let gnow = st.gr_lv;
                    let gh = (ypos(-gnow) - ypos(0.0)).min(mh);
                    painter.rect_filled(Rect::from_min_max(gr_r.min, Pos2::new(gr_r.right(), gr_r.top() + gh)), CornerRadius::same(7), CRIMSON);
                    painter.text(Pos2::new(gr_r.center().x, mbot + 8.0), Align2::CENTER_CENTER, "GR", mono(8.5), DIM);
                    // momentary loudness readout
                    // momentary loudness readout (the compact view shows it in the big tiles)
                    if !ui_set.compact {
                        let m = s.momentary.load(Ordering::Relaxed);
                        painter.text(Pos2::new(mx + 180.0, mtop + 6.0), Align2::RIGHT_CENTER, "LUFS M", mono(8.0), DIM);
                        painter.text(Pos2::new(mx + 180.0, mtop + 21.0), Align2::RIGHT_CENTER, dbs(m), mono(11.0), AMBER);
                    }
                } else {
                    // ---- loudness panel
                    let target = ui_set.target;
                    let lu_top = if ui_set.lscale == 0 { 9.0 } else { 18.0 };
                    let lu_range = 36.0;
                    let mh = mbot - mtop - 70.0;
                    let lt = mtop + 46.0;
                    let lb = lt + mh;
                    let ypos = |lufs: f32| lb - mh * (((lufs - target) - (lu_top - lu_range)) / lu_range).clamp(0.0, 1.0);
                    let vals = [s.momentary.load(Ordering::Relaxed), s.short_term.load(Ordering::Relaxed), s.integrated.load(Ordering::Relaxed)];
                    let names = ["M", "S", "I"];
                    let sel = ui_set.ltime.min(2) as usize;
                    // time scale buttons + pause / reset
                    for k in 0..3 {
                        if tbtn(ui, rect(mx + 8.0 + k as f32 * 28.0, mtop - 2.0, 26.0, 20.0), names[k], sel == k, ["Momentary (400 ms)", "Short Term (3 s)", "Integrated (since reset, gated)"][k]) {
                            ui_set.ltime = k as u8;
                        }
                    }
                    let paused = s.pause_loudness.load(Ordering::Relaxed);
                    if tbtn(ui, rect(mx + 96.0, mtop - 2.0, 26.0, 20.0), "‖", paused, "Pause the measurement") {
                        s.pause_loudness.store(!paused, Ordering::Relaxed);
                    }
                    if tbtn(ui, rect(mx + 124.0, mtop - 2.0, 26.0, 20.0), "↺", false, "Reset the measurement (and the peak readouts)") {
                        s.reset_loudness.store(true, Ordering::Relaxed);
                    }
                    if tbtn(ui, rect(mx + 152.0, mtop - 2.0, 30.0, 20.0), "AR", ui_set.auto_reset, "Auto-Reset when the host starts playing") {
                        ui_set.auto_reset = !ui_set.auto_reset;
                    }
                    // target + scale + relative
                    let tnames: Vec<String> = TARGETS.iter().map(|t| t.1.to_string()).chain(std::iter::once(format!("{:.1} custom", target))).collect();
                    let tn: Vec<&str> = tnames.iter().map(|s| s.as_str()).collect();
                    let tcur = TARGETS.iter().position(|t| (t.0 - target).abs() < 0.01).unwrap_or(4);
                    if let Some(k) = list_combo(ui, "target", rect(mx + 8.0, mtop + 22.0, 120.0, 20.0), &tn, tcur) {
                        if k < 4 {
                            ui_set.target = TARGETS[k].0;
                        } else if st.custom_target.is_empty() {
                            st.custom_target = format!("{:.1}", target);
                        }
                    }
                    if tbtn(ui, rect(mx + 132.0, mtop + 22.0, 50.0, 20.0), if ui_set.lscale == 0 { "+9 LU" } else { "+18 LU" }, false, "Meter scale: +9 or +18 LU above the target") {
                        ui_set.lscale = 1 - ui_set.lscale.min(1);
                    }
                    if !st.custom_target.is_empty() {
                        let te = egui::TextEdit::singleline(&mut st.custom_target).font(mono(11.0)).desired_width(60.0);
                        let r = ui.put(rect(mx + 8.0, mtop + 44.0, 70.0, 18.0), te);
                        r.request_focus();
                        if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            if let Ok(v) = st.custom_target.trim().trim_end_matches("lufs").trim().parse::<f32>() {
                                ui_set.target = v.clamp(-40.0, 0.0);
                            }
                            st.custom_target.clear();
                        }
                    }
                    // big meter + two small
                    let bx = mx + 34.0;
                    for (k, v) in vals.iter().enumerate() {
                        let (x, w) = if k == sel { (bx, 34.0) } else { (bx + 44.0 + if k > sel { (k - 1) as f32 } else { k as f32 } * 16.0, 10.0) };
                        let br = Rect::from_min_max(Pos2::new(x, lt), Pos2::new(x + w, lb));
                        painter.rect_filled(br, CornerRadius::same(7), Color32::from_black_alpha(90));
                        if *v > -100.0 {
                            let col = if *v > target + 1.0 { ROSE } else if *v > target - 1.0 { WHITE } else { AMBER };
                            painter.rect_filled(Rect::from_min_max(Pos2::new(x, ypos(*v)), Pos2::new(x + w, lb)), CornerRadius::same(7), col);
                        }
                        painter.text(Pos2::new(x + w * 0.5, lb + 8.0), Align2::CENTER_CENTER, names[k], mono(8.5), DIM);
                    }
                    // target line + labels
                    painter.hline(bx - 4.0..=bx + 80.0, ypos(target), Stroke::new(1.0_f32, ROSE));
                    for lu in [lu_top, lu_top - 9.0, 0.0, -9.0, -18.0, lu_top - lu_range] {
                        let y = ypos(target + lu);
                        let lab = if ui_set.relative || (target + 23.0).abs() < 0.01 { format!("{:+.0}", lu) } else { format!("{:.0}", target + lu) };
                        painter.text(Pos2::new(bx - 6.0, y), Align2::RIGHT_CENTER, lab, mono(8.0), DIM);
                    }
                    // readouts
                    let v = vals[sel];
                    let shown = if ui_set.relative { v - target } else { v };
                    let unit = if ui_set.relative { "LU" } else { "LUFS" };
                    let rx = mx + 128.0;
                    painter.text(Pos2::new(rx + 26.0, lt + 10.0), Align2::CENTER_CENTER, names[sel], mono(9.0), DIM);
                    painter.text(Pos2::new(rx + 26.0, lt + 28.0), Align2::CENTER_CENTER, if v > -100.0 { if ui_set.relative { format!("{shown:+.1}") } else { format!("{shown:.1}") } } else { "-inf".into() }, mono(15.0), WHITE);
                    painter.text(Pos2::new(rx + 26.0, lt + 44.0), Align2::CENTER_CENTER, unit, mono(8.5), DIM);
                    {
                        let mut yy = lt + 70.0;
                        if sel != 2 {
                            let iv = vals[2];
                            painter.text(Pos2::new(rx + 26.0, yy), Align2::CENTER_CENTER, "I", mono(8.5), DIM);
                            painter.text(Pos2::new(rx + 26.0, yy + 14.0), Align2::CENTER_CENTER, if iv > -100.0 { if ui_set.relative { format!("{:+.1}", iv - target) } else { format!("{iv:.1}") } } else { "-inf".into() }, mono(11.0), WHITE);
                            yy += 36.0;
                        }
                        let lra = s.lra.load(Ordering::Relaxed);
                        painter.text(Pos2::new(rx + 26.0, yy), Align2::CENTER_CENTER, "LRA", mono(8.5), DIM);
                        painter.text(Pos2::new(rx + 26.0, yy + 14.0), Align2::CENTER_CENTER, format!("{lra:.1} LU"), mono(11.0), WHITE);
                        let mm = if ui_set.max_s { s.max_s.load(Ordering::Relaxed) } else { s.max_m.load(Ordering::Relaxed) };
                        let r = rect(rx, yy + 30.0, 52.0, 34.0);
                        let resp = ui.allocate_rect(r, Sense::click()).on_hover_text("Click: Max Momentary / Max Short Term");
                        painter.text(Pos2::new(r.center().x, r.top() + 8.0), Align2::CENTER_CENTER, if ui_set.max_s { "MAX S" } else { "MAX M" }, mono(8.5), DIM);
                        painter.text(Pos2::new(r.center().x, r.top() + 22.0), Align2::CENTER_CENTER, if mm > -100.0 { if ui_set.relative { format!("{:+.1}", mm - target) } else { format!("{mm:.1}") } } else { "-inf".into() }, mono(11.0), WHITE);
                        if resp.clicked() {
                            ui_set.max_s = !ui_set.max_s;
                        }
                    }
                    if tbtn(ui, rect(rx, lb - 20.0, 52.0, 20.0), if ui_set.relative { "REL" } else { "ABS" }, ui_set.relative, "Absolute LUFS or relative to the target (LU)") {
                        ui_set.relative = !ui_set.relative;
                    }
                }

                // ================= bottom bar
                let by = top + main_h + 10.0;
                let bh = BASE_H - 12.0 - by;
                let br = rect(14.0, by, BASE_W - 28.0, bh);
                glass(&painter, br, false);
                // style
                painter.text(at(28.0, by + 14.0), Align2::LEFT_CENTER, "STYLE", mono(9.0), LILAC);
                let style = p.style.value();
                if let Some(nv) = enum_combo::<StyleParam>(ui, "style", rect(26.0, by + 26.0, 138.0, 22.0), style, style.name()) {
                    set_e(setter, &p.style, nv);
                }
                let about = style.about();
                // word-wrap to the column under the style box
                let mut lines: Vec<String> = vec![String::new()];
                for w in about.split(' ') {
                    let l = lines.last_mut().unwrap();
                    if !l.is_empty() && l.len() + 1 + w.chars().count() > 25 {
                        lines.push(w.to_string());
                    } else {
                        if !l.is_empty() {
                            l.push(' ');
                        }
                        l.push_str(w);
                    }
                }
                for (k, l) in lines.iter().take(3).enumerate() {
                    painter.text(at(28.0, by + 62.0 + k as f32 * 13.0), Align2::LEFT_CENTER, l, mono(9.0), DIM);
                }
                if tbtn(ui, rect(26.0, by + bh - 28.0, 92.0, 20.0), "ADVANCED", ui_set.advanced, "Show the look-ahead, attack / release and channel linking") {
                    ui_set.advanced = !ui_set.advanced;
                }
                // advanced knobs
                let ky = by + 44.0;
                let mut kx = 210.0;
                if ui_set.advanced {
                    knob(ui, setter, &p.lookahead, "LOOKAHEAD", "look", at(kx, ky), 22.0, "Look-ahead of the transient stage: shorter = louder and punchier but risks distortion, longer = safer.", false);
                    kx += 76.0;
                    knob(ui, setter, &p.attack, "ATTACK", "att", at(kx, ky), 22.0, "Attack of the release stage: shorter lets it take over sooner.", false);
                    kx += 76.0;
                    knob(ui, setter, &p.release, "RELEASE", "rel", at(kx, ky), 22.0, "Release of the release stage: longer = cleaner but more pumping, shorter = louder, more distortion.", false);
                    // channel link group: same size and baseline as the others, separators
                    // centred in the gaps
                    painter.vline(o.x + kx + 38.0, (o.y + by + 14.0)..=(o.y + by + bh - 14.0), Stroke::new(1.0_f32, Color32::from_white_alpha(12)));
                    kx += 76.0;
                    painter.text(at(kx + 38.0, by + 12.0), Align2::CENTER_CENTER, "CHANNEL LINK", mono(8.5), LILAC);
                    knob(ui, setter, &p.link_transient, "TRANSIENTS", "lt", at(kx, ky), 22.0, "Linking of the transient stage (short peaks): often best below 100 %.", false);
                    kx += 76.0;
                    knob(ui, setter, &p.link_release, "RELEASE", "lr", at(kx, ky), 22.0, "Linking of the release stage: 100 % keeps the stereo image steady.", false);
                    kx += 38.0;
                } else {
                    painter.text(at(kx + 120.0, ky), Align2::CENTER_CENTER, "the style sets the timing - open ADVANCED to fine-tune it", mono(9.5), DIM);
                    kx += 340.0;
                }
                painter.vline(o.x + kx, (o.y + by + 14.0)..=(o.y + by + bh - 14.0), Stroke::new(1.0_f32, Color32::from_white_alpha(12)));
                // output section
                let ox = kx + 38.0;
                knob(ui, setter, &p.output, "OUTPUT", "out", at(ox, ky), 22.0, "Output Level: the ceiling (dBTP with True Peak Limiting).", false);
                let cx = ox + 44.0;
                let tp = p.true_peak.value();
                if tbtn(ui, rect(cx, by + 14.0, 96.0, 20.0), "TRUE PEAK", tp, "True peak limiting: no inter-sample peak above the Output Level (uses at least 4x oversampling)") {
                    set_b(setter, &p.true_peak, !tp);
                }
                painter.text(at(cx, by + 47.0), Align2::LEFT_CENTER, "OS", mono(9.0), DIM);
                let osv = p.oversampling.value();
                if let Some(nv) = enum_combo::<OsParam>(ui, "os", rect(cx + 20.0, by + 37.0, 76.0, 20.0), osv, OsParam::variants()[osv.to_index()]) {
                    set_e(setter, &p.oversampling, nv);
                }
                painter.text(at(cx, by + 73.0), Align2::LEFT_CENTER, "DITHER", mono(9.0), DIM);
                let dv = p.dither.value();
                if let Some(nv) = enum_combo::<DitherParam>(ui, "dither", rect(cx + 44.0, by + 63.0, 56.0, 20.0), dv, DitherParam::variants()[dv.to_index()]) {
                    set_e(setter, &p.dither, nv);
                }
                let sv = p.shaping.value();
                if let Some(nv) = enum_combo::<ShapingParam>(ui, "shape", rect(cx + 104.0, by + 63.0, 148.0, 20.0), sv, ShapingParam::variants()[sv.to_index()]) {
                    set_e(setter, &p.shaping, nv);
                }
                // switches
                let sx = cx + 104.0;
                let sw: [(&str, &BoolParam, &str); 4] = [
                    ("DC", &p.dc_filter, "Filter DC offset (avoids lop-sided limiting)"),
                    ("UNITY", &p.unity, "Unity Gain: Output Level = -Gain, to compare the limiting at equal level (meters turn amber)"),
                    ("AUDITION", &p.audition, "Audition limiting: hear only what the limiter takes away"),
                    ("SIDECHAIN", &p.sidechain, "External side chain drives the limiter"),
                ];
                for (k, (name, bp, tip)) in sw.iter().enumerate() {
                    let r = rect(sx + (k % 2) as f32 * 76.0, by + 14.0 + (k / 2) as f32 * 23.0, 72.0, 20.0);
                    let v = bp.value();
                    if tbtn(ui, r, name, v, tip) {
                        set_b(setter, bp, !v);
                    }
                }

                // resize grip
                let grip = Rect::from_min_size(full.max - Vec2::splat(16.0), Vec2::splat(16.0));
                let r = ui.interact(grip, egui::Id::new("limit_grip"), Sense::drag()).on_hover_text("Drag to resize");
                if r.dragged() {
                    if let Some(pos) = r.interact_pointer_pos() {
                        let ppp = if cfg!(target_os = "macos") { size_state.size().0 as f32 / BASE_W } else { ctx.pixels_per_point() };
                        let w = ((pos.x - full.min.x + 8.0) * ppp).clamp(740.0, 2900.0);
                        size_state.set_requested_size((w.round() as u32, (w * BASE_H / BASE_W).round() as u32));
                    }
                }
                let gc = if r.hovered() || r.dragged() { WHITE } else { DIM };
                for k in [5.0f32, 9.0] {
                    painter.line_segment([Pos2::new(full.max.x - 3.0, full.max.y - k), Pos2::new(full.max.x - k, full.max.y - 3.0)], Stroke::new(1.0_f32, gc));
                }
            });
            if ui_set != ui_before {
                *p.ui.write() = ui_set;
            }
        },
    )
}
