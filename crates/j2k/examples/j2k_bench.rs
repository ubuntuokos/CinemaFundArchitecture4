//! Decode time of JPEG 2000 files: `cargo run --release -p cfa4-j2k --example j2k_bench -- FILE…`.

fn main() {
    for path in std::env::args().skip(1) {
        let Ok(data) = std::fs::read(&path) else {
            eprintln!("{path}: unreadable");
            continue;
        };
        let runs = 10;
        let mut times = Vec::new();
        for _ in 0..runs {
            let start = std::time::Instant::now();
            if let Err(e) = cfa4_j2k::decode(&data) {
                eprintln!("{path}: {e}");
                break;
            }
            times.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        times.sort_by(f64::total_cmp);
        let median = times.get(times.len() / 2).copied().unwrap_or(0.0);
        println!("{path}: median {median:.1} ms, best {:.1} ms per decode", times.first().copied().unwrap_or(0.0));
    }
}
