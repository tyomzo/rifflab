# RiffLab — Product Requirements Document

**Document ID:** RIFFLAB-PRD-001
**Version:** 1.1.0
**Status:** Draft
**Author:** Artiom (with Claude)
**Date:** 2026-03-22

---

## 1. Purpose and Vision

RiffLab is a modular music practice, analysis, and performance workstation for Linux. It combines real-time audio effects processing, AI-powered stem separation, pitch/timing accuracy analysis, and an Ableton-inspired (but simplified) track-based UI into a single integrated environment.

The target user is a practicing musician who wants to load a track, separate it into stems, play along with real-time effects on their instrument, and get immediate visual feedback on pitch accuracy, timing, and note correctness — all without the complexity overhead of a full-featured DAW.

### 1.1 Design Principles

- **Modular architecture.** Every major capability (effects engine, stem separation, pitch analysis, UI) is an independent module with well-defined interfaces. Modules can be developed, tested, and replaced independently.
- **Practice-first workflow.** The default interaction model is: load a song → separate → pick a stem to practice → play along → see feedback. Production/recording features are secondary.
- **Simplified control surface.** Inspired by Ableton Live's Session/Arrangement views but stripped of production-oriented complexity. The UI should feel more like a smart practice tool than a DAW.
- **Real-time where it matters, offline where it doesn't.** Audio I/O, effects processing, and pitch detection must be real-time with sub-10ms latency. Stem separation, beat tracking, and reference transcription are offline batch operations that run once per song.
- **Linux and macOS native.** Primary targets are Linux (PipeWire/JACK + ALSA) and macOS (CoreAudio). No Wine dependencies, no Electron. Native UI toolkit rendered via wgpu (GPU-accelerated where available; software rasterization fallback acceptable). No custom GPU shaders required.

### 1.2 Technology Stack (Reference)

| Layer | Technology | Platform Notes |
|---|---|---|
| Audio engine | Rust + `cpal` (primary), `jack` crate (optional) | `cpal` wraps CoreAudio (macOS) and ALSA (Linux) natively. JACK backend optional on both platforms via JackOSX or PipeWire-JACK. |
| DSP | Rust + `fundsp`, `biquad`, `rustfft` | Pure Rust, fully portable |
| Stem separation | Python + Demucs (htdemucs_6s) | CUDA on Linux (Nvidia), MPS on macOS (Apple Silicon), CPU fallback on both |
| Beat tracking | Python + `madmom` | Pure Python + numpy, fully portable |
| Pitch detection | Rust (YIN implementation) | Pure Rust, fully portable |
| Sinusoidal analysis | Rust + `rustfft` | Pure Rust, fully portable |
| UI | Rust + `egui` (via `eframe` / `wgpu`) | Renders via wgpu (auto-selects Vulkan/OpenGL on Linux, Metal on macOS). Software rasterization fallback if no GPU. |
| IPC (Rust ↔ Python) | Unix sockets + MessagePack | Unix sockets supported on both Linux and macOS |
| Persistence | SQLite + WAV/FLAC on disk | Fully portable |

---

## 2. User Personas

### 2.1 Primary: Practicing Instrumentalist

A bass, guitar, or keyboard player who wants to learn songs by ear, practice along with isolated backing tracks, and measure their accuracy over time. They are not a producer — they want minimal setup friction and clear feedback.

### 2.2 Secondary: Live Performer

A musician performing with backing tracks who needs reliable, low-latency effects processing with automatic preset switching synchronized to song position. They need the cue system and effects engine but not necessarily the analysis features.

### 2.3 Tertiary: Tinkerer / Developer

A technically proficient user who wants to extend the system with custom scripted effects (Rhai), alternative ML models, or Rust DSP modules. User-created effects define their own parameters and appear in the UI effects editor with auto-generated controls.

---

## 3. Module Definitions

The system is composed of seven core modules. Each module has a defined responsibility, public API surface, and set of dependencies.

```
┌─────────────────────────────────────────────────────────────────────┐
│                          M7: UI Shell                               │
│                   (Track View, Piano Roll, Meters)                  │
├──────────┬──────────┬──────────┬──────────┬──────────┬──────────────┤
│ M1       │ M2       │ M3       │ M4       │ M5       │ M6           │
│ Audio    │ Effects  │ Stem     │ Analysis │ Practice │ Cue          │
│ Engine   │ Engine   │ Separator│ Engine   │ Engine   │ Engine       │
│          │          │          │          │          │              │
│ I/O,     │ DSP      │ Demucs   │ Pitch,   │ Compare, │ Bar/beat     │
│ routing, │ chain,   │ ML       │ onset,   │ scoring, │ sync,        │
│ mixing,  │ presets  │ pipeline │ sinusoid │ history  │ auto-switch  │
│ transport│          │          │ decomp   │          │              │
└──────────┴──────────┴──────────┴──────────┴──────────┴──────────────┘
```

