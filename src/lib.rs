//! HOIST — awk studios. Two-stage level compressor for pushing things louder,
//! built to sit on a duplicate (parallel) track:
//!
//!   LOW LEVEL  upward compression: quiet parts are lifted (up to +48 dB), loud parts left alone.
//!   HIGH LEVEL downward compression with automatic makeup: peaks are held, everything gets louder.
//!   OUTPUT, LINK (one gain for both channels), MIX, SAFE (never over 0 dBFS).
//!
//!   SPEED      how fast the lift comes back after a hit (SMOOTH .. FAST).
//!
//! Zero latency. Cheapest possible path: per sample one abs/max and an interpolated
//! gain multiply; the detector and the gain law (log / exp) run every CR samples.
//!
//! Detector: peak with a short HOLD, then a release; plus a slow "memory" of recent
//! peaks the detector can't fall too far below. So in the gaps of sparse material the
//! lift eases up instead of snapping up (no pumping room / hiss), while dense material
//! still gets the full squash.

use nice_plug::prelude::*;
use nice_plug_egui::{EguiEditor, EguiEditorState, RepaintNotifier};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

mod awk_ui;
mod editor;
mod presets;

/// Control rate: the gain law runs every CR samples, gain is interpolated between.
const CR: usize = 16;
pub const LOW_MAX: f32 = 48.0;
pub const HIGH_MAX: f32 = 48.0;
/// LOW LEVEL (0.3.0, fitted to Waves MV2, awk-bench 2026-11-22): the lift threshold rises with the knob
/// (UP_T0 + LOW dBFS), slope 0.85 dB per dB below it, capped by LOW through a 48 dB soft knee, and the
/// lift fades out softly (24 dB) instead of stopping dead. So the whole knob works: LOW 48 still lifts
/// a 0 dBFS signal by ~17 dB, like MV2 (0.2.x stopped above -10 dBFS and topped out near +20 dB).
const UP_T0: f32 = -24.0;
const UP_SLOPE: f32 = 0.85;
const UP_KNEE: f32 = 48.0;
const UP_FADE: f32 = 24.0;
/// SPEED 50 % lands on MV2's recovery (~350-550 ms); 0.2.x was ~1.6x quicker.
const TIME_SCALE: f32 = 1.6;
/// Below FLOOR the lift fades out over FLOOR_W dB, so silence / dither / hiss isn't pulled up.
const FLOOR: f32 = -70.0;
const FLOOR_W: f32 = 20.0;
/// Detector hold after a peak (s).
const HOLD: f32 = 0.010;
/// HIGH LEVEL: 10:1 above a threshold at -HIGH, 6 dB knee, makeup = the reduction a 0 dBFS peak gets.
/// It fades in over the first 3 dB of travel, so HIGH 0 is truly off (no knee, no makeup).
const DOWN_SLOPE: f32 = 0.9;
const KNEE: f32 = 12.0;

pub struct Shared {
    pub meters: awk_ui::Meters,
    /// lift (dB, >= 0), reduction (dB, >= 0), net gain incl. makeup + output (dB)
    pub up: AtomicU32,
    pub down: AtomicU32,
    pub net: AtomicU32,
}
impl Default for Shared {
    fn default() -> Self {
        Self { meters: awk_ui::Meters::default(), up: AtomicU32::new(0), down: AtomicU32::new(0), net: AtomicU32::new(0) }
    }
}
pub fn get(a: &AtomicU32) -> f32 {
    f32::from_bits(a.load(Ordering::Relaxed))
}
fn put(a: &AtomicU32, v: f32) {
    a.store(v.to_bits(), Ordering::Relaxed);
}

