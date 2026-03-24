# CLAUDE.md — RiffLab

## What is this project?

RiffLab is a modular music practice workstation for Linux. Load a song, separate it into stems (vocals/drums/bass/guitar), play along with real-time effects on your instrument, and get pitch/timing accuracy feedback.

Built in Rust with Python only for ML inference (Demucs stem separation, madmom beat tracking).

## Development environment

- **OS:** Ubuntu 25.10, kernel 6.17
- **Rust:** 1.94.0 (cargo 1.94.0)
- **Audio:** PipeWire 1.4.7 (with PulseAudio and ALSA compatibility)
- **Audio interface:** Universal Audio Volt 2
  - **Input 1:** Mic/instrument input (mono, `alsa_input...HiFi__Mic1__source`)
  - **Input 2:** Mic/instrument input (mono, `alsa_input...HiFi__Mic2__source`)
  - **Output:** Monitor out (stereo, `alsa_output...HiFi__Line1__sink`)
  - PipeWire exposes each input as a **separate mono source** (not channels of one stereo device)
- **GPU:** NVIDIA (CUDA available for ML inference)
- **CPU:** AMD Ryzen 9950X (16 cores) — CPU load is not a concern for audio processing

## How to build and run

```bash
# Build everything
cargo build --workspace

# Run with audio (JACK via PipeWire)
LD_LIBRARY_PATH=/usr/lib/x86_64-linux-gnu/pipewire-0.3/jack cargo run -p rifflab-app -- song.wav

# Run the passthrough demo (cpal-based, no JACK needed)
cargo run -p rifflab-app --example passthrough_demo 2>/dev/null

# Run tests
cargo test --workspace
```

### Why LD_LIBRARY_PATH?

The system has both `libjack` from jackd2 (doesn't work — no JACK server) and PipeWire's JACK replacement. `LD_LIBRARY_PATH` forces the `jack` crate to load PipeWire's version. Without it, JACK backend fails with "Cannot connect to server socket".

## Workspace structure

```
rifflab/
├── crates/
│   ├── rifflab-core/       # Shared types, traits (AudioProcessor, PitchFrame, etc.)
│   ├── rifflab-audio/      # M1: Audio engine (JACK backend, transport, audio graph)
│   ├── rifflab-fx/         # M2: Effects (noise gate, compressor, overdrive, EQ, reverb)
│   ├── rifflab-ipc/        # IPC client for Python workers (Unix socket + MessagePack)
│   ├── rifflab-stems/      # M3: Stem separator (Rust client for Demucs)
│   ├── rifflab-analysis/   # M4: YIN pitch detection, onset detection, beat tracking
│   ├── rifflab-practice/   # M5: Live comparison, scoring, MIDI reference loading
│   ├── rifflab-ui/         # M7: egui UI shell (skeleton)
│   └── rifflab-app/        # Binary entry point + symphonia decoder
├── workers/                # Python workers (demucs, madmom) — not yet implemented
├── docs/
│   ├── intent/             # PRD, SRS, implementation plan, task backlog
│   └── normative/          # Architecture (SAD), design (SDD), ISO 12207 reference
└── test_tone.wav           # 3s 440Hz test tone for development
```

## Architecture essentials

- **Audio thread:** JACK backend via PipeWire. Zero-alloc real-time callback. Transport state via atomics, commands via lock-free ring buffer (rtrb).
- **Audio graph:** Fixed topology: StemPlayers → mixer → master out. Effects chain on live input via `Box<dyn AudioProcessor>`.
- **UI:** egui (immediate mode). Communicates with audio thread via ring buffers for meters, pitch frames, comparison data.
- **Python workers:** Out-of-process via Unix socket + MessagePack. Not yet wired — IPC client is stubbed.

## Key conventions

- `AudioProcessor` trait in `rifflab-core/src/audio.rs` — all DSP effects implement this. Must be real-time safe (no alloc, no locks, no syscalls).
- Transport: `Transport` (UI side, sends commands) + `TransportRtHandle` (audio thread side, processes commands via atomics).
- Metering: `MeterData` pushed from audio thread to UI via `rtrb::RingBuffer<MeterData>`.
- Presets: TOML format via `rifflab-fx/src/preset.rs`.

## Current state (Phase 1 in progress)

**Working:**
- Workspace compiles (9 crates, 0 errors, 13 tests passing)
- JACK backend connects to PipeWire, auto-connects to system:playback ports
- Symphonia decoder (WAV/FLAC/MP3 → f32)
- Transport with play/pause/stop/seek/loop (tested)
- AudioGraph with StemPlayer (plays loaded files)
- 5 DSP effects (noise gate, compressor, overdrive, 4-band EQ, reverb)
- YIN pitch detector (real-time, tested)
- Practice comparator + session scorer
- MIDI reference loader
- egui UI with transport controls and waveform meter
- Passthrough demo with PipeWire device selection

**Stubbed / TODO:**
- IPC client (WorkerClient — socket connect, background reader)
- Python workers (demucs_worker.py, madmom_worker.py)
- ALSA backend (cpal streams)
- UI views (arrangement, piano roll, sidebar, effects editor)
- EffectDescriptor trait (self-describing effects for auto-generated UI)
- Effect registry
- Rhai scripted effects (Phase 2)
- Cue engine (Phase 2)
- Session persistence to SQLite (Phase 2)

## Audio device notes

The Volt 2's two inputs appear as separate PipeWire sources. When using the passthrough demo (cpal-based), select the specific source from the dropdown. When using the JACK backend, the input auto-connects to `system:capture_1`. To use input 2, manually route in a patchbay tool like `qpwgraph`, or set the default source before launching:

```bash
pactl set-default-source alsa_input.usb-Universal_Audio_Volt_2_21512037028264-00.HiFi__Mic2__source
```

## Documentation

| Document | Path | Purpose |
|----------|------|---------|
| PRD | `docs/intent/RIFFLAB-PRD-001.md` | Product requirements, module definitions, phasing |
| SRS | `docs/intent/RIFFLAB-SRS-001.md` | ISO 12207 compliant software requirements |
| IMP | `docs/intent/RIFFLAB-IMP-001.md` | Implementation plan (phases, milestones, tasks) |
| SAD | `docs/normative/RIFFLAB-SAD-001.md` | Software architecture description |
| SDD | `docs/normative/RIFFLAB-SDD-001.md` | Software design description |
| Backlog | `docs/intent/backlog/phase*/milestone*/` | Task files with frontmatter |
