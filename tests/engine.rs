use hoist::*;
const SR: f32 = 48000.0;
fn sine(n: usize, db: f32, f: f32) -> Vec<f32> {
    let a = 10f32.powf(db / 20.0);
    (0..n).map(|i| a * (std::f32::consts::TAU * f * i as f32 / SR).sin()).collect()
}
fn noise(n: usize, a: f32) -> Vec<f32> {
    let mut s = 4242u32;
    (0..n).map(|_| { s = s.wrapping_mul(1664525).wrapping_add(1013904223); a * ((s >> 8) as f32 / 8388608.0 - 1.0) }).collect()
}
fn rms(x: &[f32]) -> f32 { (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt() }
fn run2(st: &State, l: &[f32], r: &[f32]) -> (Vec<f32>, Vec<f32>) {
    let mut e = Engine::new(SR);
    let (mut a, mut b) = (l.to_vec(), r.to_vec());
    for (x, y) in a.chunks_mut(256).zip(b.chunks_mut(256)) { e.process(x, y, st); }
    (a, b)
}
fn gain_db(st: &State, x: &[f32]) -> f32 {
    let (y, _) = run2(st, x, x);
    20.0 * (rms(&y[x.len() / 2..]) / rms(&x[x.len() / 2..])).log10()
}

#[test]
fn all_at_zero_is_transparent() {
    let x = noise(48000, 0.5);
    let st = State { safe: false, ..State::default() };
    let (y, _) = run2(&st, &x, &x);
    for i in 0..x.len() { assert!((y[i] - x[i]).abs() < 1e-6, "sample {i}"); }
}

#[test]
fn low_level_lifts_quiet_and_leaves_loud() {
    let st = State { low: 24.0, safe: false, ..State::default() };
    let q = gain_db(&st, &sine(96000, -40.0, 220.0));
    let m = gain_db(&st, &sine(96000, -20.0, 220.0));
    let l = gain_db(&st, &sine(96000, -3.0, 220.0));
    println!("LOW 24: -40 dB -> {q:+.1}, -20 dB -> {m:+.1}, -3 dB -> {l:+.1}");
    assert!(q > 20.0 && q < 24.5);
    assert!(m > 5.0 && m < q);
    assert!(l.abs() < 0.5);
}

#[test]
fn silence_and_hiss_are_not_pulled_up() {
    let st = State { low: 48.0, safe: false, ..State::default() };
    let g = gain_db(&st, &noise(96000, 10f32.powf(-95.0 / 20.0)));
    println!("LOW 48 on -95 dB hiss: {g:+.1} dB");
    assert!(g < 3.0);
}

#[test]
fn high_level_holds_peaks_and_raises_the_rest() {
    let st = State { high: 12.0, safe: false, ..State::default() };
    let loud = sine(96000, -1.0, 220.0);
    let (y, _) = run2(&st, &loud, &loud);
    let pk = y[48000..].iter().fold(0f32, |a, v| a.max(v.abs()));
    let q = gain_db(&st, &sine(96000, -30.0, 220.0));
    println!("HIGH 12: -1 dB peak -> {:.2} dBFS, -30 dB -> {q:+.1}", 20.0 * pk.log10());
    assert!(20.0 * pk.log10() < 0.6);
    assert!(q > 9.0);
}

#[test]
fn safe_never_over_zero() {
    let st = State { low: 48.0, high: 48.0, out_db: 12.0, ..State::default() };
    let x: Vec<f32> = noise(96000, 0.9).iter().enumerate().map(|(i, v)| v * if (i / 3000) % 2 == 0 { 1.0 } else { 0.01 }).collect();
    let (y, z) = run2(&st, &x, &x);
    let pk = y.iter().chain(z.iter()).fold(0f32, |a, v| a.max(v.abs()));
    assert!(pk <= 1.0, "peak {pk}");
}

#[test]
fn mix_zero_and_bypass_are_dry() {
    let x = noise(48000, 0.4);
    for st in [State { low: 30.0, high: 10.0, mix: 0.0, safe: false, ..State::default() }, State { low: 30.0, high: 10.0, bypass: true, safe: false, ..State::default() }] {
        let (y, _) = run2(&st, &x, &x);
        for i in 4000..x.len() { assert!((y[i] - x[i]).abs() < 1e-5, "sample {i}"); }
    }
}

#[test]
fn unlinked_channels_stay_apart() {
    let l = sine(96000, -3.0, 220.0);
    let r = sine(96000, -40.0, 330.0);
    let st = State { low: 24.0, link: false, safe: false, ..State::default() };
    let (_, y) = run2(&st, &l, &r);
    let g = 20.0 * (rms(&y[48000..]) / rms(&r[48000..])).log10();
    let st = State { link: true, ..st };
    let (_, y1) = run2(&st, &l, &r);
    let g1 = 20.0 * (rms(&y1[48000..]) / rms(&r[48000..])).log10();
    println!("quiet right: unlinked {g:+.1} dB, linked {g1:+.1} dB");
    assert!(g > 20.0 && g1.abs() < 1.0);
}

#[test]
fn cpu() {
    let st = State { low: 24.0, high: 10.0, ..State::default() };
    let x = noise(48000 * 10, 0.3);
    let mut e = Engine::new(SR);
    let (mut a, mut b) = (x.clone(), x.clone());
    let t = std::time::Instant::now();
    for (p, q) in a.chunks_mut(256).zip(b.chunks_mut(256)) { e.process(p, q, &st); }
    let pct = t.elapsed().as_secs_f32() / 10.0 * 100.0;
    println!("hoist: {pct:.3}% of one core (stereo, 48 kHz)");
    assert!(pct < 1.0);
}

/// Sparse hits with a quiet floor between: lift eases up in the gap (no snap / pump),
/// and SPEED sets how fast. Returns the lift (dB) 60 ms and 400 ms into the gap.
fn gap_lift(speed: f32) -> (f32, f32) {
    let st = State { low: 36.0, speed, safe: false, ..State::default() };
    let mut e = Engine::new(SR);
    let n = 48000 * 3;
    let mut s = 5u32;
    let x: Vec<f32> = (0..n).map(|i| {
        s = s.wrapping_mul(1664525).wrapping_add(1013904223);
        let nz = (s >> 8) as f32 / 8388608.0 - 1.0;
        let t = (i % 48000) as f32 / SR;
        0.8 * (-t * 40.0).exp() * nz + 0.003 * nz
    }).collect();
    let (mut a, mut b) = (x.clone(), x.clone());
    let (mut at60, mut at400) = (0.0, 0.0);
    for (k, (p, q)) in a.chunks_mut(64).zip(b.chunks_mut(64)).enumerate() {
        e.process(p, q, &st);
        let t = (k * 64) % 48000;
        if k * 64 >= 96000 {
            if t == 7168 { at60 = get(&e.shared.up); }
            if t == 19200 { at400 = get(&e.shared.up); }
        }
    }
    (at60, at400)
}

#[test]
fn lift_eases_up_in_gaps_and_speed_sets_how_fast() {
    let (s60, s400) = gap_lift(0.0);
    let (m60, m400) = gap_lift(0.5);
    let (f60, f400) = gap_lift(1.0);
    println!("lift in gap (150 ms / 400 ms): smooth {s60:.1}/{s400:.1}, med {m60:.1}/{m400:.1}, fast {f60:.1}/{f400:.1}");
    // no snap: early in the gap the lift is well short of where it ends up
    assert!(s60 < s400 * 0.6 && m60 < m400 * 0.8);
    // SPEED orders the recovery
    assert!(s60 + 1.0 < m60 && m60 + 1.0 < f60 && s400 < m400 && m400 < f400);
    assert!(m400 > 18.0, "MED still squashes close to the old detector");
    assert!(f400 > 25.0, "FAST squashes hard");
}

/// Play after quiet: the first hit must not be boosted by the lift that was up during
/// the quiet part (that spike was the click on play / stop, and SAFE crushing it).
#[test]
fn no_spike_on_the_first_hit_after_quiet() {
    let mut s = 3u32;
    let mut x: Vec<f32> = (0..48000).map(|_| { s = s.wrapping_mul(1664525).wrapping_add(1013904223); 0.001 * ((s >> 8) as f32 / 8388608.0 - 1.0) }).collect();
    x.extend((0..24000).map(|i| { let t = i as f32 / SR; 0.7 * (-t * 12.0).exp() * (std::f32::consts::TAU * (50.0 + 80.0 * (-t * 30.0).exp()) * t).sin() }));
    for (lo, hi, out, sp) in [(18.0f32, 12.0f32, -4.0f32, 0.5f32), (30.0, 24.0, -8.0, 0.7), (48.0, 48.0, 0.0, 1.0)] {
        let st = State { low: lo, high: hi, out_db: out, speed: sp, safe: false, ..State::default() };
        let (y, _) = run2(&st, &x, &x);
        let on = y[48000..48000 + 96].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let body = y[48000 + 480..48000 + 4800].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        println!("LOW {lo} HIGH {hi}: onset {:.1} dBFS, body {:.1} dBFS", 20.0 * on.log10(), 20.0 * body.log10());
        assert!(on < body * 1.26, "onset spike {:.1} dB over the body", 20.0 * (on / body).log10());
    }
}

/// Two identical kicks after silence must come out the same: the first one used to be
/// squashed (lift spike + SAFE), the second clean and pokey.
#[test]
fn back_to_back_kicks_match() {
    let kick = |i: usize| { let t = i as f32 / SR; 0.7 * (-t * 12.0).exp() * (std::f32::consts::TAU * (50.0 + 80.0 * (-t * 30.0).exp()) * t).sin() };
    for gap in [12000usize, 24000] {
        for (lo, hi, out, sp) in [(18.0f32, 12.0f32, -4.0f32, 0.5f32), (30.0, 24.0, -8.0, 0.7), (18.0, 8.0, -6.0, 0.5)] {
            let mut x = vec![0.0f32; 48000];
            x.extend((0..gap).map(kick));
            x.extend((0..gap).map(kick));
            let st = State { low: lo, high: hi, out_db: out, speed: sp, ..State::default() };
            let (y, _) = run2(&st, &x, &x);
            let w = 7200; // first 150 ms of each hit
            let (a, b) = (&y[48000..48000 + w], &y[48000 + gap..48000 + gap + w]);
            let pk = |s: &[f32]| s.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            let (dp, dr) = (20.0 * (pk(b) / pk(a)).log10(), 20.0 * (rms(b) / rms(a)).log10());
            let diff = a.iter().zip(b).map(|(p, q)| (p - q) * (p - q)).sum::<f32>().sqrt() / a.iter().map(|p| p * p).sum::<f32>().sqrt();
            println!("gap {} ms, LOW {lo} HIGH {hi}: 2nd vs 1st peak {dp:+.2} dB, RMS {dr:+.2} dB, waveform diff {:.1}%", gap / 48, diff * 100.0);
            assert!(dp.abs() < 0.5 && dr.abs() < 0.5 && diff < 0.08);
        }
    }
}
