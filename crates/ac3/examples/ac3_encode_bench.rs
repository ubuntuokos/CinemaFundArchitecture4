//! Encode speed: `cargo run --release -p cfa4-ac3 --example ac3_encode_bench` encodes 60 s of
//! synthetic 5.1 at 448 kb/s (AC-3) and 640 kb/s (E-AC-3).

fn main() {
    let rate = 48_000u32;
    let n = rate as usize * 60;
    let pcm: Vec<Vec<f32>> =
        (0..6).map(|c| (0..n).map(|i| 0.3 * (2.0 * std::f32::consts::PI * (200.0 + 150.0 * c as f32) * i as f32 / rate as f32).sin()).collect()).collect();
    let refs: Vec<&[f32]> = pcm.iter().map(Vec::as_slice).collect();
    for eac3 in [false, true] {
        let enc = if eac3 { cfa4_ac3::Encoder::new_eac3(rate, 6, 640) } else { cfa4_ac3::Encoder::new(rate, 6, 448) };
        let Ok(mut enc) = enc else { return };
        let start = std::time::Instant::now();
        let frames = enc.encode(&refs).map(|f| f.len()).unwrap_or(0);
        let secs = start.elapsed().as_secs_f64();
        println!("{}: {frames} syncframes for 60 s of 5.1 in {secs:.2} s ({:.0}× real time)", if eac3 { "E-AC-3" } else { "AC-3" }, 60.0 / secs);
    }
}
