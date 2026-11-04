//! Compares the decode of a raw `.dts` with ffmpeg's float (`f32le`) and bit-exact (`s32le`) decodes.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let data = std::fs::read(&a[1]).unwrap();
    let mut d = cfa4_dts::cfa4::Decoder::new();
    let mut o = d.push(&data).unwrap();
    let f = d.flush().unwrap();
    for (x, y) in o.channels.iter_mut().zip(f.channels) {
        x.extend(y);
    }
    println!("{} Hz {:?} lossless {} samples {}", o.sample_rate, o.speakers, o.lossless, o.channels[0].len());
    let ch = o.channels.len();
    for (path, scale, s32) in [(&a[2], 1.0f64, false), (&a[3], 1.0 / 2_147_483_648.0, true)] {
        let raw = std::fs::read(path).unwrap();
        let want: Vec<f64> = raw
            .chunks_exact(4)
            .map(|b| if s32 { i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64 * scale } else { f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64 })
            .collect();
        let n = want.len() / ch;
        let mut worst = 0f64;
        let mut snr = f64::INFINITY;
        for c in 0..ch {
            let (mut s, mut e) = (0f64, 0f64);
            for i in 0..n.min(o.channels[c].len()) {
                let (x, y) = (o.channels[c][i] as f64, want[i * ch + c]);
                s += y * y;
                e += (x - y) * (x - y);
                worst = worst.max((x - y).abs());
            }
            snr = snr.min(10.0 * (s / e.max(1e-300)).log10());
            print!("[{c}: {:.1}] ", 10.0 * (s / e.max(1e-300)).log10());
        }
        println!("{path}: n {n} worst abs {worst:e} ({:.2} lsb24) min snr {snr:.1} dB", worst * 8_388_608.0);
    }
}
