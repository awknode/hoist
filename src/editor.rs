//! HOIST editor: graphite + safety yellow, hazard stripes, three big lift faders
//! (LOW LEVEL, HIGH LEVEL, OUTPUT) and a meter column (IN, LIFT ±, OUT) with the net gain.

use crate::awk_ui::*;
use crate::presets::PresetBar;
use crate::{get, HoistParams, Shared, HIGH_MAX, LOW_MAX};
use nice_plug::context::gui::GuiContext;
use nice_plug::prelude::*;
use nice_plug_egui::{create_egui_editor, EguiEditor, EguiEditorState, EguiNiceSettings, NiceEguiApp, RepaintNotifier};
use egui::{pos2, vec2, Align2, Color32, Rect, Response, Sense, Shape, Stroke};
use std::sync::Arc;

pub const WIDTH: u32 = 780;
pub const HEIGHT: u32 = 466;
const TOP: f32 = 44.0;

const BG: Color32 = rgb(0x14, 0x15, 0x16);
const PANEL: Color32 = rgb(0x1c, 0x1d, 0x1f);
const WELL: Color32 = rgb(0x0b, 0x0b, 0x0c);
const EDGE: Color32 = rgb(0x2c, 0x2e, 0x31);
const TXT: Color32 = rgb(0xec, 0xeb, 0xe6);
const DIM: Color32 = rgb(0x8b, 0x8a, 0x85);
const FAINT: Color32 = rgb(0x4a, 0x4a, 0x47);
const YEL: Color32 = rgb(0xff, 0xc2, 0x1a);
const RED: Color32 = rgb(0xff, 0x4a, 0x3d);
const OFF: Color32 = rgb(0x26, 0x27, 0x29);

pub(crate) struct St {
    presets: PresetBar,
    skin: Skin,
}
fn new_state(name: &'static str) -> St {
    St { presets: PresetBar::new(name), skin: Skin::new(Look::Machined, YEL, "HOIST", env!("CARGO_PKG_VERSION"), "") }
}

/// The editor app (nice-plug-egui): lives as long as the plugin, the GUI context only
/// while the window is open.
pub struct HoistEditor {
    params: Arc<HoistParams>,
    shared: Arc<Shared>,
    st: St,
    gui: Option<GuiContext>,
}
impl NiceEguiApp for HoistEditor {
    fn build(&mut self, egui_ctx: egui::Context, gui: GuiContext, _f: &mut nice_plug_egui::Frame) -> Result<(), nice_plug_egui::baseview::HandlerError> {
        install_fonts(&egui_ctx);
        self.gui = Some(gui);
        Ok(())
    }
    fn ui(&mut self, ui: &mut egui::Ui, _f: &mut nice_plug_egui::Frame) {
        let Some(gui) = self.gui.as_ref() else { return };
        let setter = gui.param_setter();
        frame(ui, &setter, &mut self.st, &self.params, &self.shared);
    }
    fn editor_closed(&mut self) {
        self.gui = None;
    }
}
pub(crate) fn create(state: Arc<EguiEditorState>, repaint: RepaintNotifier, params: Arc<HoistParams>, shared: Arc<Shared>) -> Option<EguiEditor<HoistEditor>> {
    crate::presets::apply_scale(&state, &params.ui_scale);
    create_egui_editor(state, repaint, EguiNiceSettings::default(), HoistEditor { params, shared, st: new_state("Hoist"), gui: None })
}

fn alpha(c: Color32, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (a.clamp(0.0, 1.0) * 255.0) as u8)
}
fn set_bool(setter: &ParamSetter, p: &BoolParam, v: bool) {
    setter.begin_set_parameter(p);
    setter.set_parameter(p, v);
    setter.end_set_parameter(p);
}
fn hazard(pa: &Painter, r: Rect) {
    let pa = pa.with_clip_rect(r);
    pa.rect_filled(r, 0.0, YEL);
    let mut x = r.left() - r.height();
    while x < r.right() {
        pa.add(Shape::convex_polygon(vec![pos2(x, r.bottom()), pos2(x + 10.0, r.bottom()), pos2(x + 10.0 + r.height(), r.top()), pos2(x + r.height(), r.top())], rgb(0x17, 0x17, 0x17), Stroke::NONE));
        x += 20.0;
    }
}
use egui::Painter;