### M1: Audio Engine

**Responsibility:** Hardware I/O, audio routing, transport (play/pause/seek), mix bus, buffer management.

**Key requirements:**

- Audio backend abstraction behind an `AudioBackend` trait:
  - **Linux:** JACK client (via `jack` crate) exposing configurable input/output ports. Falls back to `cpal` + ALSA if JACK/PipeWire is unavailable.
  - **macOS:** CoreAudio via `cpal` (primary). Optional JACK backend via JackOSX for users who want graph-level routing.
  - Backend selected at runtime based on platform and available services.
- Supports buffer sizes of 64, 128, 256, 512 samples at 44.1/48/96 kHz.
- Internal audio graph with named buses: `input` (from hardware), `stems[]` (from loaded song), `fx_return` (post-effects input signal), `master` (final mix to hardware output).
- Transport with sample-accurate position tracking, play/pause/stop/seek. Provides current position as both sample count and (bar, beat, tick) when beat grid is available.
- Zero-copy buffer passing between nodes in the audio graph. No heap allocations in the real-time thread.
- Metering: per-bus peak and RMS levels, exposed to UI via lock-free ring buffer.

**Public API (conceptual):**

```rust
trait AudioEngine {
    fn start(&mut self, config: AudioConfig) -> Result<()>;
    fn stop(&mut self);
    fn transport(&self) -> &Transport;
    fn load_stems(&mut self, stems: Vec<StemTrack>) -> Result<()>;
    fn set_input_route(&mut self, route: InputRoute);
    fn master_bus(&self) -> &Bus;
    fn meter_rx(&self) -> MeterReceiver;
}
```

**Dependencies:** None (foundational module).

### M2: Effects Engine

**Responsibility:** Real-time DSP processing chain on the instrument input signal. Preset management. Parameter automation.

**Key requirements:**

- Effect chain modeled as an ordered list of effect nodes. Each node is a trait object implementing a common `AudioProcessor` interface.
- Built-in effects (priority order):
  1. **Tuner** — always-available chromatic tuner (bypassed during play, or shown in UI)
  2. **Noise gate** — threshold, attack, release
  3. **Compressor** — threshold, ratio, attack, release, makeup gain
  4. **Overdrive / Distortion** — drive amount, tone, waveshaper type (tanh, hard clip, soft clip, tube model)
  5. **EQ** — parametric 4-band (low shelf, 2× peaking, high shelf) using biquad filters
  6. **Modulation** — chorus, phaser, flanger (LFO-modulated delay lines)
  7. **Delay** — tempo-synced or free, feedback, mix
  8. **Reverb** — room size, damping, wet/dry mix
  9. **Cabinet simulation** — convolution-based impulse response loader
- **Self-describing effects:** All effects (built-in and user-created) expose parameter metadata (name, range, unit, type) via a common `EffectDescriptor` interface. The UI auto-generates controls (sliders, toggles, dropdowns) from this metadata. No hardcoded UI per effect.
- **User-defined scripted effects (Phase 2):** Users can create custom effects by writing Rhai scripts that declare a name, parameter descriptors, and a `process` function. Scripted effects appear in the effect chain alongside built-in effects. Scripts are pre-compiled to AST at load time (no parsing in the audio thread). Sandbox with execution limits prevents runaway scripts.
- **Effect registry:** A catalog of all available effects (built-in + user scripts discovered from `library/effects/*.rhai`). The UI "Add Effect" dialog queries this registry.
- Presets: named snapshots of the full chain state (which effects are active, all parameter values). Stored as TOML/JSON.
- Parameter changes applied via crossfade (default 10ms) to avoid clicks.
- Plugin hosting (stretch goal): LV2 on Linux, Audio Units (AU) on macOS. Load external plugins as effect nodes.

**Public API (conceptual):**

```rust
trait EffectsEngine {
    fn chain(&self) -> &EffectChain;
    fn chain_mut(&mut self) -> &mut EffectChain;
    fn load_preset(&mut self, preset: &EffectPreset);
    fn save_preset(&self, name: &str) -> EffectPreset;
    fn crossfade_to_preset(&mut self, preset: &EffectPreset, fade_ms: f32);
}

trait AudioProcessor: Send {
    fn process(&mut self, buffer: &mut [f32], sample_rate: u32);
    fn set_param(&mut self, param: ParamId, value: f32);
    fn reset(&mut self);
}

/// Self-describing audio processor — extends AudioProcessor with metadata.
/// All effects (built-in and scripted) implement this.
trait EffectDescriptor: AudioProcessor {
    fn effect_type_id(&self) -> &str;  // e.g., "builtin:compressor", "script:my_fuzz"
    fn param_descriptors(&self) -> Vec<ParamDescriptor>;
    fn get_param(&self, param: ParamId) -> f32;
}

struct ParamDescriptor {
    id: ParamId,
    name: String,
    unit: String,       // "dB", "ms", "Hz", "%", ""
    min: f32,
    max: f32,
    default: f32,
    kind: ParamKind,    // Float | Int | Bool | Enum(Vec<String>)
}
```

