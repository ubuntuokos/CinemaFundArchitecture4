fn main() {
    let p = std::env::args().nth(1).unwrap_or_default();
    let mut r = rsmedia::StreamReader::new(p.as_str()).unwrap();
    let mut d = rsmedia::DecoderBuilder::new(rsmedia::MediaType::AUDIO)
        .with_sample_fmt(rsmedia::SampleFormat::FLTP)
        .with_filters(vec![rsmedia::filter::audio::resample(2, 48000, rsmedia::SampleFormat::FLTP)])
        .build_from_reader(&r).unwrap();
    println!("decoder tb {:?}", d.time_base());
    for _ in 0..3 { if let Some(f) = d.decode::<f32>(&mut r).unwrap() { println!("pts {} be {} tb {}/{} n {}", f.pts, f.best_effort_timestamp, f.time_base.num, f.time_base.den, f.nb_samples); } }
}
