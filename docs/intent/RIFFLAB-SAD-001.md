# RiffLab — Software Architecture Description (Linux)

**Document ID:** RIFFLAB-SAD-001
**Conformance:** ISO/IEC/IEEE 12207:2017 §6.4.4 — Architecture Definition
**Version:** 1.0.0
**Status:** Draft
**Platform Scope:** Linux x86_64
**Author:** Artiom (with Claude)
**Date:** 2026-03-23
**Source Documents:** RIFFLAB-PRD-001 v1.1.0, RIFFLAB-SRS-001 v1.1.0

---

## 1. Stakeholder Concerns Addressed by Architecture

| Stakeholder | Concern | Architectural Response |
|-------------|---------|----------------------|
| SK-1 Practicing Instrumentalist | Low latency, simple workflow | Lock-free RT audio path (≤12ms); fixed audio graph topology; practice-first UI layout |
| SK-2 Live Performer | Reliable effects switching, no dropouts | Cue engine dispatches within one audio buffer; crossfade transitions; zero-alloc RT thread |
| SK-3 Tinkerer / Developer | Extensibility, custom effects | Trait-based interfaces (`AudioProcessor`, `EffectDescriptor`); Rhai scripting; modular crate boundaries |
| SK-4 System Administrator | Deployment, audio config | Feature-gated backends; AppImage/Flatpak packaging; XDG-compliant config |

---

## 2. Context View

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              Linux Host                                     │
│                                                                             │
│   ┌─────────────┐     ┌──────────────────────────────────────────────┐     │
│   │ PipeWire /  │◄───►│              RiffLab Process                  │     │
│   │ JACK / ALSA │     │                                              │     │
│   │ (Audio I/O) │     │  ┌──────────┐  ┌──────────┐  ┌──────────┐  │     │
│   └─────────────┘     │  │ RT Audio │  │ UI Thread│  │ Tokio RT │  │     │
│                       │  │ Thread   │  │ (egui)   │  │ (IPC)    │  │     │
│   ┌─────────────┐     │  └──────────┘  └──────────┘  └──────────┘  │     │
│   │ Wayland /   │◄────│         ▲              ▲            │        │     │
│   │ X11 Display │     │         │ rtrb         │ rtrb       │        │     │
│   └─────────────┘     │         ▼              ▼            ▼        │     │
│                       └──────────────────────────────────────────────┘     │
│   ┌─────────────┐          ▲                                    │          │
│   │ Filesystem  │◄────────►│ (songs, stems, presets, DB)        │          │
│   │ (Library)   │          │                                    │          │
│   └─────────────┘          │                           Unix Socket         │
│                            │                                    │          │
│   ┌─────────────┐     ┌────┴──────────────────┐    ┌───────────▼────────┐ │
│   │ NVIDIA GPU  │◄────│ Python: Demucs Worker │    │ Python: madmom     │ │
│   │ (CUDA, opt) │     │ (stem separation)     │    │ Worker (beats)     │ │
│   └─────────────┘     └───────────────────────┘    └────────────────────┘ │
└─────────────────────────────────────────────────────────────────────────────┘
```

**External entities:**

| Entity | Interface | Protocol |
|--------|-----------|----------|
| PipeWire / JACK | `jack` crate (JACK client API) | JACK audio ports (float buffers per callback) |
| ALSA | `cpal` crate | ALSA PCM stream callback |
| Wayland / X11 | `wgpu` via `eframe`/`winit` | GPU surface or software rasterization |
| Filesystem | POSIX I/O | WAV/FLAC files, JSON/TOML config, SQLite DB |
| Python workers | Unix domain socket | Length-prefixed MessagePack frames |
| NVIDIA GPU | CUDA via PyTorch (inside Python worker) | Tensor compute (stem separation inference) |

---

## 3. Module Decomposition View

### 3.1 Crate Structure

```
rifflab-core          ← Shared types, traits, ring buffer re-export (no rifflab deps)
    ↑
    ├── rifflab-audio  ← M1: Audio engine (JACK/ALSA backends, graph, transport)
    ├── rifflab-fx     ← M2: Effects engine (DSP chain, processors, presets)
    ├── rifflab-ipc    ← IPC client library (Unix socket + MessagePack)
    │       ↑
    │       ├── rifflab-stems    ← M3: Stem separator (Rust client for Demucs)
    │       └── rifflab-analysis ← M4: Pitch detection, onset, beat tracking
    ├── rifflab-practice         ← M5: Live comparison, scoring
    └── rifflab-ui               ← M7: egui shell (all visual presentation)
            ↑
        rifflab-app              ← Binary entry point (wires everything)