**Dependencies:** M1 (receives audio buffers from audio engine input bus).

### M3: Stem Separator

**Responsibility:** Offline separation of a mixed audio file into instrument stems using pretrained ML models.

**Key requirements:**

- Primary model: Meta Demucs `htdemucs_6s` (6-stem: vocals, drums, bass, guitar, piano, other). Fallback: `htdemucs` (4-stem).
- Invoked as an out-of-process Python worker. The Rust host sends a job request (input file path, model selection, output directory) over Unix socket. The Python worker runs Demucs inference and writes stem WAV files to disk. Notifies completion via the same socket.
- GPU acceleration: CUDA on Linux (Nvidia), MPS on macOS (Apple Silicon). CPU fallback on both platforms.
- Progress reporting: the worker emits progress updates (percentage) that the UI can display.
- Results cached: once a song is separated, stems are stored alongside the song in the library. Re-separation only triggered if the user requests a different model or parameters.
- Configurable stem count: user can choose 4-stem or 6-stem separation.

**Public API (conceptual):**

```rust
trait StemSeparator {
    fn separate(&self, job: SeparationJob) -> JobHandle;
    fn poll_progress(&self, handle: &JobHandle) -> Progress;
    fn cancel(&self, handle: &JobHandle);
}

struct SeparationJob {
    input_path: PathBuf,
    output_dir: PathBuf,
    model: DemucsModel,  // HtDemucs4 | HtDemucs6 | HtDemucs4Ft
}
```

**Dependencies:** External Python runtime with `demucs`, `torch`, `torchaudio`.

### M4: Analysis Engine

**Responsibility:** Pitch detection, onset detection, beat/tempo tracking, sinusoidal decomposition, and note transcription. Operates both in real-time (on live input) and offline (on stem files).

**Key requirements:**

**Real-time pitch detection (live input):**
- YIN algorithm operating on the input buffer stream.
- Output: fundamental frequency (Hz), confidence (0.0–1.0), per analysis frame.
- Analysis hop size: 256 samples at 48kHz (~5.3ms per frame). Configurable.
- Latency budget: must complete within the audio callback period.
- Frequency range: 30Hz–2000Hz (covers bass and guitar fundamentals + harmonics).

**Offline pitch detection (on stems):**
- Same YIN algorithm but run as a batch process over a full stem file.
- Output: timestamped sequence of (time_seconds, frequency_hz, confidence) events.

**Onset detection:**
- Spectral flux-based onset detection on stem audio.
- Output: list of onset timestamps (seconds).

**Beat / tempo tracking:**
- Offline only. Delegates to Python `madmom` worker (same IPC pattern as M3).
- Output: BeatGrid — a list of beat timestamps with associated BPM and time signature.
- Supports variable tempo (tempo map) for songs with tempo changes.

**Sinusoidal decomposition (stretch goal — Phase 2):**
- STFT → peak picking → partial tracking (McAulay-Quatieri) → harmonic grouping → note segmentation.
- Output: list of `NoteEvent { pitch_hz, midi_note, onset, duration, amplitude_envelope, partials[] }`.
- Operates on separated stems (much easier than on full mix).

**Note transcription (simplified — Phase 1):**
- Combine pitch detection + onset detection into discrete note events.
- Output: `NoteEvent { midi_note, onset_time, duration, average_cents_deviation }`.
- This is a simpler approximation of the full sinusoidal decomposition that is usable immediately.

**Public API (conceptual):**

```rust
trait AnalysisEngine {
    // Real-time — called from audio thread
    fn detect_pitch_rt(&mut self, buffer: &[f32]) -> PitchFrame;

    // Offline — called on stem files
    fn transcribe_notes(&self, audio_path: &Path) -> Vec<NoteEvent>;
    fn detect_beats(&self, audio_path: &Path) -> JobHandle;  // async, Python worker
    fn analyze_sinusoidal(&self, audio_path: &Path) -> Vec<NoteEvent>;  // Phase 2
}

struct PitchFrame {
    frequency_hz: f32,
    confidence: f32,
    midi_note: u8,
    cents_deviation: f32,
}

struct NoteEvent {
    midi_note: u8,
    onset_seconds: f64,
    duration_seconds: f64,
    average_cents: f32,
    confidence: f32,
}
```

