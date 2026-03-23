# RiffLab — Software Design Description (Linux)

**Document ID:** RIFFLAB-SDD-001
**Conformance:** ISO/IEC/IEEE 12207:2017 §6.4.5 — Design Definition
**Version:** 1.0.0
**Status:** Draft
**Platform Scope:** Linux x86_64
**Author:** Artiom (with Claude)
**Date:** 2026-03-23
**Source Documents:** RIFFLAB-SAD-001 v1.0.0, RIFFLAB-SRS-001 v1.1.0

---

## 1. rifflab-core — Shared Types and Traits

### 1.1 Audio Processing

**File:** `crates/rifflab-core/src/audio.rs`

```rust
// Configuration
enum SampleRate { Hz44100, Hz48000, Hz96000 }  // Default: Hz48000
enum BufferSize { B64, B128, B256, B512 }      // Default: B256
struct AudioConfig { sample_rate, buffer_size, input_channels: u16, output_channels: u16 }

// Core DSP trait — called from RT thread, MUST NOT allocate/lock/syscall
trait AudioProcessor: Send {
    fn process(&mut self, buffer: &mut [f32], sample_rate: u32);
    fn set_param(&mut self, param: ParamId, value: f32);
    fn reset(&mut self);
    fn name(&self) -> &str;
    fn is_bypassed(&self) -> bool;
}

// Self-describing extension (Phase 1) — called from UI thread
trait EffectDescriptor: AudioProcessor {
    fn effect_type_id(&self) -> &str;
    fn param_descriptors(&self) -> Vec<ParamDescriptor>;
    fn get_param(&self, param: ParamId) -> f32;
}

struct ParamDescriptor { id, name, unit, min, max, default, step: Option<f32>, kind: ParamKind }
enum ParamKind { Float, Int, Bool, Enum(Vec<String>) }
struct ParamId(pub u32);
struct ProcessContext { sample_rate, buffer_size, transport_position: u64, is_playing, bpm: Option<f64> }
```

### 1.2 Transport

**File:** `crates/rifflab-core/src/transport.rs`

```rust
enum TransportState { Stopped, Playing, Paused }
enum TransportCommand { Play, Pause, Stop, Seek(u64), SetLoop(Option<LoopRegion>) }
struct LoopRegion { start_frame: u64, end_frame: u64 }
struct SongPosition { frame: u64, sample_rate: u32 }
```

`SongPosition.seconds()` = frame / sample_rate. `bar_beat_tick(bpm, beats_per_bar)` converts to (bar, beat, tick) with 960 ticks per beat.

### 1.3 Analysis Types

**File:** `crates/rifflab-core/src/analysis.rs`

```rust
struct PitchFrame { frequency_hz: f32, confidence: f32, midi_note: u8, cents_deviation: f32 }
struct NoteEvent { midi_note: u8, onset_seconds: f64, duration_seconds: f64, average_cents: f32, confidence: f32 }
```

`PitchFrame.note_name()` returns e.g. "A4", "C#3". `NoteEvent.frequency_hz()` = 440 × 2^((midi-69)/12).

### 1.4 Practice Types

**File:** `crates/rifflab-core/src/practice.rs`

```rust
enum AccuracyBucket { Perfect(±5¢), Good(±15¢), Acceptable(±25¢), Off(>25¢) }
enum TimingBucket { Tight(±20ms), Good(±50ms), Loose(±100ms), Missed(>100ms) }
struct ComparisonFrame { reference_note, played_pitch, cents_deviation, timing_offset_ms, note_correct, accuracy_bucket, timing_bucket }
struct SessionResult { session_id, song_id, timestamp, duration, notes_total/correct/missed/extra, avg_cents, avg_timing, score: f32 }
```

### 1.5 Song and Stem Types

**File:** `crates/rifflab-core/src/song.rs`

```rust
type SongId = uuid::Uuid;
enum StemType { Vocals, Drums, Bass, Guitar, Piano, Other }
struct StemInfo { stem_type, path, sample_rate, channels, duration_seconds, num_frames }
struct Song { id, title, artist, bpm, key, time_signature: (u8,u8), stems, beat_grid }
struct BeatGrid { beats: Vec<BeatMarker> }
struct BeatMarker { time_seconds: f64, bar: u32, beat: u32, bpm: f64 }
```

