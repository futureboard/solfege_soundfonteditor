# Solfege SoundFont Editor

A SoundFont 2 (`.sf2`) editor written in Rust with **egui** on the **wgpu** renderer (via eframe), with a built-in preview synthesizer (cpal).

## Features

- **Open / save SF2**: a self-contained RIFF reader and writer (`src/sf2`). Saves go to a temp file first, so a failed save can't corrupt the original.
- **Presets / Instruments / Samples** in a filterable list. You can create, duplicate (Ctrl+D) and delete items. Deleting cleans up the references to them.
- **Zone editor**: pick the instrument/sample and set key and velocity ranges. A horizontal key map sits above a piano strip; zones that don't overlap share a row. You can add a global zone and sort zones by key.
- **Generator editor**: covers every SF2 generator, with units shown in Hz, dB, ms and note names. Preset-level values are shown as offsets. Values inherited from the global zone are displayed.
- **Modulator editor**: edits the raw SF2 modulator records.
- **Sample editor**:
  - Waveform view: scroll to zoom, drag to pan, and drag the LS/LE markers to set loop points.
  - Loop tools: snap to zero crossings, clear the loop, or loop the whole sample.
  - Sample properties: root key, pitch correction, sample rate, and stereo links.
  - Processing: normalize and reverse.
  - Create an instrument from a sample; stereo pairs are auto-panned.
- **WAV import/export**: supports 8/16/24/32-bit and float input. Stereo files become linked L/R samples.
- **Preview synth**:
  - Play with the on-screen piano or the computer keyboard (Z–M / Q–P, ←/→ to change octave, Esc for all notes off).
  - Supports a volume envelope, a low-pass filter, loop modes, exclusive classes and pan.
- **Audio settings** (View → Audio settings… or the ⚙ button): choose the output device and buffer size. The choice is remembered between runs.
- **Undo / redo** (Ctrl+Z / Ctrl+Y). Problem checker for duplicate bank/program, empty zones and invalid loops.
- Drag and drop `.sf2` files to open them, or `.wav` files to import them.

## Build & run

```sh
cargo run --release              # empty document
cargo run --release -- file.sf2  # open a file
cargo test                       # SF2 round-trip tests
```

## Known limitations

- 24-bit `sm24` data is ignored, so saved files are 16-bit.
- The preview synth does not render the modulation envelope, LFOs, chorus/reverb or modulators. These values are still edited and saved correctly.