**Dependencies:** M1 (for real-time input buffer access). External Python runtime for beat tracking.

### M5: Practice Engine

**Responsibility:** Comparison of live performance against a reference. Scoring, accuracy metrics, and historical progress tracking.

**Key requirements:**

**Reference generation:**
- Takes the bass (or guitar/piano) stem from M3 and runs offline transcription via M4 to produce a reference note sequence.
- Alternatively, accepts a MIDI file as reference.
- Reference stored as a `Vec<NoteEvent>` associated with the song.

**Real-time comparison:**
- Receives live `PitchFrame` stream from M4 (real-time pitch detection).
- Receives current transport position from M1.
- At each frame, looks up the reference note active at the current song position.
- Computes:
  - **Pitch accuracy:** cents deviation from reference note. Bucketed as: perfect (±5¢), good (±15¢), acceptable (±25¢), off (>25¢).
  - **Timing accuracy:** onset time difference between played note and reference note. Bucketed as: tight (±20ms), good (±50ms), loose (±100ms), missed (>100ms or no note played).
  - **Note correctness:** binary — did the player hit the right note (within ±50¢ of reference)?
- Outputs a continuous stream of `ComparisonFrame` events for the UI.

**Scoring:**
- Per-note score: weighted combination of pitch accuracy, timing accuracy, and note correctness.
- Per-section score: aggregate over a user-defined range (e.g., "bars 12–16").
- Per-session score: aggregate over the full playthrough.
- Scoring weights configurable by user.

**History:**
- Each practice session stored in SQLite: song ID, timestamp, per-note results, aggregate scores.
- Progress visualization: accuracy over time for a given song, broken down by section.

**Public API (conceptual):**

```rust
trait PracticeEngine {
    fn set_reference(&mut self, notes: Vec<NoteEvent>);
    fn set_reference_from_midi(&mut self, midi_path: &Path) -> Result<()>;
    fn start_session(&mut self, song_id: SongId);
    fn feed_pitch_frame(&mut self, frame: PitchFrame, position: SongPosition);
    fn current_comparison(&self) -> ComparisonFrame;
    fn end_session(&mut self) -> SessionResult;
    fn history(&self, song_id: SongId) -> Vec<SessionResult>;
}

struct ComparisonFrame {
    reference_note: Option<NoteEvent>,
    played_pitch: PitchFrame,
    cents_deviation: f32,
    timing_offset_ms: f32,
    note_correct: bool,
    accuracy_bucket: AccuracyBucket,
}
```

**Dependencies:** M1 (transport position), M4 (real-time pitch detection, offline transcription).

### M6: Cue Engine

**Responsibility:** Timeline-based automation of effect preset switching, loop regions, and section markers, synchronized to song position.

**Key requirements:**

**Cue points:**
- A cue is a (position, action) pair. Position is expressed as (bar, beat) when a beat grid is available, or as absolute time (seconds) otherwise.
- Cue types:
  - `EffectPresetSwitch { preset_name }` — crossfade to a named effect preset.
  - `LoopRegion { start, end, repeat_count }` — loop a section N times (or indefinitely until user advances).
  - `SectionMarker { label }` — informational label (e.g., "Verse 1", "Chorus", "Bridge").
  - `TempoOverride { bpm }` — practice at a different tempo for a section (time-stretch the backing stems).
- Cues stored per-song in the library database.

**Execution:**
- The cue engine subscribes to transport position updates from M1.
- When transport crosses a cue point, the engine dispatches the appropriate action:
  - For `EffectPresetSwitch`: calls M2's `crossfade_to_preset()`.
  - For `LoopRegion`: instructs M1's transport to loop between start/end.
  - For `TempoOverride`: instructs M1 to adjust playback rate (with pitch correction).

**Editing:**
- Cues are created and edited in the UI's arrangement view (M7).
- Drag-and-drop cue placement on the timeline.
- Copy/paste cue sequences between songs.

**Public API (conceptual):**

```rust
trait CueEngine {
    fn load_cues(&mut self, cues: Vec<Cue>);
    fn add_cue(&mut self, cue: Cue);
    fn remove_cue(&mut self, cue_id: CueId);
    fn tick(&mut self, position: SongPosition);  // called per audio block
    fn active_cues_rx(&self) -> CueEventReceiver;
}

struct Cue {
    id: CueId,
    position: CuePosition,
    action: CueAction,
}

enum CuePosition {
    BarBeat { bar: u32, beat: u32 },
    AbsoluteTime { seconds: f64 },
}

enum CueAction {
    EffectPresetSwitch { preset: String },
    LoopRegion { start: CuePosition, end: CuePosition, repeats: Option<u32> },
    SectionMarker { label: String },
    TempoOverride { bpm: f64 },
}
```