`BeatGrid.bpm_at(time)` — reverse-scan for nearest beat before time. `BeatGrid.bar_beat_at(time)` — same, returns (bar, beat).

### 1.6 Metering

**File:** `crates/rifflab-core/src/metering.rs`

```rust
struct MeterData { peak_l, peak_r, rms_l, rms_r: f32 }
```

`from_interleaved(buffer)` — scans stereo pairs for peak/RMS. `from_mono(buffer)` — mono variant. `peak_db_*()` / `rms_db_*()` — convert to dBFS via `20 × log10(linear)`.

### 1.7 IPC Messages

**File:** `crates/rifflab-core/src/ipc.rs`

```rust
enum DemucsModel { HtDemucs(4-stem), HtDemucs6s(6-stem) }
enum WorkerRequest { Separate{input_path, output_dir, model}, DetectBeats{input_path, output_path}, Cancel{job_id}, Shutdown }
enum WorkerResponse { JobStarted{job_id}, Progress{job_id, percent, message}, JobCompleted{job_id, output_paths}, JobFailed{job_id, error}, JobCancelled{job_id} }
```

### 1.8 Presets

**File:** `crates/rifflab-core/src/preset.rs`

```rust
struct EffectPreset { name: String, effects: Vec<EffectState> }
struct EffectState { effect_type: String, active: bool, params: Vec<ParamValue> }
struct ParamValue { name, id: ParamId, value, min, max: f32 }
```

Serialized as TOML via `rifflab-fx::preset::{save_preset, load_preset}`.

---

## 2. rifflab-audio — Audio Engine (M1)

### 2.1 Backend Abstraction

**File:** `crates/rifflab-audio/src/backend/mod.rs`

Runtime selection: `create_backend()` tries JACK first (`JackBackend::new()`), falls back to ALSA (`AlsaBackend::new()`). Both are feature-gated (`jack-backend`, `alsa-backend`).

### 2.2 JACK Backend

**File:** `crates/rifflab-audio/src/backend/jack_backend.rs`

- Registers 1 input port (mono) + 2 output ports (stereo L/R)
- `JackProcess` implements `jack::ProcessHandler`
- In `process()`: interleaves mono input to stereo, calls `AudioCallback`, de-interleaves stereo output to L/R ports
- `Notifications` handles xrun and sample_rate_changed events
- Callback wrapped in `Arc<Mutex<AudioCallback>>` (JACK API constraint)

### 2.3 ALSA Backend

**File:** `crates/rifflab-audio/src/backend/alsa_backend.rs`

Stub — verifies default output device exists via cpal. Stream creation TODO.

### 2.4 Transport

**File:** `crates/rifflab-audio/src/transport.rs`

**Split design:**

| Component | Thread | Mechanism |
|-----------|--------|-----------|
| `Transport` | UI thread (owned by AudioEngine) | Sends commands via `rtrb::Producer<TransportCommand>` |
| `TransportRtHandle` | RT audio thread | Reads commands via `rtrb::Consumer`, updates `AtomicU8`/`AtomicU64` |

**Shared atomics (all Relaxed ordering):**
- `state: Arc<AtomicU8>` — 0=Stopped, 1=Playing, 2=Paused
- `position: Arc<AtomicU64>` — current frame
- `length: Arc<AtomicU64>` — total song length in frames
- `loop_start/loop_end: Arc<AtomicU64>` — loop boundaries
- `loop_enabled: Arc<AtomicBool>`

**`TransportRtHandle.advance(frames, commands)`** — the RT-thread update function:
1. Pop all pending `TransportCommand` from ring buffer
2. Apply state transitions (Play → state=1, Pause → state=2, Stop → state=0 + position=0, Seek → position=N)
3. If playing: position += frames
4. If loop enabled and position >= loop_end: position = loop_start
5. If position >= length and no loop: stop
6. Return `is_playing` bool

### 2.5 Audio Graph

**File:** `crates/rifflab-audio/src/graph/mod.rs`

**Fixed topology:**

