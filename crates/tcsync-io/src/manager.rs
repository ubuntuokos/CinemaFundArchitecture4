//! The sync manager: opens the devices the settings ask for, feeds the chase, and tells the
//! player what to do.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use cfa4_tcsync::chase::{Chase, ChaseAction, ChaseConfig, ChaseState, ChaseStatus};
use cfa4_time::Tick;

use crate::mtc::MtcScheduler;
use crate::{Clock, ExternalRate, Input, SharedTransport, Transport, lock};

/// Where chase reads the external clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Ltc,
    Mtc,
}

/// Resolved settings (the engine's `timecodeSync.*` preferences plus the sequence).
#[derive(Clone, Debug, PartialEq)]
pub struct SyncSettings {
    /// `Some` when chase is on.
    pub chase: Option<ChaseConfig>,
    pub source: Source,
    pub rate: ExternalRate,
    /// Added to the external time to get the sequence time (includes the sequence start time).
    pub offset: Tick,
    /// cpal host ("" = default).
    pub audio_host: String,
    pub ltc_input: String,
    pub ltc_input_channel: u16,
    pub generate_ltc: bool,
    pub ltc_output: String,
    pub ltc_output_channel: u16,
    pub ltc_level_dbfs: f32,
    pub generate_mtc: bool,
    pub mtc_input_port: String,
    pub mtc_output_port: String,
    /// Send timecode this much ahead of the picture.
    pub output_latency: Tick,
}

impl SyncSettings {
    /// Settings with everything off.
    pub fn off(rate: ExternalRate) -> Self {
        SyncSettings {
            chase: None,
            source: Source::Ltc,
            rate,
            offset: Tick(0),
            audio_host: String::new(),
            ltc_input: String::new(),
            ltc_input_channel: 0,
            generate_ltc: false,
            ltc_output: String::new(),
            ltc_output_channel: 0,
            ltc_level_dbfs: -18.0,
            generate_mtc: false,
            mtc_input_port: String::new(),
            mtc_output_port: String::new(),
            output_latency: Tick(0),
        }
    }

    /// Whether any device is needed.
    pub fn active(&self) -> bool {
        self.chase.is_some() || self.generate_ltc || self.generate_mtc
    }

    fn ltc_input(&self, clock: Clock) -> Option<LtcInputConfig> {
        (self.chase.is_some() && self.source == Source::Ltc).then(|| LtcInputConfig {
            host: self.audio_host.clone(),
            device: self.ltc_input.clone(),
            channel: self.ltc_input_channel,
            rate: self.rate,
            clock,
        })
    }

    fn ltc_output(&self, clock: Clock) -> Option<LtcOutputConfig> {
        self.generate_ltc.then(|| LtcOutputConfig {
            host: self.audio_host.clone(),
            device: self.ltc_output.clone(),
            channel: self.ltc_output_channel,
            level_dbfs: self.ltc_level_dbfs,
            rate: self.rate,
            offset: self.offset,
            latency: self.output_latency,
            clock,
        })
    }