**Dependencies:** M1 (transport position), M2 (effect preset switching), M4 (beat grid).

### M7: UI Shell

**Responsibility:** All visual presentation. Track-based arrangement view, piano roll, meters, controls. Communicates with all other modules but contains no business logic.

**Key requirements:**

**Layout — three primary panels:**

```
┌─────────────────────────────────────────────────────────────────┐
│  Toolbar: Transport controls, BPM, song selector, tuner        │
├───────────────┬─────────────────────────────────────────────────┤
│               │                                                 │
│   Sidebar     │          Main Canvas                            │
│               │                                                 │
│  - Track      │  Arrangement View (default)                     │
│    list       │  ┌─────────────────────────────────────────┐    │
│  - Solo/      │  │ Drums    ░░░▓▓▓░░░▓▓▓░░░▓▓▓░░░▓▓▓     │    │
│    mute       │  │ Bass     ▓▓▓░░░▓▓▓░░░▓▓▓░░░▓▓▓░░░     │    │
│  - Volume     │  │ Guitar   ░▓░▓░▓░▓░▓░▓░▓░▓░▓░▓░▓░▓     │    │
│    faders     │  │ Vocals   ▓░░░▓▓▓▓░░░░▓▓▓▓░░░░▓▓▓▓     │    │
│  - FX         │  │ Piano    ░░▓░░▓░░▓░░▓░░▓░░▓░░▓░░▓     │    │
│    chain      │  │ Other    ░░░░░░▓▓▓▓▓░░░░░░▓▓▓▓▓░░     │    │
│    toggle     │  │ ──────── ────────────────────────────── │    │
│               │  │ Your     ▓▓▓░▓▓░▓▓▓░▓▓░▓▓▓░▓▓░▓▓▓     │    │
│  - Preset     │  │ Input    (live waveform)                │    │
│    selector   │  │ ──────── ────────────────────────────── │    │
│               │  │ Cues     |V1   |Chorus |V2   |Chorus|  │    │
│               │  └─────────────────────────────────────────┘    │
│               │                                                 │
│               │  ┌──── Bottom Drawer ──────────────────────┐    │
│               │  │  Piano Roll / Accuracy / Effects Editor │    │
│               │  └─────────────────────────────────────────┘    │
├───────────────┴─────────────────────────────────────────────────┤
│  Status bar: Latency, CPU, pitch, accuracy, session score       │
└─────────────────────────────────────────────────────────────────┘
```

