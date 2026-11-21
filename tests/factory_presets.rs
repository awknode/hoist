//! Factory presets: validated against the real params, played (bounded), written with PRESET_OUT.
use nih_plug::prelude::*;
use serde_json::{json, Map, Value};
use hoist::*;

type Preset = (&'static str, &'static [(&'static str, f64)]);
const PRESETS: &[Preset] = &[
    // the two Stacks reached for on MV2 (names only; values are HOIST's own)
    ("Squeeze", &[("speed", 0.5), ("low", 18.0), ("high", 12.0), ("out", -4.0)]),
    ("Squash", &[("speed", 0.7), ("low", 30.0), ("high", 24.0), ("out", -8.0)]),
    ("Drums - Fatter (Parallel)", &[("speed", 0.5), ("low", 18.0), ("high", 8.0), ("out", -6.0)]),
    ("Drums - Heavy Smash (Parallel)", &[("speed", 0.75), ("low", 30.0), ("high", 16.0), ("out", -10.0)]),
    ("Drums - Room Explode", &[("speed", 0.85), ("low", 40.0), ("high", 20.0), ("out", -14.0)]),
    ("Drums - Subtle Thicken", &[("speed", 0.35), ("low", 10.0), ("high", 4.0), ("out", -4.0)]),
    ("Kick + Snare Weight", &[("speed", 0.6), ("low", 14.0), ("high", 12.0), ("out", -8.0)]),
    ("Bass - Even Out", &[("speed", 0.25), ("low", 12.0), ("high", 6.0), ("out", -2.0)]),
    ("Vocal - Up Front", &[("speed", 0.35), ("low", 16.0), ("high", 6.0), ("out", -4.0)]),
    ("Bus - Loud Glue", &[("speed", 0.3), ("low", 6.0), ("high", 10.0), ("out", -3.0)]),
    ("Upward Only - Detail", &[("speed", 0.4), ("low", 20.0), ("high", 0.0), ("out", -3.0)]),
    ("Downward Only - Dense", &[("speed", 0.5), ("high", 14.0), ("out", -6.0)]),
    ("FX Return - Bloom", &[("speed", 0.2), ("low", 26.0), ("high", 12.0), ("out", -9.0)]),
    ("Unlinked - Wide Room", &[("speed", 0.6), ("low", 22.0), ("high", 10.0), ("out", -8.0), ("link", 0.0)]),
];

fn build(params: &HoistParams, over: &[(&str, f64)]) -> Value {
    let map = params.param_map();
    let mut out = Map::new();
    for (id, v) in over {
        let ptr = map.iter().find(|(i, _, _)| i == id).unwrap_or_else(|| panic!("no param {id}")).1;
        let val = unsafe {
            match ptr {
                ParamPtr::FloatParam(p) => {
                    let p = &*p;
                    assert!((p.preview_plain(p.preview_normalized(*v as f32)) - *v as f32).abs() < 1e-3, "{id} out of range");
                    json!({ "f32": *v as f32 })
                }
                ParamPtr::IntParam(p) => {
                    let p = &*p;
                    assert_eq!(p.preview_plain(p.preview_normalized(*v as i32)), *v as i32, "{id} out of range");
                    json!({ "i32": *v as i32 })
                }
                ParamPtr::BoolParam(_) => json!({ "bool": *v != 0.0 }),
                _ => panic!("{id}"),
            }
        };
        out.insert(id.to_string(), val);
    }
    json!({ "version": env!("CARGO_PKG_VERSION"), "params": out, "fields": {} })
}

#[test]
fn factory_presets_are_valid() {
    let params = HoistParams::default();
    let dir = std::env::var("PRESET_OUT").ok();
    for p in PRESETS {
        assert!(!p.0.contains('/'));
        let v = build(&params, p.1);
        let _: nih_plug::wrapper::state::PluginState = serde_json::from_value(v.clone()).unwrap();
        let mut st = State::default();
        for (id, x) in p.1 {
            match *id {
                "low" => st.low = *x as f32,
                "high" => st.high = *x as f32,
                "out" => st.out_db = *x as f32,
                "link" => st.link = *x != 0.0,
                "speed" => st.speed = *x as f32,
                k => panic!("{k}"),
            }
        }
        let mut e = Engine::new(48000.0);
        let mut s = 9u32;
        let mut l: Vec<f32> = (0..48000).map(|i| { s = s.wrapping_mul(1664525).wrapping_add(1013904223); 0.6 * ((s >> 8) as f32 / 8388608.0 - 1.0) * if (i / 5000) % 2 == 0 { 1.0 } else { 0.05 } }).collect();
        let mut r = l.clone();
        for (a, b) in l.chunks_mut(512).zip(r.chunks_mut(512)) { e.process(a, b, &st); }
        let pk = l.iter().chain(r.iter()).fold(0.0f32, |a, v| a.max(v.abs()));
        assert!(pk.is_finite() && pk <= 1.0, "{}: {pk}", p.0);
        if let Some(d) = &dir {
            std::fs::create_dir_all(d).unwrap();
            std::fs::write(format!("{d}/{}.awkp", p.0), serde_json::to_string_pretty(&v).unwrap()).unwrap();
        }
    }
}