#[derive(Params)]
pub struct HoistParams {
    #[id = "low"]
    pub low: FloatParam,
    #[id = "high"]
    pub high: FloatParam,
    #[id = "out"]
    pub out: FloatParam,
    #[id = "link"]
    pub link: BoolParam,
    #[id = "speed"]
    pub speed: FloatParam,
    #[id = "gmix"]
    pub mix: FloatParam,
    #[id = "safe"]
    pub safe: BoolParam,
    #[id = "bypass"]
    pub bypass: BoolParam,
    /// Window size (1.0 = 100%), saved with the project. 0 = not set: open at the default.
    #[persist = "ui-scale"]
    pub ui_scale: Arc<std::sync::RwLock<f32>>,
}
fn db_param(name: &str, def: f32, min: f32, max: f32) -> FloatParam {
    FloatParam::new(name, def, FloatRange::Linear { min, max }).with_smoother(SmoothingStyle::Linear(30.0)).with_unit(" dB").with_value_to_string(formatters::v2s_f32_rounded(1))
}
impl Default for HoistParams {
    fn default() -> Self {
        Self {
            low: db_param("Low Level", 0.0, 0.0, LOW_MAX),
            high: db_param("High Level", 0.0, 0.0, HIGH_MAX),
            out: db_param("Output", 0.0, -24.0, 12.0),
            link: BoolParam::new("Link", true),
            speed: FloatParam::new("Speed", 0.5, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(30.0))
                .with_value_to_string(formatters::v2s_f32_percentage(0))
                .with_string_to_value(formatters::s2v_f32_percentage())
                .with_unit("%"),
            mix: awk_ui::mix_param(),
            safe: awk_ui::safe_param(),
            bypass: awk_ui::bypass_param(),
            ui_scale: Arc::new(std::sync::RwLock::new(0.0)),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct State {
    pub low: f32,
    pub high: f32,
    pub out_db: f32,
    pub link: bool,
    /// 0 = smooth (slow lift recovery, long memory) .. 1 = fast
    pub speed: f32,
    pub mix: f32,
    pub safe: bool,
    pub bypass: bool,
}
impl Default for State {
    fn default() -> Self {
        Self { low: 0.0, high: 0.0, out_db: 0.0, link: true, speed: 0.5, mix: 1.0, safe: true, bypass: false }
    }
}

/// The gain law: detector level (dBFS) -> (lift dB, reduction dB). Pure, so the UI can draw it.
pub fn law(level: f32, low: f32, high: f32) -> (f32, f32) {
    let (up, down) = law_raw(level, low, high);
    (up * floor_k(level), down)
}
/// How much of the lift survives at this level: none in silence / hiss (below FLOOR - FLOOR_W).
pub fn floor_k(level: f32) -> f32 {
    ((level - (FLOOR - FLOOR_W)) / FLOOR_W).clamp(0.0, 1.0)
}
/// The law before the hiss floor. The engine keeps its lift state on this, and applies
/// floor_k at the output, so after silence the lift is already where the next hit needs it
/// (the first of two identical kicks used to come out ~2 dB quieter than the second).
pub fn law_raw(level: f32, low: f32, high: f32) -> (f32, f32) {
    let x = smin(low, UP_SLOPE * (UP_T0 + low - level), UP_KNEE);
    // soft floor at 0 dB; fades in over the first 3 dB of LOW so LOW 0 is truly off
    let up = (-smin(-x, 0.0, UP_FADE)).clamp(0.0, low) * (low / 3.0).clamp(0.0, 1.0);
    let over = level + up + high;
    let down = if 2.0 * over < -KNEE {
        0.0
    } else if 2.0 * over <= KNEE {
        DOWN_SLOPE * (over + KNEE * 0.5).powi(2) / (2.0 * KNEE)
    } else {
        DOWN_SLOPE * over
    };
    (up, down * (high / 3.0).clamp(0.0, 1.0))
}
/// smooth minimum of a and b with a quadratic knee w dB wide
fn smin(a: f32, b: f32, w: f32) -> f32 {
    let d = a - b;
    if d > 0.5 * w {
        b
    } else if d < -0.5 * w {
        a
    } else {
        0.5 * (a + b) - (d * d / (2.0 * w) + w / 8.0)
    }
}
/// HIGH LEVEL makeup: what a 0 dBFS peak loses, given back.
pub fn makeup(high: f32) -> f32 {
    law(0.0, 0.0, high).1
}

/// Times (s) for a SPEED setting: detector release, memory release, memory depth (dB),
/// lift recovery, reduction release. Log-interpolated SMOOTH -> FAST.
pub fn speed_times(s: f32) -> (f32, f32, f32, f32, f32) {
    let s = s.clamp(0.0, 1.0);
    let li = |a: f32, b: f32| a * (b / a).powf(s);
    let k = TIME_SCALE;
    (k * li(0.120, 0.030), k * li(0.6, 0.10), 14.0 + 18.0 * s, k * li(0.25, 0.03), k * li(0.25, 0.04))
}

pub struct Engine {
    sr: f32,
    /// block peak since the last control step, per channel
    pk: [f32; 2],
    /// detector envelope, hold counter (control steps) and slow memory, per channel
    env: [f32; 2],
    hold: [u32; 2],
    mem: [f32; 2],
    hold_n: u32,
    /// coefficients for the current SPEED (cached)
    speed_at: f32,
    env_rel: f32,
    mem_rel: f32,
    mem_k: f32,
    up_rise: f32,
    down_rel: f32,
    /// smoothed lift / reduction per channel (dB)
    up: [f32; 2],
    down: [f32; 2],
    /// hiss-floor factor per channel (lift shown / applied = up * fk)
    fk: [f32; 2],
    /// gain now and per-sample step (linear), per channel
    g: [f32; 2],
    dg: [f32; 2],
    n: usize,
    /// control-rate smoothing: gain going down fast, coming back slowly
    fast: f32,
    slow: f32,
    mix: f32,
    byp: awk_ui::BypassRamp,
    ceil: awk_ui::Ceiling,
    meter: awk_ui::PeakFollower,
    pub shared: Arc<Shared>,
}

impl Engine {
    pub fn new(sr: f32) -> Self {
        let cr = CR as f32 / sr;
        let mut e = Self {
            sr,
            pk: [0.0; 2],
            env: [0.0; 2],
            hold: [0; 2],
            mem: [0.0; 2],
            hold_n: (HOLD * sr / CR as f32).round().max(1.0) as u32,
            speed_at: -1.0,
            env_rel: 0.0,
            mem_rel: 0.0,
            mem_k: 0.0,
            up_rise: 0.0,
            down_rel: 0.0,
            fk: [1.0; 2],
            up: [0.0; 2],
            down: [0.0; 2],
            g: [1.0; 2],
            dg: [0.0; 2],
            n: 0,
            fast: (-cr / 0.0015).exp(),
            slow: 0.0,
            mix: -1.0,
            byp: awk_ui::BypassRamp::default(),
            ceil: awk_ui::Ceiling::default(),
            meter: awk_ui::PeakFollower::default(),
            shared: Arc::new(Shared::default()),
        };
        e.byp.set_sr(sr);
        e.ceil.set_sr(sr);
        e.meter.set_sr(sr);
        e
    }
    pub fn with_shared(mut self, s: Arc<Shared>) -> Self {
        self.shared = s;
        self
    }
    pub fn reset(&mut self) {
        let sr = self.sr;
        let s = self.shared.clone();
        *self = Self::new(sr).with_shared(s);
    }

    /// Runs the gain law for the next CR samples and sets up the interpolation.
    /// `snap`: a new peak just arrived: the detector, lift and reduction jump to it and
    /// the gain lands on this sample (no 16-sample ramp), so onsets after quiet passages
    /// aren't boosted by the lift that was up a moment ago.
    #[inline]
    fn control(&mut self, st: &State, snap: bool) {
        if (st.speed - self.speed_at).abs() > 1e-4 {
            let (det, mem, depth, rise, rel) = speed_times(st.speed);
            let cr = CR as f32 / self.sr;
            self.env_rel = (-cr / det).exp();
            self.mem_rel = (-cr / mem).exp();
            self.mem_k = 10f32.powf(-depth / 20.0);
            self.up_rise = (-cr / rise).exp();
            self.down_rel = (-cr / rel).exp();
            self.slow = self.up_rise;
            self.speed_at = st.speed;
        }
        // detector: peak, HOLD, release; never far below the slow memory of recent peaks
        for ch in 0..2 {
            let p = self.pk[ch];
            self.pk[ch] = 0.0;
            if p >= self.env[ch] {
                self.env[ch] = p;
                self.hold[ch] = self.hold_n;
            } else if self.hold[ch] > 0 {
                self.hold[ch] -= 1;
            } else {
                self.env[ch] = p + self.env_rel * (self.env[ch] - p);
            }
            self.mem[ch] = if self.env[ch] > self.mem[ch] { self.env[ch] } else { self.mem[ch] * self.mem_rel };
        }
        let lvl = |e: f32| 20.0 * e.max(1e-6).log10();
        let (l0, l1) = (lvl(self.env[0].max(self.mem[0] * self.mem_k)), lvl(self.env[1].max(self.mem[1] * self.mem_k)));
        let levels = if st.link { [l0.max(l1); 2] } else { [l0, l1] };
        let mk = makeup(st.high) + st.out_db;
        let chans = if st.link { 1 } else { 2 };
        for ch in 0..chans {
            let (up_t, down_t) = law_raw(levels[ch], st.low, st.high);
            let fk = floor_k(levels[ch]);
            self.fk[ch] = fk;
            // net gain falling (lift dropping or reduction growing): fast; rising: slow
            let c = if up_t < self.up[ch] { if snap { 0.0 } else { self.fast } } else { self.up_rise };
            self.up[ch] = up_t + c * (self.up[ch] - up_t);
            let c = if down_t > self.down[ch] { if snap { 0.0 } else { self.fast } } else { self.down_rel };
            self.down[ch] = down_t + c * (self.down[ch] - down_t);
            let target = 10f32.powf((self.up[ch] * fk - self.down[ch] + mk) * 0.05);
            if snap && target < self.g[ch] {
                self.g[ch] = target;
                self.dg[ch] = 0.0;
            } else {
                self.dg[ch] = (target - self.g[ch]) / CR as f32;
            }
        }
        if st.link {
            self.fk[1] = self.fk[0];
            self.up[1] = self.up[0];
            self.down[1] = self.down[0];
            self.g[1] = self.g[0];
            self.dg[1] = self.dg[0];
        }
    }

    pub fn process(&mut self, l: &mut [f32], r: &mut [f32], st: &State) {
        if self.mix < 0.0 {
            self.mix = st.mix;
        }
        let a = 1.0 - (-1.0 / (0.02 * self.sr)).exp();
        for i in 0..l.len() {
            let (x0, x1) = (l[i], r[i]);
            // block peak (the detector runs at control rate)
            self.pk[0] = self.pk[0].max(x0.abs());
            self.pk[1] = self.pk[1].max(x1.abs());
            // a new peak more than 1 dB over the detector: act on this very sample
            let snap = x0.abs() > self.env[0] * 1.122 || x1.abs() > self.env[1] * 1.122;
            if self.n == 0 || snap {
                self.control(st, snap);
                self.n = 0;
            }
            self.n = (self.n + 1) % CR;
            self.g[0] += self.dg[0];
            self.g[1] += self.dg[1];
            self.mix += a * (st.mix - self.mix);
            let (y0, y1) = (x0 + (x0 * self.g[0] - x0) * self.mix, x1 + (x1 * self.g[1] - x1) * self.mix);
            let w = self.byp.next(st.bypass);
            let (y0, y1) = (x0 + (y0 - x0) * w, x1 + (y1 - x1) * w);
            let (y0, y1) = self.ceil.tick(y0, y1, st.safe);
            self.meter.feed(x0, x1, y0, y1);
            l[i] = y0;
            r[i] = y1;
        }
        let u = [self.up[0] * self.fk[0], self.up[1] * self.fk[1]];
        let ch = if u[0] - self.down[0] >= u[1] - self.down[1] { 0 } else { 1 };
        put(&self.shared.up, u[ch]);
        put(&self.shared.down, self.down[ch]);
        put(&self.shared.net, u[ch] - self.down[ch] + makeup(st.high) + st.out_db);
        self.meter.store(&self.shared.meters);
        self.ceil.store(&self.shared.meters);
    }
}

pub struct Hoist {
    params: Arc<HoistParams>,
    shared: Arc<Shared>,
    eng: Engine,
    /// window size + open flag (nice-plug keeps it on the plugin, not in the params)
    editor_state: Arc<EguiEditorState>,
    /// realtime-safe "please repaint" from the audio thread (nice-plug-egui no longer repaints by itself)
    repaint: RepaintNotifier,
}
impl Default for Hoist {
    fn default() -> Self {
        let shared = Arc::new(Shared::default());
        Self { params: Arc::new(HoistParams::default()), shared: shared.clone(), eng: Engine::new(48000.0).with_shared(shared), editor_state: EguiEditorState::from_size(nice_plug::editor::dpi::LogicalSize::new(editor::WIDTH as f64, editor::HEIGHT as f64), 1.0), repaint: RepaintNotifier::new() }
    }
}

impl Plugin for Hoist {
    const NAME: &'static str = "Hoist";
    const VENDOR: &'static str = "awk studios";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout { main_input_channels: NonZeroU32::new(2), main_output_channels: NonZeroU32::new(2), ..AudioIOLayout::const_default() }];
    const MIDI_INPUT: MidiConfig = MidiConfig::None;
    const SAMPLE_ACCURATE_AUTOMATION: bool = false;
    type Editor = EguiEditor<editor::HoistEditor>;
    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }
    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Self::Editor> {
        editor::create(self.editor_state.clone(), self.repaint.clone(), self.params.clone(), self.shared.clone())
    }
    fn activate(&mut self, _layout: &AudioIOLayout, cfg: &BufferConfig, _ctx: &mut impl ActivateContext<Self>) -> bool {
        self.eng = Engine::new(cfg.sample_rate).with_shared(self.shared.clone());
        true
    }
    fn reset(&mut self) {
        self.eng.reset();
    }
    fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _ctx: &mut impl ProcessContext<Self>) -> ProcessStatus {
        let p = &self.params;
        let st = State {
            low: p.low.smoothed.next(),
            high: p.high.smoothed.next(),
            out_db: p.out.smoothed.next(),
            link: p.link.value(),
            speed: p.speed.smoothed.next(),
            mix: p.mix.value(),
            safe: p.safe.value(),
            bypass: p.bypass.value(),
        };
        let sl = buffer.as_slice();
        if sl.len() < 2 {
            return ProcessStatus::Normal;
        }
        let n = sl[0].len().saturating_sub(1) as u32;
        let (a, b) = sl.split_at_mut(1);
        self.eng.process(&mut *a[0], &mut *b[0], &st);
        if n > 0 {
            p.low.smoothed.next_step(n);
            p.high.smoothed.next_step(n);
            p.out.smoothed.next_step(n);
            p.speed.smoothed.next_step(n);
        }
        // meters moved: ask the open window to repaint (realtime-safe, coalesced)
        if self.editor_state.is_open() {
            self.repaint.request_repaint();
        }
        ProcessStatus::Normal
    }
}

impl ClapPlugin for Hoist {
    const CLAP_ID: &'static str = "com.awknode.hoist";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Upward + downward level compressor");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::AudioEffect, ClapFeature::Compressor, ClapFeature::Stereo];
}
impl Vst3Plugin for Hoist {
    const VST3_CLASS_ID: [u8; 16] = *b"AwknodeHoistUp01";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Fx, Vst3SubCategory::Dynamics];
}

nice_export_clap!(Hoist);
nice_export_vst3!(Hoist);