```

### 3.2 Module Responsibilities

| Crate | Module | Responsibility | Key Trait / Type |
|-------|--------|---------------|-----------------|
| rifflab-core | — | Shared types, `AudioProcessor` trait, lock-free primitives | `AudioProcessor`, `EffectDescriptor`, `PitchFrame`, `NoteEvent` |
| rifflab-audio | M1 | Hardware I/O, audio graph, transport, metering | `AudioBackend`, `AudioEngine`, `Transport` |
| rifflab-fx | M2 | DSP effect chain, built-in effects, presets | `EffectChain`, `NoiseGate`, `Compressor`, `Overdrive`, `ParametricEq`, `Reverb` |
| rifflab-ipc | — | Python worker lifecycle and communication | `WorkerClient`, `JobHandle` |
| rifflab-stems | M3 | Stem separation orchestration and caching | `StemSeparator` |
| rifflab-analysis | M4 | Pitch detection (YIN), onset, beat tracking | `YinDetector`, `BeatTracker` |
| rifflab-practice | M5 | Real-time comparison, scoring, reference loading | `Comparator`, `SessionScorer` |
| rifflab-ui | M7 | All visual presentation (egui) | `RiffLabApp` |
| rifflab-app | — | Binary: CLI, config, module wiring | `main()` |

M6 (Cue Engine) is not yet a separate crate — deferred to Phase 2.

### 3.3 Dependency Rules

1. **No circular dependencies.** Dependency flows strictly downward from app → modules → core.
2. **rifflab-core has zero rifflab dependencies.** It is the leaf of the dependency tree.
3. **rifflab-audio does NOT depend on rifflab-fx.** The effects chain is injected as a `Box<dyn AudioProcessor>` at the app wiring level.
4. **rifflab-ui depends on all module crates** (read access to state for rendering), but modules do not depend on UI.
5. **Only rifflab-stems and rifflab-analysis depend on rifflab-ipc** (the two modules that talk to Python workers).

---

## 4. Concurrency View

### 4.1 Execution Contexts

```
┌─────────────────────────────────────────────────────────────────┐
│                        RiffLab Process                           │
│                                                                  │
│  ┌──────────────────┐   ┌──────────────────┐   ┌─────────────┐ │
│  │  RT Audio Thread  │   │    UI Thread     │   │ Tokio RT    │ │
│  │  (JACK callback)  │   │    (egui)        │   │ (IPC I/O)   │ │
│  │                    │   │                  │   │             │ │
│  │  • Transport.      │   │  • RiffLabApp    │   │ • WorkerCli │ │
│  │    advance()       │   │    .update()     │   │   ent       │ │
│  │  • AudioGraph.     │   │  • Read meters   │   │ • Socket    │ │
│  │    process()       │   │  • Read pitch    │   │   read/write│ │
│  │  • YinDetector.    │   │  • Send commands │   │ • Worker    │ │
│  │    detect()        │   │  • Render UI     │   │   spawn     │ │
│  │                    │   │                  │   │             │ │
│  │ CONSTRAINTS:       │   │ CONSTRAINTS:     │   │             │ │
│  │ • Zero alloc       │   │ • 30-60 FPS      │   │             │ │
│  │ • No locks         │   │ • Non-blocking   │   │             │ │
│  │ • No syscalls      │   │   reads from     │   │             │ │
│  │ • Deterministic    │   │   ring buffers   │   │             │ │
│  └──────────────────┘   └──────────────────┘   └─────────────┘ │
│           ▲                      ▲                     │         │
│           │    rtrb (lock-free)  │                     │         │
│           ▼                      ▼                     ▼         │
│     ┌──────────────────────────────────┐    ┌────────────────┐  │
│     │  Ring Buffers & Atomics          │    │ Unix Sockets   │  │
│     │  • TransportCommand (UI→RT)      │    │ (to Python     │  │
│     │  • MeterData (RT→UI)             │    │  workers)      │  │
│     │  • Atomic: volume, mute, solo    │    │                │  │
│     │  • Atomic: transport state/pos   │    │                │  │
│     └──────────────────────────────────┘    └────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
         │                                           │
         ▼                                           ▼