```
StemPlayer[0] ──┐
StemPlayer[1] ──┤
StemPlayer[2] ──├──► mix_buffer ──► × master_volume ──► output
StemPlayer[3] ──┤           ▲
  ...          ──┘           │
                             │
Hardware Input ──► fx_chain.process() ──► × input_volume ──┘
```

**Key fields:**
- `stem_players: Vec<StemPlayer>` — one per loaded stem
- `stem_volumes/mutes/solos: Vec<f32/bool>` — per-stem control
- `fx_chain: Option<Box<dyn AudioProcessor>>` — injected from outside
- `mix_buffer: Vec<f32>` — pre-allocated, reused every callback
- `input_buffer: Vec<f32>` — pre-allocated, reused every callback

**`AudioGraph.process()` algorithm:**
1. Zero `mix_buffer`
2. Check if any stem has solo active → `any_solo` flag
3. For each stem: skip if muted or (any_solo && !this_stem_soloed); otherwise `fill_buffer()` additively into mix_buffer
4. Copy hardware input into `input_buffer`
5. Run `fx_chain.process()` on `input_buffer` (effects applied in-place)
6. Add `input_buffer × input_volume` into `mix_buffer`
7. Write `mix_buffer × master_volume` into `output`
8. Return `MeterData::from_interleaved(output)`

### 2.6 Stem Player

**File:** `crates/rifflab-audio/src/graph/node.rs`

- Audio data stored as `Arc<Vec<f32>>` (immutable, shared with UI for waveform rendering)
- Supports mono (duplicated to stereo) and stereo stems
- `fill_buffer()` syncs read position from `context.transport_position`, then reads samples at volume, mixing additively into output
- No allocation in hot path

### 2.7 Engine Orchestrator

**File:** `crates/rifflab-audio/src/engine.rs`

`AudioEngine::start()`:
1. Call `create_backend()` (JACK or ALSA)
2. Take `command_rx` from Transport (move to closure)
3. Construct callback closure capturing: graph (`Arc<Mutex>`), `TransportRtHandle`, `meter_tx`
4. Pass callback to `backend.start(config, callback)`

Callback body (per audio buffer):
1. `rt_handle.advance(frames, &mut command_rx)`
2. Construct `ProcessContext`
3. Zero output
4. Lock graph, call `graph.process(input, output, frames, context)`
5. Push `MeterData` to ring buffer

---

## 3. rifflab-fx — Effects Engine (M2)

### 3.1 Effect Chain

**File:** `crates/rifflab-fx/src/chain.rs`

`EffectChain` wraps `Vec<Box<dyn AudioProcessor>>`. Implements `AudioProcessor` itself (composable). `process()` iterates effects, calling each non-bypassed effect's `process()` in order.

### 3.2 Effect Algorithms

#### Noise Gate (`effects/noise_gate.rs`)

Envelope follower with asymmetric attack/release. Gates signal to zero when envelope < threshold.

```
Parameters: threshold (dBFS), attack_ms, release_ms
State: envelope (f32)

Per-sample:
  abs = |sample|
  if abs > envelope: envelope = attack_coeff × envelope + (1 - attack_coeff) × abs
  else:              envelope = release_coeff × envelope + (1 - release_coeff) × abs
  if envelope < threshold_linear: sample = 0
```

Where `attack_coeff = exp(-1 / (attack_ms × 0.001 × sample_rate))`.

#### Compressor (`effects/compressor.rs`)

Envelope follower + gain reduction above threshold.

```
Parameters: threshold (dBFS), ratio, attack_ms, release_ms, makeup_gain (dB)
State: envelope (f32)

Per-sample:
  Envelope follower (same as noise gate)
  if envelope > threshold_linear:
    over_dB = 20 × log10(envelope / threshold_linear)
    compressed_over = over_dB / ratio
    gain_reduction = over_dB - compressed_over
    gain = 10^(-gain_reduction / 20)
  else: gain = 1.0
  sample *= gain × makeup_linear
```

#### Overdrive (`effects/overdrive.rs`)

Waveshaper + one-pole low-pass tone control + dry/wet mix.