fn pill(ui: &mut egui::Ui, r: Rect, s: &str, on: bool, id: impl std::hash::Hash + std::fmt::Debug) -> Response {
    let resp = ui.interact(r, ui.scope_id().with(("pill", id)), Sense::click());
    let pa = ui.painter();
    pa.rect_filled(r, r.height() * 0.5, if on { YEL } else if resp.hovered() { rgb(0x2a, 0x2b, 0x2e) } else { PANEL });
    pa.rect_stroke(r, r.height() * 0.5, Stroke::new(1.0, if on { YEL } else { EDGE }), egui::StrokeKind::Inside);
    txt(pa, r.center(), Align2::CENTER_CENTER, s, labb(12.0), if on { BG } else { DIM }, 1.5);
    resp
}

/// A lift fader: recessed slot, dB scale, a yellow fill up to the cap, a chunky cap.
/// `marks`: (plain value, label). Drag = move, Shift = fine, double-click = default, wheel = nudge.
#[allow(clippy::too_many_arguments)]
fn fader(ui: &mut egui::Ui, setter: &ParamSetter, param: &FloatParam, cx: f32, top: f32, bot: f32, label: &str, marks: &[(f32, &str)], from_mid: bool, fmt: &dyn Fn(f32) -> String) -> Response {
    let col = Rect::from_min_max(pos2(cx - 40.0, top - 14.0), pos2(cx + 40.0, bot + 14.0));
    let id = ui.scope_id().with(("fader", label));
    let resp = ui.interact(col, id, Sense::click_and_drag());
    let span = bot - top;
    let mut v = param.unmodulated_normalized_value();
    if resp.double_clicked() {
        setter.begin_set_parameter(param);
        setter.set_parameter_normalized(param, param.default_normalized_value());
        setter.end_set_parameter(param);
        v = param.default_normalized_value();
    } else {
        if resp.drag_started() {
            setter.begin_set_parameter(param);
        }
        if resp.dragged() {
            let fine = ui.input(|i| i.modifiers.shift);
            let dy = resp.drag_delta().y;
            v = (v - dy / span * if fine { 0.15 } else { 1.0 }).clamp(0.0, 1.0);
            setter.set_parameter_normalized(param, v);
        }
        if resp.drag_stopped() {
            setter.end_set_parameter(param);
        }
        if resp.hovered() {
            let w: f32 = ui.input(|i| i.events.iter().map(|e| if let egui::Event::MouseWheel { delta, .. } = e { delta.y } else { 0.0 }).sum());
            if w != 0.0 {
                v = (v + w.signum() * 0.01).clamp(0.0, 1.0);
                setter.begin_set_parameter(param);
                setter.set_parameter_normalized(param, v);
                setter.end_set_parameter(param);
            }
        }
    }
    let hot = resp.hovered() || resp.dragged();
    let pa = ui.painter();
    let y_of = |n: f32| bot - n * span;
    // scale
    for (val, s) in marks {
        let y = y_of(param.preview_normalized(*val));
        pa.line_segment([pos2(cx - 30.0, y), pos2(cx - 20.0, y)], Stroke::new(1.0, FAINT));
        pa.line_segment([pos2(cx + 20.0, y), pos2(cx + 30.0, y)], Stroke::new(1.0, FAINT));
        txt(pa, pos2(cx - 34.0, y), Align2::RIGHT_CENTER, *s, mono(9.0), DIM, 0.0);
    }
    // slot
    let slot = Rect::from_min_max(pos2(cx - 4.0, top), pos2(cx + 4.0, bot));
    pa.rect_filled(slot.expand(2.0), 5.0, rgb(0x08, 0x08, 0x09));
    pa.rect_filled(slot, 4.0, WELL);
    let y = y_of(v);
    let base = if from_mid { y_of(param.preview_normalized(0.0)) } else { bot };
    let fill = Rect::from_min_max(pos2(cx - 3.0, y.min(base)), pos2(cx + 3.0, y.max(base)));
    if fill.height() > 0.5 {
        pa.rect_filled(fill.expand2(vec2(5.0, 0.0)), 4.0, alpha(YEL, 0.12));
        pa.rect_filled(fill, 3.0, YEL);
    }
    // cap
    let cap = Rect::from_center_size(pos2(cx, y), vec2(56.0, 30.0));
    for k in 0..5 {
        pa.rect_filled(cap.translate(vec2(0.0, 3.0 + k as f32)).expand(k as f32), 6.0, Color32::from_black_alpha(26));
    }
    pa.rect_filled(cap, 5.0, if hot { rgb(0x3a, 0x3b, 0x3e) } else { rgb(0x2e, 0x2f, 0x32) });
    pa.rect_filled(Rect::from_min_max(cap.min, pos2(cap.right(), cap.top() + 12.0)), egui::CornerRadius { nw: 5, ne: 5, sw: 0, se: 0 }, rgb(0x44, 0x45, 0x48));
    pa.rect_stroke(cap, 5.0, Stroke::new(1.0, Color32::from_white_alpha(24)), egui::StrokeKind::Inside);
    for gy in [-6.0, -2.0, 2.0, 6.0] {
        pa.line_segment([pos2(cap.left() + 8.0, cap.center().y + gy), pos2(cap.right() - 8.0, cap.center().y + gy)], Stroke::new(1.0, Color32::from_black_alpha(90)));
    }
    pa.rect_filled(Rect::from_center_size(cap.center(), vec2(cap.width() - 4.0, 2.0)), 1.0, YEL);
    // label + value
    txt(pa, pos2(cx, top - 30.0), Align2::CENTER_CENTER, label, labb(14.0), if hot { TXT } else { DIM }, 2.5);
    let s = fmt(param.value());
    txt(pa, pos2(cx, bot + 30.0), Align2::CENTER_CENTER, s, mono(13.0), YEL, 0.0);
    resp
}