    fn mtc(&self, clock: Clock) -> Option<MtcConfig> {
        let input = (self.chase.is_some() && self.source == Source::Mtc).then(|| self.mtc_input_port.clone());
        let output = self.generate_mtc.then(|| self.mtc_output_port.clone());
        (input.is_some() || output.is_some()).then_some(MtcConfig { input, output, rate: self.rate, offset: self.offset, latency: self.output_latency, clock })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LtcInputConfig {
    pub host: String,
    /// Device name ("" = default input).
    pub device: String,
    pub channel: u16,
    pub rate: ExternalRate,
    pub clock: Clock,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LtcOutputConfig {
    pub host: String,
    /// Device name ("" = default output).
    pub device: String,
    pub channel: u16,
    pub level_dbfs: f32,
    pub rate: ExternalRate,
    pub offset: Tick,
    pub latency: Tick,
    pub clock: Clock,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MtcConfig {
    /// `Some` to read MTC; the port to connect from ("" = only CFA4's own input port).
    pub input: Option<String>,
    /// `Some` to send MTC; the port to connect to ("" = only CFA4's own output port).
    pub output: Option<String>,
    pub rate: ExternalRate,
    pub offset: Tick,
    pub latency: Tick,
    pub clock: Clock,
}

impl PartialEq for Clock {
    fn eq(&self, _: &Self) -> bool {
        true // one clock per manager
    }
}

/// An open device; dropping it stops it.
pub trait Device: Send {
    /// The error the device stopped with, if any.
    fn error(&self) -> Option<String>;
}

/// Opens devices (the hardware backend, or fakes in tests).
pub trait Backend {
    fn ltc_input(&mut self, cfg: &LtcInputConfig, tx: Sender<Input>) -> Result<Box<dyn Device>, String>;
    fn ltc_output(&mut self, cfg: &LtcOutputConfig, transport: SharedTransport) -> Result<Box<dyn Device>, String>;
    fn mtc(&mut self, cfg: &MtcConfig, tx: Sender<Input>, transport: SharedTransport) -> Result<Box<dyn Device>, String>;
}

/// What the status command shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SyncStatus {
    pub chase: Option<ChaseStatus>,
    pub ltc_out_running: bool,
    pub mtc_out_running: bool,
    pub error: Option<String>,
    pub warning: Option<String>,
}

struct Slot<C> {
    cfg: Option<C>,
    dev: Option<Box<dyn Device>>,
    error: Option<String>,
}

impl<C> Default for Slot<C> {
    fn default() -> Self {
        Slot { cfg: None, dev: None, error: None }
    }
}

impl<C> Slot<C> {
    fn error(&self) -> Option<String> {
        self.error.clone().or_else(|| self.dev.as_ref().and_then(|d| d.error()))
    }
}

/// Hold back repeated "locate and play" actions this long while the player has not answered.
fn locate_debounce() -> Tick {
    Tick::from_units(1, 4)
}

pub struct SyncManager<B: Backend> {
    backend: B,
    clock: Clock,
    settings: Option<SyncSettings>,
    transport: SharedTransport,
    chase: Option<Chase>,
    tx: Sender<Input>,
    rx: Receiver<Input>,
    ltc_in: Slot<LtcInputConfig>,
    ltc_out: Slot<LtcOutputConfig>,
    mtc: Slot<MtcConfig>,
    input_error: Option<String>,
    /// A locate was sent at this time and the player has not reported since.
    locate_sent: Option<Tick>,
}

impl<B: Backend> SyncManager<B> {
    pub fn new(backend: B) -> Self {
        let (tx, rx) = channel();
        SyncManager {
            backend,
            clock: Clock::new(),
            settings: None,
            transport: Arc::new(Mutex::new(Transport::default())),
            chase: None,
            tx,
            rx,
            ltc_in: Slot::default(),
            ltc_out: Slot::default(),
            mtc: Slot::default(),
            input_error: None,
            locate_sent: None,
        }
    }

    pub fn clock(&self) -> Clock {
        self.clock
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    /// Applies new settings, reopening only the devices whose settings changed.
    pub fn apply(&mut self, s: SyncSettings) {
        if self.settings.as_ref() == Some(&s) {
            return;
        }
        let clock = self.clock;
        match (s.chase, self.chase.as_mut()) {
            (None, _) => self.chase = None,
            (Some(cfg), Some(c)) if c.config().rate == cfg.rate => c.set_config(cfg),
            (Some(cfg), _) => self.chase = Some(Chase::new(cfg)),
        }
        let tx = self.tx.clone();
        let transport = self.transport.clone();
        let backend = &mut self.backend;
        let reopened_in = reopen(&mut self.ltc_in, s.ltc_input(clock), |c| backend.ltc_input(c, tx.clone()));
        reopen(&mut self.ltc_out, s.ltc_output(clock), |c| backend.ltc_output(c, transport.clone()));
        let reopened_mtc = reopen(&mut self.mtc, s.mtc(clock), |c| backend.mtc(c, tx.clone(), transport.clone()));
        if reopened_in || reopened_mtc {
            // frames from a closed input must not reach the new chase
            while self.rx.try_recv().is_ok() {}
            self.input_error = None;
        }
        self.settings = Some(s);
    }

    /// The player's transport, as it reports it.
    pub fn set_transport(&mut self, playing: bool, position: Tick, speed: f64) {
        let at = self.clock.now();
        *lock(&self.transport) = Transport { playing, position, at, speed: if speed.is_finite() { speed } else { 1.0 } };
        self.locate_sent = None;
    }

    pub fn transport(&self) -> Transport {
        *lock(&self.transport)
    }

    /// Reads what the inputs received and checks the player: returns what it should do.
    pub fn poll(&mut self) -> Vec<ChaseAction> {
        let now = self.clock.now();
        let mut out = Vec::new();
        let mut inputs = Vec::new();
        while let Ok(i) = self.rx.try_recv() {
            inputs.push(i);
        }
        let tr = self.transport();
        let Some(chase) = self.chase.as_mut() else {
            return out;
        };
        let offset = chase.config().offset;
        for i in inputs {
            match i {
                Input::Frame(o) => chase.observe(o, &mut out),
                Input::Locate(o) => {
                    if !matches!(chase.status().state, ChaseState::Locked | ChaseState::Freewheel) {
                        out.push(ChaseAction::Locate { to: Tick(o.position.0.saturating_add(offset.0)), play: false });
                    }
                }
                Input::Error(e) => self.input_error = Some(e),
            }
        }
        chase.tick(now, tr.playing.then(|| tr.position_at(now)), &mut out);
        // only the latest locate counts
        if let Some(last) = out.iter().rposition(|a| matches!(a, ChaseAction::Locate { .. })) {
            let mut i = 0;
            out.retain(|a| {
                let keep = i == last || !matches!(a, ChaseAction::Locate { .. });
                i += 1;
                keep
            });
        }
        // While a locate is on its way, the player still reports its old state: do not repeat it.
        if let Some(sent) = self.locate_sent
            && now.0.saturating_sub(sent.0) < locate_debounce().0
        {
            out.retain(|a| !matches!(a, ChaseAction::Locate { play: true, .. }));
        }
        if out.iter().any(|a| matches!(a, ChaseAction::Locate { play: true, .. })) {
            self.locate_sent = Some(now);
        }
        out
    }

    pub fn status(&self) -> SyncStatus {
        let s = self.settings.as_ref();
        let error = [self.ltc_in.error(), self.ltc_out.error(), self.mtc.error(), self.input_error.clone()].into_iter().flatten().collect::<Vec<_>>();
        let warning = s.filter(|s| (s.generate_mtc || (s.chase.is_some() && s.source == Source::Mtc)) && !MtcScheduler::exact(s.rate)).map(|s| {
            format!("MIDI Timecode has no code for {} fps: it carries the labels with the nearest code, at the real {0} fps speed", s.rate.rate.label())
        });
        SyncStatus {
            chase: self.chase.as_ref().map(Chase::status),
            ltc_out_running: self.ltc_out.dev.as_ref().is_some_and(|d| d.error().is_none()),
            mtc_out_running: s.is_some_and(|s| s.generate_mtc) && self.mtc.dev.as_ref().is_some_and(|d| d.error().is_none()),
            error: (!error.is_empty()).then(|| error.join("; ")),
            warning,
        }
    }

    /// Closes every device.
    pub fn shutdown(&mut self) {
        self.apply(SyncSettings::off(self.settings.as_ref().map_or(ExternalRate::new(Default::default(), false), |s| s.rate)));
    }
}

/// Closes and reopens a device when its settings changed; returns whether it did.
fn reopen<C: PartialEq>(slot: &mut Slot<C>, cfg: Option<C>, open: impl FnOnce(&C) -> Result<Box<dyn Device>, String>) -> bool {
    if slot.cfg == cfg {
        return false;
    }
    slot.dev = None; // stop the old one before the new one takes the device
    slot.error = None;
    if let Some(c) = &cfg {
        match open(c) {
            Ok(d) => slot.dev = Some(d),
            Err(e) => slot.error = Some(e),
        }
    }
    slot.cfg = cfg;
    true
}

#[cfg(test)]
mod tests {
    use cfa4_tcsync::chase::Observation;
    use cfa4_time::FrameRate;

    use super::*;

    #[derive(Default)]
    struct Fake {
        opened: Vec<String>,
        fail_output: bool,
        tx: Option<Sender<Input>>,
    }

    struct FakeDev(Option<String>);

    impl Device for FakeDev {
        fn error(&self) -> Option<String> {
            self.0.clone()
        }
    }

    impl Backend for Fake {
        fn ltc_input(&mut self, cfg: &LtcInputConfig, tx: Sender<Input>) -> Result<Box<dyn Device>, String> {
            self.opened.push(format!("ltc in {}:{}", cfg.device, cfg.channel));
            self.tx = Some(tx);
            Ok(Box::new(FakeDev(None)))
        }
        fn ltc_output(&mut self, cfg: &LtcOutputConfig, _: SharedTransport) -> Result<Box<dyn Device>, String> {
            self.opened.push(format!("ltc out {}", cfg.device));
            if self.fail_output { Err("no output device `x`".into()) } else { Ok(Box::new(FakeDev(None))) }
        }
        fn mtc(&mut self, cfg: &MtcConfig, tx: Sender<Input>, _: SharedTransport) -> Result<Box<dyn Device>, String> {
            self.opened.push(format!("mtc {:?} {:?}", cfg.input, cfg.output));
            self.tx = Some(tx);
            Ok(Box::new(FakeDev(None)))
        }
    }

    fn settings() -> SyncSettings {
        let rate = ExternalRate::new(FrameRate::FPS_25, false);
        SyncSettings { chase: Some(ChaseConfig::new(FrameRate::FPS_25)), ..SyncSettings::off(rate) }
    }

    #[test]
    fn opens_what_the_settings_ask_for_and_reopens_only_changes() {
        let mut m = SyncManager::new(Fake::default());
        m.apply(SyncSettings::off(ExternalRate::new(FrameRate::FPS_25, false)));
        assert!(m.backend.opened.is_empty());
        let mut s = settings();
        m.apply(s.clone());
        assert_eq!(m.backend.opened, ["ltc in :0"]);
        s.generate_mtc = true;
        m.apply(s.clone());
        assert_eq!(m.backend.opened, ["ltc in :0", "mtc None Some(\"\")"]);
        m.apply(s.clone());
        assert_eq!(m.backend.opened.len(), 2, "same settings: nothing reopens");
        s.source = Source::Mtc;
        s.mtc_input_port = "20:0".into();
        m.apply(s);
        assert_eq!(m.backend.opened[2], "mtc Some(\"20:0\") Some(\"\")");
        assert!(m.ltc_in.dev.is_none(), "LTC input closed when the source moved to MTC");
    }

    #[test]
    fn errors_and_warnings_show_in_the_status() {
        let mut m = SyncManager::new(Fake { fail_output: true, ..Fake::default() });
        let mut s = settings();
        s.generate_ltc = true;
        s.generate_mtc = true;
        s.rate = ExternalRate::new(FrameRate::FPS_23_976, false);
        m.apply(s);
        let st = m.status();
        assert_eq!(st.error.as_deref(), Some("no output device `x`"));
        assert!(!st.ltc_out_running);
        assert!(st.mtc_out_running);
        assert!(st.warning.is_some_and(|w| w.contains("23.976")));
        assert_eq!(st.chase.map(|c| c.state), Some(ChaseState::NoSignal));
    }

    #[test]
    fn frames_lock_and_locates_are_not_repeated_while_the_player_answers() {
        let mut m = SyncManager::new(Fake::default());
        m.apply(settings());
        let tx = m.backend.tx.clone().expect("input opened");
        let frame = FrameRate::FPS_25.frame_duration().0;
        let start = Tick(m.clock.now().0 - 3 * frame);
        let obs = |k: i64| Input::Frame(Observation { at: Tick(start.0 + k * frame), position: Tick(Tick::from_units(10, 1).0 + k * frame) });
        for k in 0..3 {
            let _ = tx.send(obs(k));
        }
        let a = m.poll();
        assert!(matches!(a.as_slice(), [ChaseAction::Locate { play: true, .. }]), "{a:?}");
        // The player has not reported yet: the chase would locate again, the manager holds it.
        let _ = tx.send(obs(3));
        assert!(m.poll().is_empty());
        m.set_transport(true, Tick::from_units(10, 1), 1.0);
        assert_eq!(m.transport().speed, 1.0);
    }

    #[test]
    fn a_full_frame_locates_without_play_unless_locked() {
        let mut m = SyncManager::new(Fake::default());
        let mut s = settings();
        s.source = Source::Mtc;
        if let Some(c) = s.chase.as_mut() {
            c.offset = Tick::from_units(-3600, 1);
        }
        m.apply(s);
        let tx = m.backend.tx.clone().expect("mtc opened");
        let _ = tx.send(Input::Locate(Observation { at: m.clock.now(), position: Tick::from_units(3610, 1) }));
        assert_eq!(m.poll(), [ChaseAction::Locate { to: Tick::from_units(10, 1), play: false }]);
        let _ = tx.send(Input::Error("MIDI port gone".into()));
        m.poll();
        assert_eq!(m.status().error.as_deref(), Some("MIDI port gone"));
    }

    #[test]
    fn chase_off_gives_no_actions_and_shutdown_closes_all() {
        let mut m = SyncManager::new(Fake::default());
        let mut s = SyncSettings::off(ExternalRate::new(FrameRate::FPS_25, false));
        s.generate_ltc = true;
        m.apply(s);
        assert!(m.poll().is_empty());
        assert!(m.status().ltc_out_running);
        m.shutdown();
        assert!(!m.status().ltc_out_running);
    }
}