```
Parameters: drive (0–1), tone (0–1), mix (0–1), shaper_type (Tanh|HardClip|SoftClip)
State: lp_state (f32)

Per-sample:
  gain = 1 + drive × 30        (1× to 31×)
  driven = shaper(sample × gain)
  lp_state += tone × (driven - lp_state)   (one-pole LP)
  output = dry × (1 - mix) + lp_state × mix
```

Waveshapers:
- **Tanh:** `x.tanh()` — smooth saturation
- **HardClip:** `x.clamp(-1, 1)` — hard limiting
- **SoftClip:** `x - x³/3` for |x| < 1, else `±2/3` — polynomial approximation

#### Parametric EQ (`effects/eq.rs`)

4 cascaded biquad filter sections. Double-precision (f64) arithmetic for numerical stability.

```
Band layout:
  [0] Low shelf  @ 100 Hz, Q=0.707
  [1] Peaking    @ 500 Hz, Q=1.0
  [2] Peaking    @ 2000 Hz, Q=1.0
  [3] High shelf @ 8000 Hz, Q=0.707

Parameters per band: frequency, gain_db, Q
  Encoded as ParamId = band_idx × 3 + param_idx

Coefficient computation (lazy, dirty-flag):
  Low shelf:  Audio EQ Cookbook formulas (Robert Bristow-Johnson)
  Peaking:    Audio EQ Cookbook formulas
  High shelf: Audio EQ Cookbook formulas

Per-sample: cascaded direct form II biquad
  y = b0×x + b1×x1 + b2×x2 - a1×y1 - a2×y2
```

#### Reverb (`effects/reverb.rs`)

Schroeder reverb: 4 parallel comb filters → 2 series allpass filters.

```
Comb filter sizes (scaled from 44.1kHz reference):
  1116, 1188, 1277, 1356 samples

Allpass filter sizes:
  556, 441 samples

Parameters: room_size (0–1), damping (0–1), wet (0–1), dry (0–1)

Processing:
  feedback = 0.28 + room_size × 0.7
  comb_sum = Σ comb[i].process(input)
  output = allpass[0].process(allpass[1].process(comb_sum))
  final = input × dry + output × wet

Comb filter (with damping):
  output = buffer[index]
  damp_state = output × (1 - damp) + damp_state × damp
  buffer[index] = input + damp_state × feedback
  advance index

Allpass filter:
  buffered = buffer[index]
  output = -input + buffered
  buffer[index] = input + buffered × feedback
  advance index
```

All delay lines pre-allocated at construction time. Zero allocation in `process()`.

### 3.3 Preset I/O

**File:** `crates/rifflab-fx/src/preset.rs`

`save_preset()` — serialize `EffectPreset` to TOML string, write to file.
`load_preset()` — read file, deserialize from TOML.

---

## 4. rifflab-ipc — Worker Communication

### 4.1 Protocol

**File:** `crates/rifflab-ipc/src/protocol.rs`

Frame format: `[4-byte big-endian u32 length][MessagePack payload]`

- `encode_request(req: &WorkerRequest) -> Vec<u8>` — serialize to MessagePack, prepend length
- `decode_response(data: &[u8]) -> WorkerResponse` — deserialize MessagePack payload

### 4.2 Worker Lifecycle

**File:** `crates/rifflab-ipc/src/lifecycle.rs`

`spawn_worker(script_path, socket_path)` — executes `python3 {script} --socket {socket_path}`.

### 4.3 Client (stub)

**File:** `crates/rifflab-ipc/src/client.rs`

`WorkerClient` — holds socket_path and worker_script. Methods `connect()`, `submit()`, `poll()` are stubbed. Will use tokio async socket I/O with a background reader task.

---

## 5. rifflab-analysis — Analysis Engine (M4)

### 5.1 YIN Pitch Detection Algorithm

**File:** `crates/rifflab-analysis/src/pitch/yin.rs`

**Design:** Maintains a circular buffer (size = 2 × max_period for 30Hz) to handle audio callback boundaries. Processes incrementally — each `detect()` call writes new samples and runs detection on the full buffer.

**Algorithm (5 steps):**

1. **Difference function:** For each lag τ from min_tau to max_tau:
   `diff[τ] = Σ(buffer[i] - buffer[i+τ])²` over half the buffer

2. **Cumulative Mean Normalized Difference (CMND):**
   `cmnd[0] = 1.0`
   `cmnd[τ] = diff[τ] × τ / Σ(diff[1..τ])`

