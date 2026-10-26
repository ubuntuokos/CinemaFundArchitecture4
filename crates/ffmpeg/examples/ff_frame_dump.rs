use cfa4_frame::PixelData;
fn main() {
    let mut a = std::env::args().skip(1);
    let p = a.next().unwrap_or_default();
    let n: i64 = a.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let fps: i64 = a.next().and_then(|s| s.parse().ok()).unwrap_or(25);
    let src = cfa4_ffmpeg::open(std::path::Path::new(&p)).unwrap();
    let t = cfa4_time::Tick(n * cfa4_time::TICKS_PER_SECOND / fps + cfa4_time::TICKS_PER_SECOND / fps / 2);
    let f = src.video_frame(cfa4_media::FrameRequest::full(t)).unwrap();
    eprintln!("{} {:?} pts {:?} info {:?}", f.format_label(), (f.width, f.height), f.pts, src.info().video.as_ref().map(|v| (v.frame_rate, v.pixel_format.clone())));
    let b: Vec<u8> = match &f.data { PixelData::Yuv8 { planes, .. } => planes.iter().flat_map(|p| p.iter().copied()).collect(), PixelData::Yuv16 { planes, .. } => planes.iter().flat_map(|p| p.iter().flat_map(|s| s.to_le_bytes())).collect(), PixelData::Rgba8(d) => d.to_vec(), _ => vec![] };
    std::io::Write::write_all(&mut std::io::stdout(), &b).unwrap();
}
