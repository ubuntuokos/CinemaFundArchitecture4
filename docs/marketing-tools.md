# Marketing and social video tools

Tools for promo, launch and social edits: cutting on the beat, delivering one edit in every
aspect ratio, making ad variants from a table, animated stat titles and a pre-export check. They
came from a review of the GitHub `marketing-video` topic (2026-10-26): the projects there are AI
agent workflows that render HTML to video, so nothing was copied; these are CFA4's own
implementations of the editing steps those workflows repeat. None of them uses a model. The AI
ideas from the same review are listed in [gaps.md](gaps.md) G12.

Every tool is a command (UI, CLI, control channel and MCP dispatch by id) and sits in a menu.

| Tool | Menu | Command | Code |
|---|---|---|---|
| Beat markers | Markers ▸ Add Beat Markers, Clear Beat Markers | `markers.addBeats`, `markers.clearBeats` | `crates/engine/src/beat_markers.rs` |
| Social safe zones | View ▸ Social Safe Zones ▸ Off / TikTok / Instagram Reels / YouTube Shorts | `view.socialSafeZone.*` | `crates/engine/src/social.rs` (`SafeZone`), Program monitor overlay |
| Auto Reframe Sequence | Sequence ▸ Auto Reframe Sequence… | `sequence.autoReframe` | `crates/engine/src/social.rs` |
| Social deliverables | File ▸ Export ▸ Social Deliverables… | `export.socialDeliverables` | `crates/engine/src/social.rs` |
| Check before export | File ▸ Export ▸ Check Before Export…, Markers ▸ Clear Check Markers | `sequence.checkDelivery`, `markers.clearChecks` | `crates/engine/src/delivery_check.rs` |
| Batch variants | File ▸ Export ▸ Batch Variants from Spreadsheet… | `export.batchVariants` | `crates/engine/src/variants.rs` |
| Text animators | Graphics and Titles ▸ Text Animator ▸ None / Counter / Typewriter / Word by Word | `graphics.textAnimator.*` | `crates/project/src/text_animator.rs` |

## Beat markers

Select a music clip on an audio track and choose Markers ▸ Add Beat Markers. A marker lands on
the attack of every beat (`every`: 2, 4 or 8 for every 2nd beat, every bar of 4/4, every
2 bars). The beats come from the deterministic tracker of Clip ▸ Remix (onset envelope, tempo by
autocorrelation, dynamic-programming beat tracking), cached per media range. Speed-changed
clips map through their speed and remixed clips through their pieces; reversed clips are refused.
Running it again replaces the beat markers inside the clip's span. Cuts snap to markers, so
clips can be cut on the beat.

```json
{"cmd": "markers.addBeats", "params": {"every": 4, "color": "Cerulean"}}
```

## Social safe zones

TikTok, Instagram Reels and YouTube Shorts cover the edges of a vertical video with their own
buttons, captions and account names. The overlay shades those parts of the Program monitor and
outlines the safe area. The insets are CFA4's own conservative approximations (as fractions of
a 1080 × 1920 frame), not the platforms' graphics; platforms change their layouts, so treat the
outline as a guide. Monitor state key: `social_zone` (`tiktok`, `reels`, `shorts`, empty = off).

## Auto Reframe Sequence and Social Deliverables

Sequence ▸ Auto Reframe Sequence… copies the active sequence at 9:16, 1:1, 4:5 or 16:9: the short
side of the frame stays, the long side follows the aspect. Every picture clip in the copy gets
the Auto Reframe effect (Target Aspect: Sequence), which keeps the subject in frame. Motion
positions (keyframes included) move with the frame's centre, so centred clips and titles stay
centred; titles keep their size, and the check below reports layers that leave the new frame.

File ▸ Export ▸ Social Deliverables… does that for every aspect (default 16:9, 9:16, 1:1, 4:5;
the sequence itself stands for its own aspect) and adds one export per aspect to the export
queue, loudness-normalised to −14 LUFS (`loudnessLufs`, `false` to skip). Reframed copies from an
earlier run are reused, so after fixing a title in the 9:16 copy, running it again exports that
copy. `start: true` starts the queue.

## Check Before Export

Finds what usually spoils a finished promo, and marks each problem with a red marker:

| Kind | Problem |
|---|---|
| `flashFrame` | a picture clip or graphic only one or two frames long (`flashFrames`) |
| `blackGap` | time inside the edit where no enabled video track shows anything |
| `offline` | a clip on any track (music and voice-over included) whose media is offline |
| `captionFast` | a caption with more than `maxCps` (20) characters per second |
| `captionShort` | a caption on screen for less than 0.7 s |
| `offFrame` | a graphic layer reaching outside the frame |
| `safeZone` | a text layer under a platform's buttons (`zone`: `tiktok`, `reels`, `shorts`, `all`, `none`; vertical frames default to `all`) |

It reads the timeline only (no rendering, no models), so it is instant and deterministic.
Graphics are measured at the middle of the clip, through the clip's Motion, against the
sequence frame, so titles left outside a reframed copy are found.

## Batch Variants from Spreadsheet

Build one sequence as a template, write `{{Column}}` placeholders into its titles (or give a
text layer a Layer Name equal to a column name), then choose File ▸ Export ▸ Batch Variants from
Spreadsheet… and pick a CSV, TSV, `.xlsx` or `.ods` file. The first row names the columns; every
other row becomes a copy of the sequence with its values filled in, and each copy is queued for
export. A `Name` (or `File`) column names the sequences and files; a name that repeats gets
` (2)`, ` (3)`…, so no export overwrites another. A changed source graphic gets its own project
item in each variant, so variants never sync back to the template. Placeholders with no column
are reported. Up to 500 rows; the whole batch is one undo step.

```csv
Name,Headline,Price
Spring sale,Fresh looks,19 €
VIP,Members first,free
```

## Text animators

Graphics and Titles ▸ Text Animator sets an animator on the selected text layer. It changes the
text shown during `anim_delay` … `anim_delay + anim_duration` seconds of the clip; the Source Text
stays what was typed and is shown after the animator ends.

- **Counter** counts the first number in the text up from 0 with an ease-out ("432,014,696
  views"), keeping thousands separators and decimals (`1,250.50`, `1.234.567`, `1 234,5`).
- **Typewriter** reveals the characters one by one.
- **Word by Word** reveals the words one by one.

Per-character styles stay on their characters. The parameters (`anim`, `anim_duration`,
`anim_delay`) are in Effect Controls under Text Animator and settable with `graphics.set`.

## Not included

- Spring keyframe easing (closed-form damped springs) is being added with the keyframe easing
  presets work, so it is not duplicated here.
- AI features (promo from a brief, AI review, generated shots and music, smart shot picking) are
  in the AI plan, [gaps.md](gaps.md) G12.