┌──────────────────┐                    ┌──────────────────────┐
│ JACK / ALSA      │                    │ Python Worker Procs  │
│ (hardware audio) │                    │ (Demucs, madmom)     │
└──────────────────┘                    └──────────────────────┘
```

### 4.2 Communication Channels

| Channel | Direction | Mechanism | Capacity | Data Type |
|---------|-----------|-----------|----------|-----------|
| Transport commands | UI → RT | `rtrb` SPSC ring buffer | 64 entries | `TransportCommand` |
| Meter data | RT → UI | `rtrb` SPSC ring buffer | 1024 entries | `MeterData` |
| Transport state | RT ↔ UI | `AtomicU8` | 1 value | 0=Stopped, 1=Playing, 2=Paused |
| Transport position | RT → UI | `AtomicU64` | 1 value | Frame count |
| Stem volume/mute/solo | UI → RT | `f32`/`bool` fields (graph locked) | per-stem | Direct field access |
| Worker requests | Tokio → Python | Unix socket + MessagePack | Stream | `WorkerRequest` |
| Worker responses | Python → Tokio | Unix socket + MessagePack | Stream | `WorkerResponse` |

### 4.3 Lock Analysis

| Lock | Location | Contention Risk | Mitigation |
|------|----------|----------------|------------|
| `Arc<Mutex<AudioGraph>>` | `engine.rs` | RT thread vs UI thread (stem loading, FX chain swap) | Lock held briefly; UI loads stems before playback; future: double-buffer swap |
| `Arc<Mutex<AudioCallback>>` | `jack_backend.rs` | JACK process handler requires `&mut self` | Unavoidable JACK API constraint; callback is short |
| None | Transport | — | All atomics with Relaxed ordering |
| None | Metering | — | Lock-free ring buffer |

---

## 5. Data Flow Views

### 5.1 Real-Time Audio Path (per callback, ≤5.3ms @ 256 samples/48kHz)

```
JACK/ALSA Hardware Input
    │
    ▼
┌─ AudioEngine.callback() ──────────────────────────────────────────┐
│                                                                    │
│  1. TransportRtHandle.advance(frames, commands)                   │
│     ├─ Pop all TransportCommands from ring buffer                 │
│     ├─ Update state/position atomics                              │
│     └─ Handle loop wraparound                                     │
│                                                                    │
│  2. Construct ProcessContext {sample_rate, position, is_playing}   │
│                                                                    │
│  3. AudioGraph.process(input, output, frames, context)            │
│     │                                                              │
│     ├─ Zero mix_buffer (pre-allocated)                            │
│     │                                                              │
│     ├─ For each StemPlayer (not muted, or soloed):                │
│     │   └─ fill_buffer() → read from Arc<Vec<f32>> at position   │
│     │      └─ Mix additively into mix_buffer                      │
│     │                                                              │
│     ├─ Copy input → input_buffer (pre-allocated)                  │
│     ├─ fx_chain.process(input_buffer) → effects in series         │
│     ├─ Mix input_buffer into mix_buffer                           │
│     │                                                              │
│     ├─ Apply master_volume → write to output                      │
│     └─ MeterData::from_interleaved(output)                        │
│                                                                    │
│  4. meter_tx.push(meter_data)  [ring buffer, non-blocking]        │
│                                                                    │
└─ Return output to JACK/ALSA ──────────────────────────────────────┘
```

**Zero-allocation guarantees:**
- `mix_buffer` and `input_buffer` pre-allocated in `AudioGraph::new()`
- `StemPlayer` reads from `Arc<Vec<f32>>` (no clone, no alloc)
- Ring buffer push is non-blocking (drops if full)
- No `String`, `Vec`, or `Box` created in this path

### 5.2 Offline Analysis Path (async, non-RT)

```
User imports song file
    │
    ▼
