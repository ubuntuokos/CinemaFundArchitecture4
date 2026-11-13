//! LTC through cpal: an input stream that reads LTC and an output stream that writes it.
//!
//! Each stream lives on its own thread (cpal streams are not `Send` on every backend), like the
//! voice-over input in the app. Opening reports its result back before the thread parks.

use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};

use crate::ltc::{LtcGenerator, LtcReader, pick_channel};
use crate::manager::{Device, LtcInputConfig, LtcOutputConfig};
use crate::{Clock, Input, SharedTransport, Transport, duration_ticks, lock, samples_ticks};

fn host(name: &str) -> cpal::Host {
    if !name.is_empty()
        && let Some(id) = cpal::available_hosts().into_iter().find(|h| h.name() == name)
        && let Ok(h) = cpal::host_from_id(id)
    {
        return h;
    }
    cpal::default_host()
}

fn find(devices: Result<impl Iterator<Item = cpal::Device>, cpal::DevicesError>, name: &str) -> Option<cpal::Device> {
    devices.ok()?.find(|d| d.name().is_ok_and(|n| n == name))
}

type ErrorCell = Arc<Mutex<Option<String>>>;

fn report(cell: &ErrorCell, message: String) {
    let mut e = lock(cell);
    if e.is_none() {
        *e = Some(message);
    }
}

/// A stream kept alive on its own thread until dropped.
struct StreamThread {
    stop: Option<Sender<()>>,
    worker: Option<std::thread::JoinHandle<()>>,
    error: ErrorCell,
}

impl Device for StreamThread {
    fn error(&self) -> Option<String> {
        lock(&self.error).clone()
    }
}