3. **Absolute threshold:** Find first τ where cmnd[τ] < 0.15, then track to local minimum.

4. **Parabolic interpolation:** Refine τ using 3-point parabolic fit:
   `τ_refined = τ + (s0 - s2) / (2 × (2×s1 - s2 - s0))`

5. **Frequency conversion:** `freq = sample_rate / τ_refined`
   Then `midi = 69 + 12 × log2(freq/440)`, `cents = (midi_float - round(midi_float)) × 100`

**Parameters:**
- `threshold = 0.15` (YIN confidence threshold)
- `min_freq = 30 Hz`, `max_freq = 2000 Hz`
- Buffer size: `2 × ceil(sample_rate / 30)` ≈ 3200 samples at 48kHz

**RT safety note:** The current implementation allocates `diff` and `cmnd` vectors per call. Should be pre-allocated for production RT use.

### 5.2 Onset Detection (stub)

**File:** `crates/rifflab-analysis/src/onset.rs`

Planned: spectral flux onset detection using rustfft. Not yet implemented.

### 5.3 Beat Tracking

**File:** `crates/rifflab-analysis/src/beats.rs`

Delegates to Python madmom worker via `rifflab-ipc`. Submits `DetectBeats` request with input/output paths. Worker writes `beat_grid.json`.

---

## 6. rifflab-practice — Practice Engine (M5)

### 6.1 Comparator

**File:** `crates/rifflab-practice/src/compare.rs`

**Algorithm:**
1. `find_active_note(time)` — linear scan through reference notes, find one where `onset ≤ time < onset + duration`
2. If reference note found: `cents = 1200 × log2(played_freq / reference_freq)`
3. `note_correct = |cents| < 50`
4. `accuracy_bucket = AccuracyBucket::from_cents(cents)`
5. Timing offset computation is currently stubbed (always 0.0)

### 6.2 Session Scorer

**File:** `crates/rifflab-practice/src/scoring.rs`

Accumulates `ComparisonFrame` values. Score formula:

```
correctness = notes_correct / notes_total
avg_cents = total_cents_deviation / notes_total
pitch_score = max(0, 1 - min(avg_cents / 50, 1))
score = (correctness × 0.5 + pitch_score × 0.5) × 100
```

Range: 0.0–100.0. Weights configurable in future.

### 6.3 Reference Loading

**File:** `crates/rifflab-practice/src/reference.rs`

- `load_from_json(path)` — deserialize `Vec<NoteEvent>` from JSON
- `load_from_midi(path)` — parse MIDI via `midly` crate:
  - Reads all tracks, handles NoteOn/NoteOff events
  - Tracks active notes per key (supports polyphony)
  - Converts MIDI ticks → seconds using tempo map (default 120 BPM)
  - Sorts by onset_seconds

---

## 7. rifflab-ui — UI Shell (M7)

### 7.1 Application Structure

**File:** `crates/rifflab-ui/src/app.rs`

`RiffLabApp` implements `eframe::App`. `update()` renders per frame:

| Panel | Position | Content |
|-------|----------|---------|
| Toolbar | Top | "RiffLab" heading, Play/Pause/Stop buttons |
| Status Bar | Bottom | Latency, CPU, Pitch, Score labels |
| Sidebar | Left (200px, resizable) | "Tracks" heading, placeholder |
| Central | Remaining | "Arrangement View" placeholder |

`run()` launches `eframe::run_native()` with 1280×800 window.

### 7.2 View Modules (all stubs, Phase 1 Step 8)

| View | File | Purpose |
|------|------|---------|
| arrangement | `views/arrangement.rs` | Timeline, waveforms, playhead, loop handles, cue lane |
| piano_roll | `views/piano_roll.rs` | Reference vs. played notes, color-coded |
| sidebar | `views/sidebar.rs` | Track list, solo/mute/volume |
| toolbar | `views/toolbar.rs` | Transport, BPM, tuner, song selector |
| status_bar | `views/status_bar.rs` | Latency, CPU, pitch, accuracy, score |

### 7.3 Widget Modules (all stubs, Phase 1 Step 8)