rifflab-app: copy file to library/{song_id}/original.wav
    │
    ├──► rifflab-stems: StemSeparator.separate()
    │        │
    │        ▼
    │    rifflab-ipc: WorkerClient.submit(Separate{...})
    │        │
    │        ▼
    │    Unix Socket ──► demucs_worker.py
    │        │               │
    │        │               ├─ Load htdemucs model (CUDA/CPU)
    │        │               ├─ Run inference
    │        │               ├─ Emit Progress{percent} ──► UI progress bar
    │        │               └─ Write stems/*.wav
    │        │
    │        ▼
    │    JobCompleted{output_paths}
    │
    ├──► rifflab-analysis: BeatTracker.detect_beats()
    │        │
    │        ▼
    │    rifflab-ipc: WorkerClient.submit(DetectBeats{...})
    │        │
    │        ▼
    │    Unix Socket ──► madmom_worker.py
    │        │               │
    │        │               └─ Write beat_grid.json
    │        ▼
    │    JobCompleted{output_paths}
    │
    └──► rifflab-analysis: transcribe_notes() [Rust, offline YIN + onset]
             │
             └─ Write bass_notes.json
```

### 5.3 Practice Comparison Path (real-time)

```
RT Audio Thread                     UI Thread
───────────────                     ─────────
YinDetector.detect(input_buffer)
    │
    ▼
PitchFrame {freq, confidence,       ┌─ Poll ring buffer ─┐
            midi_note, cents}        │                     │
    │                                │  ComparisonFrame    │
    ▼                                │  {reference_note,   │
Comparator.compare(frame, position)  │   played_pitch,     │
    │                                │   cents_deviation,   │
    ▼                                │   note_correct,      │
ComparisonFrame ──► ring buffer ────►│   accuracy_bucket}   │
    │                                │                     │
    ▼                                └─► Piano roll render  │
SessionScorer.feed(frame)               Status bar update  │
    │                                   Accuracy plots      │
    ▼                                                       │
score: 0.0–100.0                                            │
```

---

## 6. Deployment View

```
┌────────────────────────────────────────────────┐
│           AppImage / Flatpak                    │
│                                                 │
│  ┌─────────────────────────────────────────┐   │
│  │  rifflab (Rust binary)                   │   │
│  │  • rifflab-app + all library crates      │   │
│  │  • Statically linked (musl or glibc)     │   │
│  └─────────────────────────────────────────┘   │
│                                                 │
│  Dynamically linked:                            │
│  • libjack.so (via PipeWire or jackd2)         │
│  • libasound.so (ALSA)                         │
│  • Vulkan/OpenGL drivers (wgpu)                │
│                                                 │
│  ┌─────────────────────────────────────────┐   │
│  │  System Python 3.10+                     │   │
│  │  • demucs, torch, torchaudio             │   │
│  │  • madmom, msgpack                       │   │
│  └─────────────────────────────────────────┘   │
│                                                 │
│  ┌─────────────────────────────────────────┐   │
│  │  User Library (~/.local/share/rifflab/)  │   │
│  │  • songs/, presets/, effects/, sessions/ │   │
│  │  • rifflab.db (SQLite)                   │   │
│  └─────────────────────────────────────────┘   │
│                                                 │
│  ┌─────────────────────────────────────────┐   │
│  │  Config (~/.config/rifflab/)             │   │
│  │  • config.toml                           │   │
│  └─────────────────────────────────────────┘   │
└────────────────────────────────────────────────┘
```

---

## 7. Interface Contracts

### 7.1 AudioBackend (M1 boundary — hardware abstraction)

```rust
pub trait AudioBackend: Send {
    fn start(&mut self, config: &AudioConfig, callback: AudioCallback) -> Result<(), BackendError>;
    fn stop(&mut self) -> Result<(), BackendError>;
    fn backend_type(&self) -> BackendType;
    fn actual_sample_rate(&self) -> Option<u32>;
    fn actual_buffer_size(&self) -> Option<usize>;
}

pub type AudioCallback = Box<dyn FnMut(&[f32], &mut [f32], usize) + Send + 'static>;
//                              input    output   frames
```

**Contract:** Callback invoked from RT thread. Input/output are interleaved stereo. Callback must not block or allocate.

### 7.2 AudioProcessor (M2 boundary — DSP processing)

```rust
pub trait AudioProcessor: Send {
    fn process(&mut self, buffer: &mut [f32], sample_rate: u32);
    fn set_param(&mut self, param: ParamId, value: f32);
    fn reset(&mut self);
    fn name(&self) -> &str;
    fn is_bypassed(&self) -> bool;
}
```

**Contract:** `process()` is called from the RT thread. Must not allocate, lock, or syscall. Buffer is interleaved stereo f32, modified in-place.

### 7.3 EffectDescriptor (M2 boundary — self-describing effects)

```rust
pub trait EffectDescriptor: AudioProcessor {
    fn effect_type_id(&self) -> &str;
    fn param_descriptors(&self) -> Vec<ParamDescriptor>;
    fn get_param(&self, param: ParamId) -> f32;
}

pub struct ParamDescriptor {
    pub id: ParamId,
    pub name: String,
    pub unit: String,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub step: Option<f32>,
    pub kind: ParamKind,
}

pub enum ParamKind { Float, Int, Bool, Enum(Vec<String>) }
```

**Contract:** UI queries `param_descriptors()` to auto-generate controls. `get_param()` returns current value for display. Called from UI thread only (not RT-safe requirement on these methods).

### 7.4 IPC Protocol (M3/M4 boundary — Python workers)

```
Frame format: [4-byte BE length][MessagePack payload]

Direction: Rust → Python
    WorkerRequest::Separate { input_path, output_dir, model }
    WorkerRequest::DetectBeats { input_path, output_path }
    WorkerRequest::Cancel { job_id }
    WorkerRequest::Shutdown

Direction: Python → Rust
    WorkerResponse::JobStarted { job_id }
    WorkerResponse::Progress { job_id, percent, message }
    WorkerResponse::JobCompleted { job_id, output_paths }
    WorkerResponse::JobFailed { job_id, error }
    WorkerResponse::JobCancelled { job_id }
```

**Contract:** Workers are long-lived daemon processes. Socket path: `/run/user/{uid}/rifflab-{worker_type}.sock`. Worker spawned via `python3 {script} --socket {path}`. Crash recovery: Rust client detects broken pipe, respawns on next request.

### 7.5 Transport Control (M1 internal — UI ↔ RT thread)

```
UI Thread                          RT Audio Thread
─────────                          ──────────────
Transport.play()                   TransportRtHandle.advance()
  │                                  │
  └─► rtrb::Producer.push(Play) ───►│ rtrb::Consumer.pop() → Play
                                     │ state.store(1, Relaxed)
                                     │ position += frames
                                     │
Transport.position()◄──────────────── position.load(Relaxed)
Transport.state()◄────────────────── state.load(Relaxed)
```

**Contract:** Commands are fire-and-forget (ring buffer may drop if full). Position/state reads are eventually consistent (Relaxed ordering). No blocking on either side.

---

## 8. Architectural Decisions Log

| # | Decision | Rationale | Alternatives Considered |
|---|----------|-----------|------------------------|
| AD-1 | Fixed audio graph topology | Only one graph shape needed (stems + input + mixer); avoids topological sort in RT thread; simpler to reason about RT safety | Dynamic DAG with topological sort (rejected: over-engineered, allocation risk) |
| AD-2 | Lock-free inter-thread communication via rtrb | RT thread cannot block; rtrb is purpose-built for audio (zero-alloc after construction, SPSC, bounded) | Crossbeam channels (rejected: unbounded = allocation), mutex-protected queues (rejected: blocks RT thread) |
| AD-3 | Out-of-process Python workers | Python GIL + GC would block RT thread if in-process; PyTorch requires Python runtime; process isolation prevents crashes from affecting audio | In-process Python via PyO3 (rejected: GIL contention), ONNX Runtime in Rust (rejected: Demucs model not easily portable) |
| AD-4 | egui for UI toolkit | Immediate mode natural for real-time data viz; MIT license; trivial custom widgets; strong audio community precedent (Meadowlark, VST UIs) | iced (rejected: Elm architecture less natural for continuous data), slint (rejected: GPL or commercial license) |
| AD-5 | Rhai for scripted effects | Rust-native (no FFI), sandboxable (max_operations), Rust-like syntax; acceptable perf for 256-sample buffers | Lua/mlua (rejected: C FFI, GC issues), custom DSL (rejected: parser engineering effort), WASM (rejected: too heavy for per-sample DSP) |
| AD-6 | TOML for presets | Human-readable, hand-editable, native serde support; musicians can share preset files | JSON (rejected: less readable), binary (rejected: not hand-editable), SQLite (rejected: overkill for simple key-value) |
| AD-7 | Pre-loaded stem audio (heap buffer, not mmap) | mmap risks page faults in RT thread; 330MB for 6 stems is manageable on 16GB+ systems | mmap + madvise (rejected: page fault risk under memory pressure), streaming from disk (rejected: disk I/O in RT thread) |
| AD-8 | Trait-based module interfaces | Loose coupling; effects swappable (built-in, scripted, LV2 future); backends swappable (JACK, ALSA, future PulseAudio) | Concrete types with generics (rejected: less flexible), message passing (rejected: overhead for per-sample DSP) |
| AD-9 | Single Rust binary + Python subprocesses | Minimal deployment complexity; Python only where ML models require it; clean process boundary | Microservices (rejected: overkill), all-Rust with ONNX (rejected: model portability issues) |
| AD-10 | Atomics with Relaxed ordering for transport | Transport state/position are single-writer (RT thread writes, UI reads); Relaxed is sufficient for monotonic counters and status flags | SeqCst (rejected: unnecessary overhead), Acquire/Release (rejected: no dependent data requiring ordering guarantees) |

---

## 9. Traceability to Requirements

| Architecture Element | Traces To Requirements |
|---------------------|----------------------|
| Fixed audio graph (AD-1) | FR-M1-04, FR-M1-07, NFR-PERF-03, CPM-5 |
| Lock-free comms (AD-2) | FR-M1-08, NFR-PERF-04, CPM-6 |
| Out-of-process Python (AD-3) | FR-M3-01, FR-M4-07, IC-04, IC-05 |
| egui UI (AD-4) | FR-M7-01, IC-08, NFR-USE-01 |
| Rhai scripting (AD-5) | FR-M2-10, FR-M2-12, IC-11, IC-12 |
| JACK + ALSA backends | FR-M1-01, FR-M1-02, NFR-PLAT-02 |
| Module decomposition (M1–M7) | SR-10, NFR-PLAT-05 |
| Transport atomics (AD-10) | FR-M1-05, NFR-PERF-03 |
| Stem pre-loading (AD-7) | NFR-PERF-07, NFR-PERF-03 |
| Unix socket IPC (AD-3) | IC-06, FR-M3-01 |