impl Drop for StreamThread {
    fn drop(&mut self) {
        self.stop.take();
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

/// Runs `open` on a new thread that keeps the stream until the device is dropped.
fn spawn(name: &str, error: ErrorCell, open: impl FnOnce() -> Result<cpal::Stream, String> + Send + 'static) -> Result<Box<dyn Device>, String> {
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let worker = std::thread::Builder::new()
        .name(name.into())
        .spawn(move || match std::panic::catch_unwind(std::panic::AssertUnwindSafe(open)).unwrap_or_else(|_| Err("the audio stream thread panicked".into())) {
            Ok(stream) => {
                let _ = ready_tx.send(Ok(()));
                let _ = stop_rx.recv(); // returns when the device is dropped
                drop(stream);
            }
            Err(e) => {
                let _ = ready_tx.send(Err(e));
            }
        })
        .map_err(|e| e.to_string())?;
    let dev = StreamThread { stop: Some(stop_tx), worker: Some(worker), error };
    ready_rx.recv().map_err(|e| e.to_string()).and_then(|r| r)?;
    Ok(Box::new(dev))
}

fn check_channel(channel: u16, channels: u16, device: &str) -> Result<(), String> {
    if channel >= channels {
        return Err(format!("{device} has {channels} channels; LTC channel {} does not exist", channel + 1));
    }
    Ok(())
}

fn input_stream<T: SizedSample>(
    dev: &cpal::Device,
    config: &cpal::StreamConfig,
    mut reader: LtcReader,
    channel: u16,
    clock: Clock,
    tx: Sender<Input>,
    error: ErrorCell,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    f32: FromSample<T>,
{
    let channels = usize::from(config.channels);
    let sr = config.sample_rate.0;
    let (mut all, mut mono, mut obs) = (Vec::new(), Vec::new(), Vec::new());
    let stream_error = error.clone();
    let data_tx = tx.clone();
    dev.build_input_stream(
        config,
        move |data: &[T], info: &cpal::InputCallbackInfo| {
            let now = clock.now();
            all.clear();
            all.extend(data.iter().map(|s| s.to_sample::<f32>()));
            pick_channel(&all, channels, usize::from(channel), &mut mono);
            // the block's first sample was captured `lag` before now
            let ts = info.timestamp();
            let lag = ts.callback.duration_since(&ts.capture).map(duration_ticks).unwrap_or_default();
            let end = cfa4_time::Tick(now.0 - lag.0 + samples_ticks(mono.len() as u64, sr).0);
            obs.clear();
            reader.push(&mono, end, &mut obs);
            for o in &obs {
                let _ = data_tx.send(Input::Frame(*o));
            }
        },
        move |e| {
            let msg = format!("LTC input: {e}");
            report(&stream_error, msg.clone());
            let _ = tx.send(Input::Error(msg));
        },
        None,
    )
}

/// Opens the LTC input; frames go to `tx`.
pub fn open_ltc_input(cfg: &LtcInputConfig, tx: Sender<Input>) -> Result<Box<dyn Device>, String> {
    let cfg = cfg.clone();
    let error = ErrorCell::default();
    let stream_error = error.clone();
    spawn("timecode LTC in", error, move || {
        let h = host(&cfg.host);
        let dev = if cfg.device.is_empty() { h.default_input_device() } else { find(h.input_devices(), &cfg.device) };
        let label = if cfg.device.is_empty() { "the default input".to_string() } else { format!("input `{}`", cfg.device) };
        let dev = dev.ok_or_else(|| format!("LTC input: no {label}"))?;
        let sc = dev.default_input_config().map_err(|e| format!("LTC input: {e}"))?;
        check_channel(cfg.channel, sc.channels(), &label)?;
        let sr = sc.sample_rate().0;
        let reader = LtcReader::new(sr, cfg.rate).map_err(|e| format!("LTC input: {e}"))?;
        let format = sc.sample_format();
        let config: cpal::StreamConfig = sc.into();
        let (ch, clock, err) = (cfg.channel, cfg.clock, stream_error);
        let stream = match format {
            cpal::SampleFormat::I8 => input_stream::<i8>(&dev, &config, reader, ch, clock, tx, err),
            cpal::SampleFormat::I16 => input_stream::<i16>(&dev, &config, reader, ch, clock, tx, err),
            cpal::SampleFormat::I24 => input_stream::<cpal::I24>(&dev, &config, reader, ch, clock, tx, err),
            cpal::SampleFormat::I32 => input_stream::<i32>(&dev, &config, reader, ch, clock, tx, err),
            cpal::SampleFormat::I64 => input_stream::<i64>(&dev, &config, reader, ch, clock, tx, err),
            cpal::SampleFormat::U8 => input_stream::<u8>(&dev, &config, reader, ch, clock, tx, err),
            cpal::SampleFormat::U16 => input_stream::<u16>(&dev, &config, reader, ch, clock, tx, err),
            cpal::SampleFormat::U32 => input_stream::<u32>(&dev, &config, reader, ch, clock, tx, err),
            cpal::SampleFormat::U64 => input_stream::<u64>(&dev, &config, reader, ch, clock, tx, err),
            cpal::SampleFormat::F32 => input_stream::<f32>(&dev, &config, reader, ch, clock, tx, err),
            cpal::SampleFormat::F64 => input_stream::<f64>(&dev, &config, reader, ch, clock, tx, err),
            other => return Err(format!("LTC input: unsupported sample format {other:?}")),
        }
        .map_err(|e| format!("LTC input: {e}"))?;
        stream.play().map_err(|e| format!("LTC input: {e}"))?;
        Ok(stream)
    })
}

fn output_stream<T: SizedSample + FromSample<f32>>(
    dev: &cpal::Device,
    config: &cpal::StreamConfig,
    mut generator: LtcGenerator,
    channel: u16,
    clock: Clock,
    transport: SharedTransport,
    error: ErrorCell,
) -> Result<cpal::Stream, cpal::BuildStreamError> {
    let channels = usize::from(config.channels).max(1);
    let mut mono = Vec::new();
    let mut last = Transport::default();
    dev.build_output_stream(
        config,
        move |data: &mut [T], info: &cpal::OutputCallbackInfo| {
            let now = clock.now();
            // never wait for the control thread in the audio callback: keep the last transport
            if let Ok(t) = transport.try_lock() {
                last = *t;
            }
            let ts = info.timestamp();
            let ahead = ts.playback.duration_since(&ts.callback).map(duration_ticks).unwrap_or_default();
            mono.resize(data.len() / channels, 0.0);
            generator.fill(&mut mono, cfa4_time::Tick(now.0 + ahead.0), &last);
            for (frame, &s) in data.chunks_exact_mut(channels).zip(&mono) {
                for (c, out) in frame.iter_mut().enumerate() {
                    *out = if c == usize::from(channel) { T::from_sample(s) } else { T::EQUILIBRIUM };
                }
            }
        },
        move |e| report(&error, format!("LTC output: {e}")),
        None,
    )
}

/// Opens the LTC output; it follows `transport`.
pub fn open_ltc_output(cfg: &LtcOutputConfig, transport: SharedTransport) -> Result<Box<dyn Device>, String> {
    let cfg = cfg.clone();
    let error = ErrorCell::default();
    let stream_error = error.clone();
    spawn("timecode LTC out", error, move || {
        let h = host(&cfg.host);
        let dev = if cfg.device.is_empty() { h.default_output_device() } else { find(h.output_devices(), &cfg.device) };
        let label = if cfg.device.is_empty() { "the default output".to_string() } else { format!("output `{}`", cfg.device) };
        let dev = dev.ok_or_else(|| format!("LTC output: no {label}"))?;
        let sc = dev.default_output_config().map_err(|e| format!("LTC output: {e}"))?;
        check_channel(cfg.channel, sc.channels(), &label)?;
        let generator = LtcGenerator::new(sc.sample_rate().0, cfg.rate, cfg.level_dbfs, cfg.offset, cfg.latency).map_err(|e| format!("LTC output: {e}"))?;
        let format = sc.sample_format();
        let config: cpal::StreamConfig = sc.into();
        let (ch, clock, tr, err) = (cfg.channel, cfg.clock, transport, stream_error);
        let stream = match format {
            cpal::SampleFormat::I8 => output_stream::<i8>(&dev, &config, generator, ch, clock, tr, err),
            cpal::SampleFormat::I16 => output_stream::<i16>(&dev, &config, generator, ch, clock, tr, err),
            cpal::SampleFormat::I24 => output_stream::<cpal::I24>(&dev, &config, generator, ch, clock, tr, err),
            cpal::SampleFormat::I32 => output_stream::<i32>(&dev, &config, generator, ch, clock, tr, err),
            cpal::SampleFormat::I64 => output_stream::<i64>(&dev, &config, generator, ch, clock, tr, err),
            cpal::SampleFormat::U8 => output_stream::<u8>(&dev, &config, generator, ch, clock, tr, err),
            cpal::SampleFormat::U16 => output_stream::<u16>(&dev, &config, generator, ch, clock, tr, err),
            cpal::SampleFormat::U32 => output_stream::<u32>(&dev, &config, generator, ch, clock, tr, err),
            cpal::SampleFormat::U64 => output_stream::<u64>(&dev, &config, generator, ch, clock, tr, err),
            cpal::SampleFormat::F32 => output_stream::<f32>(&dev, &config, generator, ch, clock, tr, err),
            cpal::SampleFormat::F64 => output_stream::<f64>(&dev, &config, generator, ch, clock, tr, err),
            other => return Err(format!("LTC output: unsupported sample format {other:?}")),
        }
        .map_err(|e| format!("LTC output: {e}"))?;
        stream.play().map_err(|e| format!("LTC output: {e}"))?;
        Ok(stream)
    })
}

#[cfg(test)]
mod tests {
    use cfa4_time::{FrameRate, Tick};

    use super::*;
    use crate::ExternalRate;

    #[test]
    fn a_missing_device_is_an_error_not_a_crash() {
        let rate = ExternalRate::new(FrameRate::FPS_25, false);
        let clock = Clock::new();
        let (tx, _rx) = mpsc::channel();
        let cfg = LtcInputConfig { host: "no such host".into(), device: "no such device ×".into(), channel: 0, rate, clock };
        let r = open_ltc_input(&cfg, tx);
        assert!(r.is_err_and(|e| e.contains("no such device")));
        let cfg = LtcOutputConfig {
            host: String::new(),
            device: "no such device ×".into(),
            channel: 0,
            level_dbfs: -18.0,
            rate,
            offset: Tick(0),
            latency: Tick(0),
            clock,
        };
        assert!(open_ltc_output(&cfg, SharedTransport::default()).is_err());
        assert!(check_channel(2, 2, "x").is_err() && check_channel(1, 2, "x").is_ok());
    }
}
