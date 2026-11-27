# Target app: Effects panel names (reference for `cargo xtask parity`)

The names of the effects and transitions the target video editor's Effects panel lists, by folder. They are
the reference of `cargo xtask parity --area effects` and `--area transitions`
(`xtask/parity/reference/effects.json`, `transitions.json`; owner decision 2026-11-25, G7). Names are functional
facts; no icon, preset, LUT, font or other asset of that application is copied (AGENTS.md §1, §2).

How the list was checked: the video effects are the table in `premiere_26_video_effects_catalogue`
(`crates/project/src/effect/vfx.rs`), the Legacy video effects the table `LEGACY_NAMES` next to it, the audio
effects `PREMIERE_AUDIO_EFFECTS` (`premiere_audio_set_is_complete_and_foldered`, `crates/project/src/effect/audio.rs`),
and the transitions the folders of `premiere_26_transition_tree` (`crates/project/src/vtransition.rs`) without CFA4's
own transitions. Counts: video effects 93 in 16 folders, Legacy video effects 15, audio effects 53, video
transitions 84 in 10 folders, audio transitions 3. Not listed: Obsolete and Legacy transitions, presets (out of
scope for the tool), and the effects CFA4 adds on its own. A name here is a presence reference only, not a
behaviour claim (gap G1); fidelity is tracked in `xtask/parity/fidelity.json`.


## Video effects


### Video Effects ▸ Adjust

- Extract
- Levels
- Lighting Effects
- ProcAmp

### Video Effects ▸ Blur & Sharpen

- Bokeh Blur
- Channel Blur
- Compound Blur
- Directional Blur
- Focus Blur
- Gaussian Blur
- Reduce Interlace Flicker
- Sharpen
- Unsharp Mask

### Video Effects ▸ Color Correction

- ASC CDL
- Brightness & Contrast
- Lumetri Color
- Tint
- Video Limiter
- Vignette

### Video Effects ▸ Distort

- Corner Pin
- Lens Distortion
- Magnify
- Mirror
- Spherize
- Turbulent Displace
- Twirl
- Warp Stabilizer
- Wave Warp

### Video Effects ▸ Generate

- 4-Color Gradient
- Gradient

### Video Effects ▸ Image Control

- Black & White
- Channel Mix
- Color Pass
- Color Replace
- Gamma Correction
- Invert
- Rounded Crop

### Video Effects ▸ Immersive Video

- VR Blur
- VR Chromatic Aberrations
- VR Color Gradients
- VR De-Noise
- VR Digital Glitch
- VR Fractal Noise
- VR Glow
- VR Plane to Sphere
- VR Projection
- VR Rotate Sphere
- VR Sharpen

### Video Effects ▸ Keying

- Alpha Adjust
- Color Key
- Logo Cutout
- Luma Key
- Track Matte Key
- Ultra Key

### Video Effects ▸ Lights & Glows

- Echo Glow
- Edge Glow
- Glint
- Lens Flare
- Light Leaks
- RGB Split
- Volumetric Rays
- Wonder Glow

### Video Effects ▸ Noise & Grain

- Noise

### Video Effects ▸ Perspective

- Basic 3D
- Drop Shadow
- Long Shadow

### Video Effects ▸ Stylize

- Brush Strokes
- Color Emboss
- Find Edges
- Mosaic
- Posterize
- Roughen Edges
- Strobe Light

### Video Effects ▸ Time

- Posterize Time

### Video Effects ▸ Transform

- 3D Rotate
- Auto Reframe
- Camera Shake
- Grow
- Horizontal Flip
- Move
- Offset
- Shrink
- Spacer
- Spin
- Transform
- Vertical Flip
- Wiggle

### Video Effects ▸ Utility

- Auto Align
- Cineon Converter
- Clone
- Simple Text
- Stroke

### Video Effects ▸ Video

- Metadata & Timecode Burn-in

### Legacy ▸ Video Effects