fn bar(pa: &Painter, r: Rect, u: f32, col: Color32, label: &str) {
    pa.rect_filled(r, 3.0, WELL);
    let n = 30;
    let h = r.height() / n as f32;
    for i in 0..n {
        let p = (i as f32 + 0.5) / n as f32;
        let seg = Rect::from_min_max(pos2(r.left() + 2.0, r.bottom() - (i + 1) as f32 * h + 1.0), pos2(r.right() - 2.0, r.bottom() - i as f32 * h - 1.0));
        pa.rect_filled(seg, 1.0, if p <= u { col } else { OFF });
    }
    txt(pa, pos2(r.center().x, r.bottom() + 14.0), Align2::CENTER_CENTER, label, labb(11.0), DIM, 1.5);
}

fn frame(root: &mut egui::Ui, setter: &ParamSetter, st: &mut St, p: &HoistParams, sh: &Shared) {
    let ctx = root.ctx().clone();
    ctx.request_repaint_after(std::time::Duration::from_millis(16));
    st.skin.begin(&ctx);
    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(BG)).show(root, |ui| {
        let whole = ui.max_rect();
        let o = whole.min.to_vec2() + vec2(30.0, TOP);
        let at = |x: f32, y: f32| pos2(x, y) + o;
        let pa = ui.painter().clone();
        // header
        txt(&pa, at(24.0, 34.0), Align2::LEFT_CENTER, "HOIST", labb(38.0), TXT, 6.0);
        txt(&pa, at(156.0, 28.0), Align2::LEFT_CENTER, "PARALLEL LOUDNESS", mono(10.0), YEL, 1.5);
        txt(&pa, at(156.0, 42.0), Align2::LEFT_CENTER, "UP + DOWN // ZERO LATENCY", mono(10.0), DIM, 1.5);
        hazard(&pa, Rect::from_min_size(at(520.0, 22.0), vec2(180.0, 22.0)));
        // fader deck
        let deck = Rect::from_min_max(at(16.0, 60.0), at(460.0, 370.0));
        pa.rect_filled(deck, 10.0, PANEL);
        pa.rect_stroke(deck, 10.0, Stroke::new(1.0, EDGE), egui::StrokeKind::Inside);
        let (top, bot) = (o.y + 120.0, o.y + 318.0);
        let lm = [(0.0, "0"), (12.0, "12"), (24.0, "24"), (36.0, "36"), (48.0, "48")];
        let hm = [(0.0, "0"), (12.0, "12"), (24.0, "24"), (36.0, "36"), (48.0, "48")];
        let om = [(-24.0, "-24"), (-12.0, "-12"), (-6.0, "-6"), (0.0, "0"), (6.0, "+6"), (12.0, "+12")];
        let db = |v: f32| format!("{:.1} dB", v);
        let speed_s = |v: f32| format!("{} {:.0}%", if v < 0.34 { "SMOOTH" } else if v < 0.67 { "MED" } else { "FAST" }, v * 100.0);
        let sm = [(0.0, "SMOOTH"), (0.5, "MED"), (1.0, "FAST")];
        fader(ui, setter, &p.low, o.x + 96.0, top, bot, "LOW LEVEL", &lm, false, &db).on_hover_text("Upward compression: lifts the quiet parts (up to +48 dB). Loud parts are left alone.");
        fader(ui, setter, &p.high, o.x + 202.0, top, bot, "HIGH LEVEL", &hm, false, &db).on_hover_text("Holds the peaks and makes the rest up: everything gets louder and denser.");
        fader(ui, setter, &p.speed, o.x + 308.0, top, bot, "SPEED", &sm, false, &speed_s).on_hover_text("How fast the lift comes back after a hit. SMOOTH: glued, no pumping. FAST: aggressive, everything up. Double-click: 50%");
        fader(ui, setter, &p.out, o.x + 404.0, top, bot, "OUTPUT", &om, true, &|v: f32| format!("{:+.1} dB", v)).on_hover_text("Output level. Double-click: 0 dB");
        // meters
        let mp = Rect::from_min_max(at(472.0, 60.0), at(704.0, 370.0));
        pa.rect_filled(mp, 10.0, PANEL);
        pa.rect_stroke(mp, 10.0, Stroke::new(1.0, EDGE), egui::StrokeKind::Inside);
        let net = get(&sh.net);
        txt(&pa, at(588.0, 88.0), Align2::CENTER_CENTER, format!("{:+.1}", net), labb(34.0), YEL, 1.0);
        txt(&pa, at(588.0, 112.0), Align2::CENTER_CENTER, "dB NET GAIN", mono(9.0), DIM, 1.5);
        let u = |d: f32| ((d + 48.0) / 54.0).clamp(0.0, 1.0);
        let m = &sh.meters;
        let (bt, bb) = (o.y + 130.0, o.y + 336.0);
        bar(&pa, Rect::from_min_max(pos2(o.x + 512.0, bt), pos2(o.x + 540.0, bb)), u(m.db(0).max(m.db(1))), TXT, "IN");
        bar(&pa, Rect::from_min_max(pos2(o.x + 636.0, bt), pos2(o.x + 664.0, bb)), u(m.db(2).max(m.db(3))), YEL, "OUT");
        // LIFT: bipolar, up = lift (yellow), down = reduction (red), +-24 dB
        let lr = Rect::from_min_max(pos2(o.x + 574.0, bt), pos2(o.x + 602.0, bb));
        pa.rect_filled(lr, 3.0, WELL);
        let mid = lr.center().y;
        let half = lr.height() * 0.5;
        let up = (get(&sh.up) / 24.0).clamp(0.0, 1.0);
        let dn = (get(&sh.down) / 24.0).clamp(0.0, 1.0);
        let n = 15;
        let h = half / n as f32;
        for i in 0..n {
            let p = (i as f32 + 0.5) / n as f32;
            let a = Rect::from_min_max(pos2(lr.left() + 2.0, mid - (i + 1) as f32 * h + 1.0), pos2(lr.right() - 2.0, mid - i as f32 * h - 1.0));
            pa.rect_filled(a, 1.0, if p <= up { YEL } else { OFF });
            let b = Rect::from_min_max(pos2(lr.left() + 2.0, mid + i as f32 * h + 1.0), pos2(lr.right() - 2.0, mid + (i + 1) as f32 * h - 1.0));
            pa.rect_filled(b, 1.0, if p <= dn { RED } else { OFF });
        }
        pa.line_segment([pos2(lr.left() - 4.0, mid), pos2(lr.right() + 4.0, mid)], Stroke::new(1.0, DIM));
        txt(&pa, pos2(lr.center().x, bb + 14.0), Align2::CENTER_CENTER, "LIFT", labb(11.0), DIM, 1.5);
        // footer
        let fr = Rect::from_min_max(pos2(whole.left(), o.y + 382.0), whole.right_bottom());
        pa.rect_filled(fr, 0.0, rgb(0x0e, 0x0e, 0x0f));
        pa.line_segment([fr.left_top(), fr.right_top()], Stroke::new(1.0, EDGE));
        let cy = fr.center().y;
        let link = p.link.value();
        if pill(ui, Rect::from_center_size(pos2(o.x + 56.0, cy), vec2(76.0, 20.0)), "LINK", link, "link").on_hover_text("One gain for both channels (off: left and right on their own)").clicked() {
            set_bool(setter, &p.link, !link);
        }
        let safe = p.safe.value();
        let lim = -m.db(4);
        let ls = if !safe { "SAFE OFF".to_string() } else if lim > 0.1 { format!("SAFE -{:.1}", lim) } else { "SAFE 0 dB".to_string() };
        if pill(ui, Rect::from_center_size(pos2(o.x + 150.0, cy), vec2(96.0, 20.0)), &ls, safe, "safe").on_hover_text("Never over 0 dBFS").clicked() {
            set_bool(setter, &p.safe, !safe);
        }
        txt(&pa, pos2(o.x + 214.0, cy), Align2::LEFT_CENTER, "LATENCY 0", mono(10.0), DIM, 1.5);
        txt(&pa, pos2(o.x + 704.0, cy), Align2::RIGHT_CENTER, format!("awk studios · hoist v{}", env!("CARGO_PKG_VERSION")), mono(10.0), DIM, 1.0);
        let _ = (LOW_MAX, HIGH_MAX);
        let _ = st.presets.strip(ui, setter, p, whole, &st.skin, &p.mix, &p.bypass, &p.ui_scale);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Engine, State};
    use nice_plug::context::gui::GuiContextInner;
    use nice_plug::plugin::PluginState;
    use egui::{epaint, Event, Pos2, ImageData, Modifiers, PointerButton, RawInput, TextureId, TextureWrapMode, Vec2};
    use std::collections::HashMap;

    struct Mock;
    impl GuiContextInner for Mock {
        fn plugin_api(&self) -> PluginApi {
            PluginApi::Vst3
        }
        fn request_restart(&self) {}
        unsafe fn raw_begin_set_parameter(&self, _p: ParamPtr) {}
        unsafe fn raw_set_parameter_normalized(&self, _p: ParamPtr, _n: f32) {}
        unsafe fn raw_end_set_parameter(&self, _p: ParamPtr) {}
        fn get_state(&self) -> PluginState {
            PluginState { version: String::new(), params: Default::default(), fields: Default::default() }
        }
        fn set_state(&self, _s: PluginState) {}
    }
    struct Tex {
        w: usize,
        h: usize,
        px: Vec<[f32; 4]>,
        repeat: bool,
    }
    fn apply_delta(texs: &mut HashMap<TextureId, Tex>, out: &egui::FullOutput) {
        // egui 0.37: several deltas per texture, and the font atlas is a plain colour image
        for (id, ds) in &out.textures_delta.set {
            for d in ds.iter() {
                let ImageData::Color(c) = &d.image;
                let (w, h) = (c.size[0], c.size[1]);
                let px: Vec<[f32; 4]> = c.pixels.iter().map(|p| [p.r() as f32 / 255.0, p.g() as f32 / 255.0, p.b() as f32 / 255.0, p.a() as f32 / 255.0]).collect();
                let repeat = d.options.wrap_mode == TextureWrapMode::Repeat;
                if let Some(pos) = d.pos {
                    let t = texs.get_mut(id).unwrap();
                    for y in 0..h {
                        for x in 0..w {
                            t.px[(pos[1] + y) * t.w + pos[0] + x] = px[y * w + x];
                        }
                    }
                } else {
                    texs.insert(*id, Tex { w, h, px, repeat });
                }
            }
        }
    }
    fn render(ctx: &egui::Context, out: egui::FullOutput, texs: &mut HashMap<TextureId, Tex>, path: &str) {
        let img = raster(ctx, out, texs);
        let bytes: Vec<u8> = img.iter().flat_map(|p| p.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0) as u8).collect::<Vec<_>>()).collect();
        std::fs::write(path, bytes).unwrap();
    }
    fn raster(ctx: &egui::Context, out: egui::FullOutput, texs: &mut HashMap<TextureId, Tex>) -> Vec<[f32; 3]> {
        apply_delta(texs, &out);
        let (w, h) = (WIDTH as usize, HEIGHT as usize);
        let mut img = vec![[0.0f32; 3]; w * h];
        let prims = ctx.tessellate(out.shapes, 1.0);
        for cp in prims {
            let epaint::Primitive::Mesh(mesh) = cp.primitive else { continue };
            let clip = cp.clip_rect;
            let tex = texs.get(&mesh.texture_id);
            for tri in mesh.indices.chunks(3) {
                let v = [mesh.vertices[tri[0] as usize], mesh.vertices[tri[1] as usize], mesh.vertices[tri[2] as usize]];
                let (x0, x1) = (v.iter().map(|q| q.pos.x).fold(f32::MAX, f32::min).max(clip.left()).max(0.0), v.iter().map(|q| q.pos.x).fold(f32::MIN, f32::max).min(clip.right()).min(w as f32));
                let (y0, y1) = (v.iter().map(|q| q.pos.y).fold(f32::MAX, f32::min).max(clip.top()).max(0.0), v.iter().map(|q| q.pos.y).fold(f32::MIN, f32::max).min(clip.bottom()).min(h as f32));
                let area = (v[1].pos.x - v[0].pos.x) * (v[2].pos.y - v[0].pos.y) - (v[2].pos.x - v[0].pos.x) * (v[1].pos.y - v[0].pos.y);
                if area.abs() < 1e-6 {
                    continue;
                }
                for py in (y0.floor() as usize)..(y1.ceil() as usize).min(h) {
                    for px in (x0.floor() as usize)..(x1.ceil() as usize).min(w) {
                        let (fx, fy) = (px as f32 + 0.5, py as f32 + 0.5);
                        let e = |a: &epaint::Vertex, b: &epaint::Vertex| (b.pos.x - a.pos.x) * (fy - a.pos.y) - (fx - a.pos.x) * (b.pos.y - a.pos.y);
                        let (w0, w1, w2) = (e(&v[1], &v[2]) / area, e(&v[2], &v[0]) / area, e(&v[0], &v[1]) / area);
                        if w0 < -1e-4 || w1 < -1e-4 || w2 < -1e-4 {
                            continue;
                        }
                        let mut c = [0.0f32; 4];
                        for i in 0..4 {
                            c[i] = (w0 * v[0].color.to_array()[i] as f32 + w1 * v[1].color.to_array()[i] as f32 + w2 * v[2].color.to_array()[i] as f32) / 255.0;
                        }
                        if let Some(t) = tex {
                            let mut u = w0 * v[0].uv.x + w1 * v[1].uv.x + w2 * v[2].uv.x;
                            let mut vv = w0 * v[0].uv.y + w1 * v[1].uv.y + w2 * v[2].uv.y;
                            if t.repeat {
                                u = u.rem_euclid(1.0);
                                vv = vv.rem_euclid(1.0);
                            }
                            let tx = ((u * t.w as f32) as usize).min(t.w - 1);
                            let ty = ((vv * t.h as f32) as usize).min(t.h - 1);
                            let s = t.px[ty * t.w + tx];
                            for i in 0..4 {
                                c[i] *= s[i];
                            }
                        }
                        let d = &mut img[py * w + px];
                        for i in 0..3 {
                            d[i] = c[i] + d[i] * (1.0 - c[3]);
                        }
                    }
                }
            }
        }
        img
    }
    fn raw(i: usize, ev: Vec<Event>) -> RawInput {
        RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(WIDTH as f32, HEIGHT as f32))), time: Some(i as f64 * 0.04), events: ev, ..Default::default() }
    }

    fn drums(n: usize) -> Vec<f32> {
        let mut s = 77u32;
        (0..n).map(|i| {
            s = s.wrapping_mul(1664525).wrapping_add(1013904223);
            let nz = (s >> 8) as f32 / 8388608.0 - 1.0;
            let t = (i % 24000) as f32 / 48000.0;
            let t2 = ((i + 12000) % 24000) as f32 / 48000.0;
            let kick = (-t * 12.0).exp() * (std::f32::consts::TAU * (50.0 + 80.0 * (-t * 30.0).exp()) * t).sin();
            let snare = if (i / 24000) % 2 == 1 { (-t2 * 18.0).exp() * nz * 0.6 } else { 0.0 };
            0.5 * (kick + snare) + 0.02 * nz
        }).collect()
    }

    #[test]
    fn draws_and_clicks() {
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        let p = HoistParams::default();
        let sh = Shared::default();
        let mock = Mock;
        let setter = ParamSetter::new(&mock);
        let mut st = new_state("HoistTest");
        let mut i = 0;
        for y in (50..466).step_by(17) {
            for x in (6..776).step_by(29) {
                let q = Pos2::new(x as f32, y as f32);
                let _ = ctx.run_ui(raw(i, vec![Event::PointerMoved(q), Event::PointerButton { pos: q, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }]), |c| frame(c, &setter, &mut st, &p, &sh));
                let _ = ctx.run_ui(raw(i + 1, vec![Event::PointerButton { pos: q, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }]), |c| frame(c, &setter, &mut st, &p, &sh));
                i += 2;
            }
        }
    }

    /// Pill text dead centre (rendered pixels).
    #[test]
    fn pill_text_is_centred() {
        let labels = ["LINK", "SAFE 0 dB", "SAFE OFF"];
        for on in [false, true] {
            let ctx = egui::Context::default();
            install_fonts(&ctx);
            let mut texs = HashMap::new();
            let rects: Vec<Rect> = (0..labels.len()).map(|i| Rect::from_min_size(pos2(30.0 + i as f32 * 150.0, 100.0), vec2(96.0, 20.0))).collect();
            let mut last = None;
            for f in 0..3 {
                let out = ctx.run_ui(raw(f, vec![]), |c| {
                    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(BG)).show(c, |ui| {
                        for (i, r) in rects.iter().enumerate() {
                            let _ = pill(ui, *r, labels[i], on, ("t", i));
                        }
                    });
                });
                if f < 2 {
                    apply_delta(&mut texs, &out);
                }
                last = Some(out);
            }
            let img = raster(&ctx, last.unwrap(), &mut texs);
            let w = WIDTH as usize;
            for (i, r) in rects.iter().enumerate() {
                let bg = img[(r.center().y as usize) * w + (r.left() + 12.0) as usize];
                let inner = r.shrink2(vec2(10.0, 3.0));
                let (mut x0, mut x1, mut y0, mut y1) = (usize::MAX, 0, usize::MAX, 0);
                for y in inner.top() as usize..inner.bottom() as usize {
                    for x in inner.left() as usize..inner.right() as usize {
                        let q = img[y * w + x];
                        if (q[0] - bg[0]).abs() + (q[1] - bg[1]).abs() + (q[2] - bg[2]).abs() > 0.45 {
                            x0 = x0.min(x);
                            x1 = x1.max(x);
                            y0 = y0.min(y);
                            y1 = y1.max(y);
                        }
                    }
                }
                assert!(x0 < x1, "{}: no text", labels[i]);
                let (dx, dy) = ((x0 + x1 + 1) as f32 * 0.5 - r.center().x, (y0 + y1 + 1) as f32 * 0.5 - r.center().y);
                assert!(dx.abs() <= 1.0 && dy.abs() <= 1.0, "{} off centre ({dx:+.1}, {dy:+.1})", labels[i]);
            }
        }
    }

    /// SHOT_DIR: the real engine on a drum loop. 0 = LOW 18 / HIGH 8, 1 = LOW 36 / HIGH 20 / OUT -6.
    #[test]
    fn screenshots() {
        let Ok(dir) = std::env::var("SHOT_DIR") else { return };
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        let mock = Mock;
        let setter = ParamSetter::new(&mock);
        let mut texs = HashMap::new();
        let dbp = |n: &str, v: f32, a: f32, b: f32| FloatParam::new(n, v, FloatRange::Linear { min: a, max: b });
        for (n, (lo, hi, out)) in [(18.0, 8.0, 0.0), (36.0, 20.0, -6.0)].iter().enumerate() {
            let mut p = HoistParams::default();
            p.low = dbp("Low Level", *lo, 0.0, crate::LOW_MAX);
            p.high = dbp("High Level", *hi, 0.0, crate::HIGH_MAX);
            p.out = dbp("Output", *out, -24.0, 12.0);
            let stt = State { low: *lo, high: *hi, out_db: *out, ..State::default() };
            let sh = Arc::new(Shared::default());
            let mut e = Engine::new(48000.0).with_shared(sh.clone());
            let x = drums(48000 + 9000);
            let (mut l, mut r) = (x.clone(), x.clone());
            for (a, b) in l.chunks_mut(256).zip(r.chunks_mut(256)) {
                e.process(a, b, &stt);
            }
            let mut st = new_state("HoistShot");
            let mut last = None;
            for f in 0..3 {
                last = Some(ctx.run_ui(raw(n * 10 + f, vec![]), |c| frame(c, &setter, &mut st, &p, &sh)));
                if f < 2 {
                    apply_delta(&mut texs, last.as_ref().unwrap());
                }
            }
            render(&ctx, last.unwrap(), &mut texs, &format!("{}/hoist{}.rgb", dir, n));
        }
    }
}