**Arrangement View (Main Canvas):**
- Horizontal timeline with beat grid overlay (vertical lines at each beat, bolder at each bar).
- One horizontal lane per stem + one lane for live input + one lane for cue markers.
- Each stem lane shows a waveform overview (rendered from the stem audio).
- Playhead (vertical line) moves with transport.
- Zoom and scroll (horizontal: time, vertical: track height).
- Click on timeline to seek.
- **Loop region selection:** Drag on the timeline to select a loop region. A highlighted loop bracket appears with draggable start/end handles. Handles can be repositioned independently to adjust the loop boundaries. Loop bracket snaps to beat grid when available. Loop toggle button in the toolbar enables/disables looping. When enabled, the transport wraps from the loop end back to the loop start.
- Cue lane shows color-coded markers for section labels, effect switches, and loop regions.
- Stem lanes have inline solo/mute/volume controls (appears on hover, like Ableton's minimal approach).

**Bottom Drawer — switchable views:**

*Piano Roll View:*
- Displays reference note events (from M5) as semi-transparent rectangles on a piano roll grid.
- Overlays the player's detected notes (from M4/M5) as solid rectangles, color-coded by accuracy.
- Color scheme: green (correct + in tune), yellow (correct + slightly off pitch), orange (correct + significantly off), red (wrong note), grey outline (missed note).
- Horizontal axis synchronized with the arrangement view timeline.
- Vertical axis: MIDI note number / pitch.

*Accuracy View:*
- Time-series plot of pitch deviation (cents) over time. Zero line = perfect pitch. Shows ±50¢ range.
- Second plot: timing deviation (ms) per note onset. Zero line = on the beat.
- Aggregate stats panel: % notes correct, average cents deviation, average timing offset, session score.
- Historical comparison: ghost line showing previous session's accuracy for A/B comparison.

*Effects Editor View:*
- Visual representation of the current effect chain (left-to-right signal flow).
- Click an effect node to expand its parameter controls, auto-generated from the effect's `param_descriptors()`: float params as sliders, bool params as toggles, enum params as dropdowns.
- "Add Effect" button opens a browser listing all available effects from the effect registry (built-in and user-created scripts).
- Drag to reorder effects in the chain. Remove button per effect.
- For scripted effects, an "Edit Script" action opens the `.rhai` file in the system's default text editor.
- Preset selector with save/load.
- Cue integration: "assign this preset to a cue point" button.

**Sidebar:**
- Track list with per-track: name, color, solo (S), mute (M), volume slider, pan knob.
- Input track at the bottom with an additional "FX" toggle to bypass the effects chain.
- Preset quick-switcher: dropdown of saved effect presets, click to switch.

**Toolbar:**
- Transport: play, pause, stop, record (for capturing practice sessions), loop toggle.
- BPM display (from beat tracking). Tap tempo button as fallback.
- Song selector / library browser.
- Global tuner indicator: shows current input pitch + note name + cents deviation. Always visible.
- Metronome toggle with volume control.

**Interaction simplifications (vs. Ableton):**
- No clip/session view — only arrangement.
- No MIDI editing (notes are detected, not authored).
- No mixer window — mixing controls are inline on track headers.
- No external plugin browser — effects are built-in effects and user-defined Rhai scripts from M2, browseable via the "Add Effect" dialog.
- No audio recording to tracks — only practice session capture for analysis.
- No automation lanes — automation is handled exclusively through the cue system.

**Dependencies:** M1 (transport, metering), M2 (effects state), M4 (real-time pitch), M5 (comparison stream), M6 (cue state).

---

## 4. Data Model

### 4.1 Song Library

```
library/
├── songs/
│   ├── {song_id}/
│   │   ├── original.wav          # Original mixed audio
│   │   ├── stems/
│   │   │   ├── vocals.wav
│   │   │   ├── drums.wav
│   │   │   ├── bass.wav
│   │   │   ├── guitar.wav
│   │   │   ├── piano.wav
│   │   │   └── other.wav
│   │   ├── analysis/
│   │   │   ├── beat_grid.json    # Beat timestamps, BPM, time sig
│   │   │   ├── bass_notes.json   # Transcribed reference notes
│   │   │   └── sections.json     # Auto-detected or user-defined sections
│   │   ├── cues.json             # Cue points for this song
│   │   └── metadata.json         # Title, artist, key, BPM, separation model used
│   └── ...
├── effects/
│   ├── my_fuzz.rhai            # User-defined effect scripts
│   ├── bass_enhancer.rhai
│   └── ...
├── presets/
│   ├── bass_clean.toml
│   ├── bass_drive.toml
│   └── ...
├── sessions/
│   └── {session_id}.json         # Practice session results
└── rifflab.db                    # SQLite: song index, session history, stats
```

### 4.2 Core Data Types

```rust
struct Song {
    id: SongId,
    title: String,
    artist: Option<String>,
    bpm: Option<f64>,
    key: Option<MusicalKey>,
    time_signature: (u8, u8),
    stems: Vec<StemInfo>,
    beat_grid: Option<BeatGrid>,
    cues: Vec<Cue>,
}

struct BeatGrid {
    beats: Vec<BeatMarker>,
}

struct BeatMarker {
    time_seconds: f64,
    bar: u32,
    beat: u32,
    bpm: f64,  // local BPM (supports tempo changes)
}

struct SessionResult {
    session_id: SessionId,
    song_id: SongId,
    timestamp: DateTime,
    duration_seconds: f64,
    notes_total: u32,
    notes_correct: u32,
    notes_missed: u32,
    notes_extra: u32,
    avg_cents_deviation: f32,
    avg_timing_offset_ms: f32,
    per_note_results: Vec<NoteResult>,
    score: f32,  // 0.0–100.0
}

struct NoteResult {
    reference: NoteEvent,
    played: Option<NoteEvent>,
    cents_deviation: f32,
    timing_offset_ms: f32,
    correct: bool,
}
```

---

## 5. Workflow Narratives

### 5.1 First-Time Song Import

1. User clicks "Import Song" and selects an audio file (WAV, FLAC, MP3).
2. File is copied to the library under a new `song_id` directory.
3. A separation job is dispatched to M3. UI shows a progress bar ("Separating stems…").
4. On completion, stems appear as tracks in the arrangement view.
5. A beat tracking job is dispatched to M4. On completion, the beat grid overlays the timeline.
6. A reference transcription job runs on the user-selected practice stem (e.g., bass). On completion, reference notes appear in the piano roll view.
7. Song metadata (title, BPM, key, stem paths) is persisted to the library.

### 5.2 Practice Session

1. User selects a song from the library. Stems and analysis data load instantly (cached).
2. User mutes the bass stem (they'll play it themselves).
3. User selects an effect preset for their bass.
4. User hits Play. Backing stems play through the master bus. The live input is processed through the effects chain and mixed in.
5. The real-time pitch detector (M4) feeds the practice engine (M5), which compares against the reference.
6. The piano roll in the bottom drawer shows reference notes scrolling past the playhead with the user's played notes overlaid in real time.
7. The status bar shows continuous pitch deviation and a running accuracy score.
8. If cues are defined, the effects engine auto-switches presets at the designated bar positions.
9. User can hit a section marker in the cue lane to loop a difficult passage.
10. On stop, the session result is saved to the database. The accuracy view shows a summary.

### 5.3 Setting Up Auto-Switch Cues

1. User plays through the song once to identify sections.
2. User pauses at the start of a new section (e.g., the chorus).
3. User right-clicks the cue lane at the playhead position → "Add Effect Cue."
4. User selects a preset from the dropdown (e.g., "Bass Drive" for the chorus).
5. A colored marker appears on the cue lane.
6. On next playback, the effects engine crossfades to "Bass Drive" when the playhead reaches that cue.
7. User adds more cues for verse (clean), bridge (reverb), etc.
8. Cues are saved with the song.

---

## 6. Phasing

### Phase 1 — Core Loop (MVP)

**Goal:** Load a song, separate stems, play along with effects, see real-time pitch accuracy.

| Module | Scope |
|---|---|
| M1: Audio Engine | JACK client, playback of stem WAVs, live input routing, basic transport (play/pause/stop/seek), loop regions, master mix |
| M2: Effects Engine | Linear chain with compressor, overdrive, EQ, reverb. Self-describing parameter metadata on all effects (`EffectDescriptor`). Effect registry (built-in only). Preset save/load. No crossfade switching yet |
| M3: Stem Separator | Demucs htdemucs (4-stem) via Python subprocess. Basic progress reporting |
| M4: Analysis Engine | Real-time YIN pitch detection on live input. Offline beat tracking via madmom. Simplified note transcription (pitch + onset detection) |
| M5: Practice Engine | Real-time comparison (pitch accuracy, note correctness). Per-session scoring. No history yet |
| M6: Cue Engine | Not included |
| M7: UI Shell | Arrangement view (stem waveforms + playhead), sidebar (solo/mute/volume per track), tuner display, basic piano roll with reference vs. played notes, status bar with accuracy |

**Deliverable:** A functional practice tool. User can import a song, separate it, mute a stem, play along, and see whether they're hitting the right notes with correct pitch.

### Phase 2 — Automation and Polish

**Goal:** Cue-based automation, historical tracking, 6-stem separation, improved UI.

| Module | Scope |
|---|---|
| M1 | Tempo override (playback rate adjustment) |
| M2 | Crossfade preset switching, additional effects (delay, chorus, cabinet sim). User-defined scripted effects (Rhai). Effect registry with script discovery from `library/effects/` |
| M3 | htdemucs_6s (6-stem) support, model selection UI |
| M4 | Full sinusoidal decomposition on stems (peak picking, partial tracking, harmonic grouping) |
| M5 | Session history in SQLite, progress-over-time visualization, per-section breakdown |
| M6 | Full cue engine: effect switches, loop regions, section markers, tempo overrides |
| M7 | Cue lane in arrangement view, accuracy history view, effects editor panel with auto-generated controls and effect browser, section markers |

### Phase 3 — Advanced Features

**Goal:** Deep editing, extended instrument support, possible LV2 plugin hosting.

| Module | Scope |
|---|---|
| M2 | Plugin hosting (LV2 on Linux, AU on macOS), convolution cabinet loader with IR import |
| M4 | Polyphonic pitch detection (for chords on guitar), MIDI export of transcribed notes |
| M5 | Difficulty curve analysis (identify hardest sections automatically), adaptive tempo (auto-slow difficult passages) |
| M6 | Tempo ramp cues (gradual tempo increase for progressive practice) |
| M7 | RipX-style note editor (edit individual notes in the sinusoidal representation), drag-to-reassign notes between stems |

---

## 7. Non-Functional Requirements

### 7.1 Latency

- **Input-to-output latency (effects chain):** ≤ 12ms at 48kHz with 256-sample buffers. Target ≤ 6ms with 128-sample buffers.
- **Pitch detection latency:** ≤ 10ms (one analysis frame behind).
- **UI update rate:** 30–60 FPS for meters and piano roll. Decoupled from audio thread.

### 7.2 Performance

- **Real-time audio thread:** zero heap allocations, no locks, no syscalls. All inter-thread communication via lock-free ring buffers or atomic operations.
- **Stem separation:** targets ≤ 2× real-time on a modern GPU (i.e., a 4-minute song separates in ≤ 8 minutes). CPU fallback may be slower.
- **Memory:** stem audio kept memory-mapped where possible. A 5-minute song with 6 stems at 48kHz/32-bit ≈ 330MB. This should be manageable on 16GB+ systems.

### 7.3 Platform

- **Primary:** Linux x86_64 (Ubuntu 22.04+, Pop!_OS, Fedora) and macOS (13 Ventura+, Apple Silicon and Intel).
- **Audio backends:**
  - Linux: JACK via PipeWire (primary), ALSA (fallback).
  - macOS: CoreAudio via `cpal` (primary), JackOSX (optional).
- **GPU acceleration for ML inference:**
  - Linux: Nvidia CUDA (optional). CPU fallback always available.
  - macOS: Apple MPS via PyTorch (optional on Apple Silicon). CPU fallback always available.
- **Display:**
  - Linux: Wayland and X11 support via egui/wgpu. GPU-accelerated where available; software rasterization fallback acceptable.
  - macOS: Native Cocoa windowing via egui/wgpu.
- **Build:** Single Rust codebase with `#[cfg(target_os)]` gating limited to the audio backend selection in M1. No platform-specific code in M2–M6. M7 relies on the UI toolkit's cross-platform abstraction.
- **Packaging:**
  - Linux: AppImage or Flatpak, with system Python for ML workers.
  - macOS: `.app` bundle via `cargo-bundle`, with bundled Python venv for ML workers.

### 7.4 Data Integrity

- Practice session data stored in SQLite with WAL mode for crash safety.
- Stem files are immutable once generated. Re-separation creates new files; old ones are cleaned up only on user request.

---

## 8. Open Questions

| # | Question | Impact | Status |
|---|---|---|---|
| 1 | UI toolkit choice: `egui` (immediate mode, simpler) vs. `iced` (Elm-like, more structured) vs. `slint` (declarative, commercial-friendly)? | M7 architecture | Open |
| 2 | Should the Python workers be long-lived daemons or spawned per-job? Daemon amortizes startup cost but adds lifecycle management complexity. | M3, M4 | Open |
| 3 | Should the practice engine support polyphonic instruments (guitar chords) in Phase 1, or defer to Phase 3? | M4, M5 scope | Deferred to Phase 3 |
| 4 | Is plugin hosting (LV2 on Linux, AU on macOS) worth the implementation cost, or should M2 focus on high-quality built-in effects? | M2 Phase 3 scope | Open |
| 5 | Should the cue system support OSC or MIDI CC output for controlling external hardware (pedals, lighting)? | M6 extensibility | Open |
| 6 | Licensing model: fully open source, or open core with proprietary UI/presets? | Distribution | Open |
| 7 | Project name: "RiffLab" is a working title. Final name TBD. | Branding | Open |
| 8 | Support for instruments beyond bass/guitar (e.g., vocals with lyrics tracking, drums with pattern analysis)? | M4, M5 scope | Deferred |
| 9 | Should scripted effects support per-buffer processing (script receives whole buffer) or per-sample only? Per-buffer is more efficient but more complex for script authors. | M2 scripted effect performance | Open |
| 10 | Should bundled example Rhai effect scripts ship with the application to help onboarding? | M2 user experience | Open |

---

## 9. Glossary

| Term | Definition |
|---|---|
| Stem | An isolated audio track for a single instrument or group, derived from a mixed recording via source separation |
| YIN | A fundamental frequency estimation algorithm based on autocorrelation, optimized for monophonic signals |
| STFT | Short-Time Fourier Transform — converts time-domain audio into a time-frequency representation (spectrogram) |
| Partial | A single sinusoidal component in the spectral decomposition of an audio signal |
| Cue | A time-positioned event on the song timeline that triggers an action (effect switch, loop, marker) |
| Beat grid | A sequence of beat timestamps aligned to the song's tempo, enabling bar/beat addressing |
| Crossfade | A smooth transition between two states (e.g., effect presets) over a short duration to avoid audible artifacts |
| xrun | An audio buffer underrun or overrun, causing an audible glitch. The primary failure mode to avoid in real-time audio |
| Lock-free | A concurrent programming pattern that avoids mutex locks, critical for real-time audio where blocking is unacceptable |
| Rhai | An embedded scripting language for Rust with a Rust-like syntax, sandboxing, and no external dependencies. Used for user-defined effects |
| Effect descriptor | Metadata exposed by an audio effect declaring its parameters (names, ranges, types), enabling auto-generated UI controls |
| Effect registry | A catalog of all available audio effects (built-in and user-created) that the UI and effect chain can query |
| wgpu | A Rust graphics API that auto-selects the best available GPU backend (Vulkan, OpenGL, Metal) and falls back to software rasterization |