- Alpha Glow
- Block Dissolve
- Camera Blur
- Crop
- Directional Blur (Legacy)
- Edge Feather
- Gaussian Blur (Legacy)
- Gradient Wipe (Legacy)
- Linear Wipe (Legacy)
- Magnify (Legacy)
- Mosaic (Legacy)
- Noise (Legacy)
- Ramp
- Replicate
- Twirl (Legacy)

## Audio effects


### Audio Effects ▸ Amplitude and Compression

- Amplify
- Channel Mixer
- Channel Volume
- DeEsser
- Dynamics
- Dynamics Processing
- Hard Limiter
- Multiband Compressor
- Single-band Compressor
- Tube-modeled Compressor

### Audio Effects ▸ Delay and Echo

- Analog Delay
- Delay
- Multitap Delay

### Audio Effects ▸ Filter and EQ

- Bandpass
- Bass
- FFT Filter
- Graphic Equalizer (10 Bands)
- Graphic Equalizer (20 Bands)
- Graphic Equalizer (30 Bands)
- Highpass
- Lowpass
- Notch Filter
- Parametric Equalizer
- Scientific Filter
- Simple Notch Filter
- Simple Parametric EQ
- Treble

### Audio Effects ▸ Modulation

- Chorus/Flanger
- Flanger
- Phaser

### Audio Effects ▸ Noise Reduction/Restoration

- Automatic Click Remover
- DeHummer
- DeNoise
- DeReverb

### Audio Effects ▸ Reverb

- Convolution Reverb
- Studio Reverb
- Surround Reverb

### Audio Effects ▸ Special

- Binauralizer - Ambisonics
- Distortion
- Fill Left with Right
- Fill Right with Left
- GuitarSuite
- Invert
- Loudness Meter
- Mastering
- Panner - Ambisonics
- Swap Channels
- Vocal Enhancer

### Audio Effects ▸ Stereo Imagery

- Stereo Expander

### Audio Effects ▸ Time and Pitch

- Pitch Shifter

### Audio Effects

- Balance
- Mute
- Volume

## Video and audio transitions


### Video Transitions ▸ Animation

- Block Motion
- Flip Motion
- Fold Motion
- Pop Motion
- Pull Motion
- Spin Motion
- Spring Motion
- Travel Motion

### Video Transitions ▸ Dissolve

- Additive Dissolve
- Blur Dissolve
- Burn Alpha
- Cross Dissolve
- Dip to Black
- Dip to Color
- Dip to White
- Film Dissolve
- Luma Fade
- Morph Cut
- Mosaic

### Video Transitions ▸ Grunge & Distort

- Chaos
- Earthquake
- Flicker
- Glass
- Glitch
- Grunge
- Kaleidoscope
- Liquid Distortion
- TV Power
- VHS Damage

### Video Transitions ▸ Immersive Video

- VR Chroma Leaks
- VR Gradient Wipe
- VR Iris Wipe
- VR Light Leaks
- VR Light Rays
- VR Mobius Zoom
- VR Random Blocks
- VR Spherical Blur

### Video Transitions ▸ Lights & Blurs

- Burn Chroma
- Chroma Leak
- Cross Zoom
- Directional Blur
- Flare
- Flash
- Glow
- Lens Blur
- Light Leak
- Light Sweep
- Phosphor
- Radial Blur
- Ray
- Solarize
- Stripe
- Zoom Blur

### Video Transitions ▸ Slide

- 3D Roll
- Film Roll
- Push
- Roll
- Slide
- Split
- Stretch
- Whip

### Video Transitions ▸ Smart Tools

- Motion Camera
- Motion Tween
- Shape Dissolve
- Shape Flow

### Video Transitions ▸ Text

- Text Animator
- Typewriter

### Video Transitions ▸ Transformers

- 3D Spin
- 3D Spinback
- Frame
- Louver
- Mirror
- Page Peel
- Slice
- Wave

### Video Transitions ▸ Wipe

- Clock Wipe
- Linear Wipe
- Neon Wipe
- Panel Wipe
- Plateau Wipe
- Radial Wipe
- Soft Wipe
- Star Wipe
- Stretch Wipe

### Audio Transitions ▸ Crossfade

- Constant Power
- Constant Gain
- Exponential Fade
