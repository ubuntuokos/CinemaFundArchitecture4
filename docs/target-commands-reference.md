# Target app: menu command names (reference for `cargo xtask parity --area commands`)

The menu commands of the target video editor, by menu: the reference of `cargo xtask parity --area commands`
(`xtask/parity/reference/commands.json`, owner decision 2026-11-28, G7). Names only; no icon, preset or other asset
of that application is copied, and nothing was read from the application itself (AGENTS.md §2, clean-room).

**Provenance and status.** Compiled from the vendor's public documentation and general knowledge of the menu
structure, 196 commands in 7 menus. It is **not** read from the application, so it may differ from a
specific version: the owner checks it against an installed copy and corrects it here (and in `commands.json`, which
`cargo xtask parity --check-reference` keeps in step with this file). Left out on purpose: the application, Apple
and Help menus, the Window menu (its items are the panels, see `docs/qt-parity.md`), dynamic entries (Open Recent,
Import Recent File, the workspace list, the Window ▸ Extensions list), and anything this list is not sure of. A name
here says that the command exists in the reference, not how it behaves (gap G1).

## File

- New ▸ Project
- New ▸ Production
- New ▸ Sequence
- New ▸ Sequence From Clip
- New ▸ Bin
- New ▸ Search Bin
- New ▸ Offline File
- New ▸ Adjustment Layer
- New ▸ Legacy Title
- New ▸ Photoshop File
- New ▸ Bars and Tone
- New ▸ Black Video
- New ▸ Color Matte
- New ▸ HD Bars and Tone
- New ▸ Universal Counting Leader
- New ▸ Transparent Video
- Open Project
- Open Production
- Close
- Close Project
- Close Production
- Close All Projects
- Refresh All Projects
- Revert
- Save
- Save As
- Save a Copy
- Save All
- Capture
- Batch Capture
- Link Media
- Make Offline
- Import from Media Browser
- Import
- Export ▸ Media
- Export ▸ Motion Graphics Template
- Export ▸ Captions
- Export ▸ Markers
- Export ▸ Selection
- Export ▸ EDL
- Export ▸ OMF
- Export ▸ AAF
- Export ▸ Avid Log Exchange
- Export ▸ Final Cut Pro XML
- Export ▸ Selects As Premiere Project
- Get Properties for ▸ File
- Get Properties for ▸ Selection
- Project Settings ▸ General
- Project Settings ▸ Scratch Disks
- Project Settings ▸ Ingest Settings
- Production Settings
- Project Manager
- Exit

## Edit

- Undo
- Redo
- Cut
- Copy
- Paste
- Paste Insert
- Paste Attributes
- Clear
- Ripple Delete
- Duplicate
- Select All
- Deselect All
- Find
- Find Next
- Label ▸ Violet
- Label ▸ Iris
- Label ▸ Caribbean
- Label ▸ Lavender
- Label ▸ Cerulean
- Label ▸ Forest
- Label ▸ Rose
- Label ▸ Mango
- Label ▸ Purple
- Label ▸ Blue
- Label ▸ Teal
- Label ▸ Magenta
- Label ▸ Tan
- Label ▸ Green
- Label ▸ Brown
- Label ▸ Yellow
- Label ▸ Select Label Group
- Label ▸ Label Defaults
- Remove Unused
- Consolidate Duplicates
- Edit Original
- Edit in Adobe Audition ▸ Sequence
- Edit in Adobe Audition ▸ Clip
- Edit in Adobe Photoshop
- Keyboard Shortcuts
- Preferences

## Clip

- Rename
- Make Subclip
- Edit Subclip
- Edit Offline
- Source Settings
- Modify ▸ Audio Channels
- Modify ▸ Interpret Footage
- Modify ▸ Timecode
- Video Options ▸ Frame Hold Options
- Video Options ▸ Add Frame Hold
- Video Options ▸ Insert Frame Hold Segment
- Video Options ▸ Field Options
- Video Options ▸ Time Interpolation
- Video Options ▸ Scale to Frame Size
- Video Options ▸ Set to Frame Size
- Audio Options ▸ Audio Gain
- Audio Options ▸ Breakout to Mono
- Audio Options ▸ Extract Audio
- Speed/Duration
- Remove Effects
- Insert
- Overwrite
- Enable
- Link
- Group
- Ungroup
- Synchronize
- Nest
- Make Subsequence

## Sequence

- Sequence Settings
- Render Effects in Work Area
- Render Entire Work Area
- Render Selection
- Render Audio
- Delete Render Files
- Delete Work Area Render Files
- Lift
- Extract
- Apply Video Transition
- Apply Audio Transition
- Apply Default Transitions
- Zoom In
- Zoom Out
- Go to Gap ▸ Next in Sequence
- Go to Gap ▸ Previous in Sequence
- Go to Gap ▸ Next in Track
- Go to Gap ▸ Previous in Track
- Add Edit
- Add Edit to All Tracks
- Trim Edit
- Extend Selected Edit to Playhead
- Add Tracks
- Delete Tracks
- Normalize Master Track
- Make Subsequence

## Markers

- Mark In
- Mark Out
- Mark Clip
- Mark Selection
- Go to In
- Go to Out
- Clear In
- Clear Out
- Clear In and Out
- Add Marker
- Go to Next Marker
- Go to Previous Marker
- Clear Selected Marker
- Clear All Markers
- Edit Marker
- Add Chapter Marker

## Graphics and Titles

- New Layer ▸ Text
- New Layer ▸ Vertical Text
- New Layer ▸ Rectangle
- New Layer ▸ Ellipse
- New Layer ▸ From File
- Install Motion Graphics Template
- Select Next Layer
- Select Previous Layer
- Align ▸ Left
- Align ▸ Center Horizontally
- Align ▸ Right
- Align ▸ Top
- Align ▸ Center Vertically
- Align ▸ Bottom
- Arrange ▸ Bring to Front
- Arrange ▸ Bring Forward
- Arrange ▸ Send Backward
- Arrange ▸ Send to Back
- Replace All Fonts

## View

- Playback Resolution
- Pause Resolution
- Display Mode
- Magnification
- Show Rulers
- Show Guides
- Lock Guides
- Add Guide
- Clear Guides
- Snap in Program Monitor
- Show Transparency Grid
- Loop
- Overlays