| Widget | File | Purpose |
|--------|------|---------|
| waveform | `widgets/waveform.rs` | Render audio waveform overview |
| meter | `widgets/meter.rs` | VU/peak meter bar |
| knob | `widgets/knob.rs` | Rotary parameter control |
| fader | `widgets/fader.rs` | Linear volume control |

---

## 8. Database Schema

**File:** `library/rifflab.db` (SQLite, WAL mode)

```sql
CREATE TABLE songs (
    id          TEXT PRIMARY KEY,    -- UUID
    title       TEXT NOT NULL,
    artist      TEXT,
    bpm         REAL,
    key_root    TEXT,
    key_quality TEXT,                -- 'Major' | 'Minor'
    time_sig_n  INTEGER DEFAULT 4,
    time_sig_d  INTEGER DEFAULT 4,
    stem_model  TEXT,                -- 'htdemucs' | 'htdemucs_6s'
    imported_at TEXT NOT NULL        -- ISO 8601
);

CREATE TABLE sessions (
    id                  TEXT PRIMARY KEY,    -- UUID
    song_id             TEXT NOT NULL REFERENCES songs(id),
    timestamp           TEXT NOT NULL,       -- ISO 8601
    duration_seconds    REAL NOT NULL,
    notes_total         INTEGER NOT NULL,
    notes_correct       INTEGER NOT NULL,
    notes_missed        INTEGER NOT NULL,
    notes_extra         INTEGER NOT NULL,
    avg_cents_deviation REAL NOT NULL,
    avg_timing_offset   REAL NOT NULL,
    score               REAL NOT NULL        -- 0.0–100.0
);

CREATE TABLE note_results (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id          TEXT NOT NULL REFERENCES sessions(id),
    ref_midi_note       INTEGER NOT NULL,
    ref_onset_seconds   REAL NOT NULL,
    ref_duration        REAL NOT NULL,
    played_midi_note    INTEGER,             -- NULL if missed
    played_onset        REAL,
    cents_deviation     REAL NOT NULL,
    timing_offset_ms    REAL NOT NULL,
    correct             INTEGER NOT NULL     -- 0 or 1
);

CREATE INDEX idx_sessions_song ON sessions(song_id);
CREATE INDEX idx_note_results_session ON note_results(session_id);
```

---

## 9. IPC Protocol Specification

### 9.1 Framing

```
┌──────────────────┬─────────────────────────────┐
│ Length (4 bytes)  │ MessagePack Payload          │
│ Big-endian u32   │ (WorkerRequest or Response)  │
└──────────────────┴─────────────────────────────┘
```

### 9.2 Worker Lifecycle

```
1. Rust spawns: python3 {script} --socket /run/user/{uid}/rifflab-{type}.sock
2. Python worker creates Unix socket listener, accepts connection
3. Rust connects via tokio::net::UnixStream
4. Request/response loop over socket
5. On shutdown: Rust sends WorkerRequest::Shutdown, waits for process exit
6. On crash: Rust detects BrokenPipe, cleans up socket, respawns on next request
```

### 9.3 Worker Types

| Worker | Script | Model | Socket Name |
|--------|--------|-------|-------------|
| Stem separation | `workers/demucs_worker.py` | htdemucs / htdemucs_6s | `rifflab-demucs.sock` |
| Beat tracking | `workers/madmom_worker.py` | madmom BeatTracker | `rifflab-madmom.sock` |

---

## 10. Configuration

### 10.1 Application Config

**File:** `~/.config/rifflab/config.toml`

```toml
[audio]
backend = "auto"            # "jack", "alsa", or "auto"
sample_rate = 48000
buffer_size = 256
input_channels = 2
output_channels = 2

[library]
path = "~/.local/share/rifflab"

[ui]
theme = "dark"
fps_target = 60

[workers]
demucs_script = "workers/demucs_worker.py"
madmom_script = "workers/madmom_worker.py"
```

### 10.2 Audio Config Defaults

| Parameter | Default | Valid Values |
|-----------|---------|-------------|
| sample_rate | 48000 | 44100, 48000, 96000 |
| buffer_size | 256 | 64, 128, 256, 512 |
| input_channels | 2 | 1, 2 |
| output_channels | 2 | 2 |

