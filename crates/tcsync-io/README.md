# cfa4-tcsync-io

External timecode sync devices for CFA4 (plan TC-5): LTC in and out over cpal, MIDI Timecode in
and out over the ALSA sequencer, and the `SyncManager` that opens what the `timecodeSync.*`
preferences ask for, feeds the chase (`cfa4-tcsync`) and returns what the player should do.
Device problems become errors in `SyncStatus`, never a crash.

## Adding a timecode source

The chase does not care where time comes from: it takes `Observation { at, position }`, the
local monotonic time (`Clock::now()` of the manager, as a `Tick`) at which the external clock was
at `position` (time of day, wrapping at 24 hours). LTC and MTC are two producers of these. A new
producer, for example the RP188/VITC timecode of a video input, plugs in without changing the
manager (READY-V6, CFA4-DECKLINK-READY-001):

1. Register the source once and keep the sender:

   ```rust
   let tx = manager.external_feed("SDI 1 RP188");
   ```

2. For every frame the source receives, send `Input::Frame(Observation { at, position })`, with
   `at` taken from `manager.clock()` (the same clock the chase checks against) as close to the
   frame's arrival as the source can tell. A repeated frame carries no news; a jump starts a new
   lock. `Input::Locate` is a position without play (like an MTC full frame), and
   `Input::Error(text)` puts the source's failure in the status.

3. Chase reads the feed while the settings say `source: Source::External` and
   `external_source: "SDI 1 RP188"`. Otherwise the feed's input is dropped at each `poll`, so a
   source may keep sending while it is not selected. Selecting a name nobody registered shows
   `no external timecode source named …` in `SyncStatus::error`; `SyncStatus::source` names the
   source in use (`LTC`, `MTC` or the feed's name).

Lock, freewheel through a dropout, stop after it, and relock work for an external feed exactly as
for LTC (test `an_external_50p_feed_locks_freewheels_and_relocks`). The `timecodeSync.*`
preferences do not offer `External` yet: it becomes a choice when a real source registers a feed.