---

## 11. Library Directory Layout

```
~/.local/share/rifflab/
├── songs/
│   └── {uuid}/
│       ├── original.wav                # Imported audio file
│       ├── stems/
│       │   ├── vocals.wav              # Demucs output (44.1/48kHz, 32-bit float)
│       │   ├── drums.wav
│       │   ├── bass.wav
│       │   ├── guitar.wav              # 6-stem only
│       │   ├── piano.wav               # 6-stem only
│       │   └── other.wav
│       ├── analysis/
│       │   ├── beat_grid.json          # BeatGrid { beats: [BeatMarker] }
│       │   ├── bass_notes.json         # Vec<NoteEvent>
│       │   └── sections.json           # Auto-detected song sections
│       ├── cues.json                   # Vec<Cue> (Phase 2)
│       └── metadata.json              # Song { title, artist, bpm, key, ... }
├── effects/
│   ├── my_fuzz.rhai                    # User-defined Rhai effect scripts
│   └── ...
├── presets/
│   ├── bass_clean.toml                 # EffectPreset serialized as TOML
│   ├── bass_drive.toml
│   └── ...
├── sessions/
│   └── {uuid}.json                     # SessionResult (backup, primary in SQLite)
└── rifflab.db                          # SQLite database (WAL mode)
```

**File formats:**

| File | Format | Schema |
|------|--------|--------|
| `*.wav` | WAV PCM 32-bit float | Standard RIFF/WAV |
| `metadata.json` | JSON | `Song` struct (serde) |
| `beat_grid.json` | JSON | `BeatGrid` struct (serde) |
| `bass_notes.json` | JSON | `Vec<NoteEvent>` (serde) |
| `cues.json` | JSON | `Vec<Cue>` (serde) |
| `*.toml` | TOML | `EffectPreset` struct (serde) |
| `*.rhai` | Rhai script | Must define `fn name()`, `fn params()`, `fn process()` |
| `rifflab.db` | SQLite 3 | Schema in §8 |

---

## 12. Error Handling Strategy

| Context | Strategy | Implementation |
|---------|----------|---------------|
| RT audio thread | Never panic; log and continue. Drop meter/comparison frames if ring buffer full. Bypass failed effects. | `rtrb::push()` returns error on full → ignored |
| Backend init | Fallback chain: JACK → ALSA → error to user | `create_backend()` in `backend/mod.rs` |
| Python worker crash | Detect broken pipe, mark jobs as failed, respawn on next request | `WorkerClient` error handling (TODO) |
| File I/O | `anyhow::Result` propagated to UI for user-facing error messages | All file operations return `Result` |
| Preset load failure | Log warning, use default empty chain | `load_preset()` returns `Result` |
| Rhai script error | Compile error at load time → reject script. Runtime error → bypass effect, log warning | `rhai::Engine` error handling (Phase 2) |

---

## 13. Traceability to Architecture

| Design Element | Architecture Decision | Requirements |
|----------------|----------------------|-------------|
| Pre-allocated mix/input buffers in AudioGraph | AD-1 (fixed graph), AD-7 (heap pre-load) | FR-M1-07, NFR-PERF-03, CPM-5 |
| Transport atomics (Relaxed ordering) | AD-10 | FR-M1-05, NFR-PERF-03 |
| rtrb ring buffers for meter/command channels | AD-2 (lock-free comms) | FR-M1-08, NFR-PERF-04 |
| EffectChain as Vec<Box<dyn AudioProcessor>> | AD-8 (trait interfaces) | FR-M2-01, SR-10 |
| Biquad EQ in f64 | Numerical stability for cascaded filters | FR-M2-02 |
| Schroeder reverb with pre-allocated delay lines | AD-1, AD-7 | FR-M2-02, NFR-PERF-03 |
| YIN with circular buffer | Real-time pitch detection across callback boundaries | FR-M4-01, FR-M4-04, CPM-2 |
| Length-prefixed MessagePack IPC | AD-3 (out-of-process Python) | FR-M3-01, IC-06 |
| SQLite WAL mode | Crash-safe session persistence | NFR-REL-01 |
| egui immediate-mode UI | AD-4 | FR-M7-01, IC-08 |
