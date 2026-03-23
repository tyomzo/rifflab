# RiffLab — Software Requirements Specification (Linux)

**Document ID:** RIFFLAB-SRS-001
**Conformance:** ISO/IEC/IEEE 12207:2017 — Tailored conformance to tasks (§4.3)
**Version:** 1.1.0
**Status:** Draft
**Platform Scope:** Linux x86_64
**Author:** Artiom (with Claude)
**Date:** 2026-03-23
**Source Document:** RIFFLAB-PRD-001 v1.0.0

---

## Document Control

| Version | Date | Author | Change Summary |
|---------|------|--------|----------------|
| 1.0.0 | 2026-03-23 | Artiom | Initial release — Linux scope only |
| 1.1.0 | 2026-03-23 | Artiom | Soften GPU UI requirement; add self-describing effects + programmable effects (Rhai) |

**Tailoring Rationale (per Annex A):**
This document combines outputs from three ISO/IEC/IEEE 12207:2017 Technical Processes into a single specification to match the project's single-developer, open-source context:

- §6.4.1 Business or Mission Analysis
- §6.4.2 Stakeholder Needs and Requirements Definition
- §6.4.3 System/Software Requirements Definition

Architecture Definition (§6.4.4) and Design Definition (§6.4.5) are deferred to separate documents.

**Normative References:**

- ISO/IEC/IEEE 12207:2017 — Software life cycle processes
- ISO/IEC/IEEE 29148:2011 — Requirements engineering
- ISO/IEC 25010:2011 — Systems and software quality models
- ISO/IEC/IEEE 42010:2011 — Architecture description

---

## 1. Business or Mission Analysis (§6.4.1)

### 1.1 Problem or Opportunity Space (§6.4.1.3.b)

**Problem Statement:**
Practicing musicians on Linux lack an integrated tool that combines stem separation, real-time audio effects, and pitch/timing accuracy feedback. Existing options require either: (a) a full DAW (Ardour, REAPER) with significant configuration overhead and no built-in practice analysis, (b) separate disjointed tools for each capability, or (c) proprietary platforms unavailable on Linux.

**Opportunity:**
Deliver a single, modular application that covers the complete practice workflow — load a song, separate stems, play along with effects, receive accuracy feedback — with native Linux audio integration and sub-10ms latency.

### 1.2 Preliminary Operational Concept (§6.4.1.3.c)

The system operates as a desktop application on Linux x86_64 workstations. It connects to the system audio subsystem (PipeWire/JACK or ALSA) for hardware I/O, runs ML inference workloads (stem separation) as out-of-process Python workers, and renders its UI via wgpu (GPU-accelerated where available, software rasterization fallback).

**High-level operational modes:**

| Mode | Description |
|------|-------------|
| Import | User loads an audio file; system performs offline stem separation and beat tracking |
| Practice | Real-time playback of backing stems with live instrument input, effects processing, and accuracy comparison |
| Review | Post-session review of accuracy metrics and historical progress |
| Configure | Effect chain editing, preset management, cue point authoring |

### 1.3 Candidate Solution Classes (§6.4.1.3.c.2)

| Alternative | Assessment | Disposition |
|-------------|-----------|-------------|
| Full DAW with plugins | Overcomplicated for practice workflow; no integrated accuracy analysis | Rejected |
| Web-based application | Cannot meet latency requirements; no direct JACK/ALSA access | Rejected |
| Native modular Rust application with Python ML workers | Meets latency, modularity, and Linux-native requirements | **Selected** |

### 1.4 Traceability (§6.4.1.3.e)

| Business Objective | Traces To |
|--------------------|-----------|
| BO-1: Single integrated practice environment | Stakeholder needs SN-1, SN-2, SN-3 |
| BO-2: Native Linux audio with low latency | Stakeholder need SN-4 |
| BO-3: Modular, extensible architecture | Stakeholder need SN-5 |

---

## 2. Stakeholder Needs and Requirements Definition (§6.4.2)

### 2.1 Stakeholder Identification (§6.4.2.3.a)

| ID | Stakeholder Class | Role | Life Cycle Interest |
|----|-------------------|------|---------------------|
| SK-1 | Practicing Instrumentalist | Primary user | Operation: daily practice sessions |
| SK-2 | Live Performer | Secondary user | Operation: performance with backing tracks and auto-switching effects |
| SK-3 | Tinkerer / Developer | Tertiary user | Maintenance: custom DSP modules, alternative ML models, scripting |
| SK-4 | System Administrator | Enabler | Deployment: packaging, audio subsystem configuration |

### 2.2 Stakeholder Needs (§6.4.2.3.b)

| ID | Need | Stakeholder | Priority | Rationale |
|----|------|-------------|----------|-----------|
| SN-1 | Load a song and separate it into instrument stems | SK-1, SK-2 | Essential | Core workflow prerequisite — cannot practice along with a specific part without isolation |
| SN-2 | Play along with isolated backing tracks while hearing own instrument with real-time effects | SK-1, SK-2 | Essential | Defines the primary interaction loop |
| SN-3 | Receive visual feedback on pitch accuracy, timing, and note correctness during practice | SK-1 | Essential | Differentiator from a plain DAW — quantified practice feedback |
| SN-4 | Achieve low-latency audio I/O suitable for real-time instrument monitoring | SK-1, SK-2 | Essential | Latency > 12ms is perceptible and degrades playability |
| SN-5 | Extend the system with custom DSP or alternative ML models | SK-3 | Desirable | Supports long-term evolution and community contributions |
| SN-6 | Automatic effect preset switching synchronized to song position | SK-2 | Desirable | Enables hands-free performance with multiple tones per song |
| SN-7 | Track practice progress over time per song | SK-1 | Desirable | Motivates deliberate practice through visible improvement |
| SN-8 | Minimal setup friction — not a full DAW | SK-1 | Essential | Target users are not producers; complexity is a barrier |
| SN-9 | Create custom audio effects with user-defined parameters, editable in the UI | SK-3, SK-1 | Desirable | Enables experimentation and customization beyond built-in effects |

### 2.3 Operational Concept (§6.4.2.3.c)

#### 2.3.1 Scenario: First-Time Song Import

1. User invokes "Import Song" and selects an audio file (WAV, FLAC, MP3).
2. File is copied to the library under a new `song_id` directory.
3. A stem separation job is dispatched to the ML worker process. UI displays progress.
4. On completion, stems appear as tracks in the arrangement view.
5. A beat tracking job is dispatched. On completion, the beat grid overlays the timeline.
6. A reference transcription job runs on the user-selected practice stem. On completion, reference notes populate the piano roll.
7. Song metadata is persisted to the library.

#### 2.3.2 Scenario: Practice Session

1. User selects a song from the library. Cached stems and analysis data load immediately.
2. User mutes the stem they intend to play (e.g., bass).
3. User selects an effect preset for their instrument input.
4. User starts playback. Backing stems play through the master bus; live input is processed through the effects chain and mixed in.
5. Real-time pitch detection feeds the practice engine, which compares against the reference.
6. Piano roll displays reference notes with played notes overlaid in real time, color-coded by accuracy.
7. Status bar shows continuous pitch deviation and running accuracy score.
8. On stop, the session result is saved. Accuracy view shows a summary.

#### 2.3.3 Scenario: Cue-Based Performance (Phase 2)

1. User defines cue points on the song timeline: section markers and effect preset switches.
2. During playback, the cue engine dispatches preset changes at designated bar positions via crossfade.
3. User performs hands-free with automatic tone changes.

### 2.4 Context of Use — Linux Environment (§6.4.2.3.b.1)

| Factor | Description |
|--------|-------------|
| Operating System | Linux x86_64 — Ubuntu 22.04+, Pop!_OS, Fedora, Arch |
| Audio Subsystem | PipeWire (exposing JACK API) as primary; native JACK via `jack` crate; ALSA fallback via `cpal` |
| Display Server | Wayland (primary) and X11 (fallback) via UI toolkit abstraction |
| GPU | Vulkan or OpenGL preferred for UI rendering (software rasterization fallback acceptable); NVIDIA CUDA for optional ML acceleration |
| Python Runtime | System Python 3.10+ with `demucs`, `torch`, `torchaudio`, `madmom` |
| Hardware I/O | USB audio interfaces (class-compliant), onboard audio via ALSA |

### 2.5 Stakeholder Requirements (§6.4.2.3.d)

| ID | Requirement | Traces to Need | Validation Criterion |
|----|-------------|----------------|---------------------|
| SR-1 | The system shall separate a mixed audio file into 4 or 6 instrument stems | SN-1 | Stem files produced; subjective listening confirms isolation quality |
| SR-2 | The system shall play back stem audio files with per-stem solo, mute, and volume control | SN-2 | All stems audible independently; mute/solo toggles verified |
| SR-3 | The system shall process live instrument input through a configurable real-time effects chain | SN-2 | Audible effects on monitored input with measured latency ≤ 12ms |
| SR-4 | The system shall detect the fundamental pitch of the live input in real time | SN-3 | Pitch displayed on tuner; verified against reference tone generator |
| SR-5 | The system shall compare played notes against a reference transcription and display accuracy | SN-3 | Piano roll overlay; accuracy metrics within ±5¢ of manual measurement |
| SR-6 | The system shall connect to PipeWire/JACK or fall back to ALSA on Linux | SN-4 | Audio output confirmed on PipeWire, JACK, and ALSA-only systems |
| SR-7 | The system shall provide a practice-focused UI with arrangement view, piano roll, and accuracy display | SN-8 | Usability test: new user completes import-to-practice workflow in < 5 minutes |
| SR-8 | The system shall persist practice session results and display historical progress | SN-7 | Session data retrievable after application restart; trend chart renders |
| SR-9 | The system shall support cue-based automatic effect preset switching during playback | SN-6 | Effect change audible at designated cue point; crossfade smooth |
| SR-10 | The system's module interfaces shall be documented and support independent replacement | SN-5 | A module can be replaced with an alternative implementation without modifying other modules |
| SR-11 | The system shall allow users to create custom effects via scripting, with auto-generated UI parameter controls | SN-9 | User creates a scripted effect; parameters appear in effects editor; audio processes correctly |
| SR-12 | All effects (built-in and user-created) shall expose self-describing parameter metadata | SN-9, SN-5 | UI auto-generates controls from metadata for both built-in and scripted effects |

### 2.6 Critical Performance Measures (§6.4.2.3.e)

| ID | Measure | Target | Method |
|----|---------|--------|--------|
| CPM-1 | Input-to-output audio latency (effects chain) | ≤ 12ms @ 48kHz/256 samples; ≤ 6ms @ 48kHz/128 samples | Round-trip measurement with loopback cable |
| CPM-2 | Pitch detection latency | ≤ 10ms (one analysis frame) | Timestamp comparison: onset vs. detection event |
| CPM-3 | UI frame rate | 30–60 FPS for meters and piano roll | Frame time profiling |
| CPM-4 | Stem separation throughput | ≤ 2× real-time on NVIDIA GPU | Wall-clock timing of 4-minute song |
| CPM-5 | Real-time thread: heap allocations | Zero | Instrumented allocator in debug builds |
| CPM-6 | Audio dropouts (xruns) | Zero during normal operation at 256-sample buffer | JACK xrun callback counter over 30-minute session |

---

## 3. System/Software Requirements Definition (§6.4.3)

### 3.1 Functional Boundary (§6.4.3.3.a.1)

```
                    ┌──────────────────────────────────────┐
                    │           RiffLab Process             │
   Audio In ───────►│                                      │───────► Audio Out
  (JACK/ALSA)       │  ┌────┐ ┌────┐ ┌────┐ ┌────┐ ┌────┐ │       (JACK/ALSA)
                    │  │ M1 │ │ M2 │ │ M4 │ │ M5 │ │ M6 │ │
                    │  └────┘ └────┘ └────┘ └────┘ └────┘ │
                    │          ▲                            │
   Song files ─────►│  ┌────┐  │ IPC (Unix socket+MsgPack) │
  (WAV/FLAC/MP3)    │  │ M7 │  ▼                           │
                    │  └────┘ ┌────────────────────┐       │
   Display ◄────────│         │ Python Workers (M3)│       │
  (Wayland/X11)     │         │ Demucs, madmom     │       │
                    │         └────────────────────┘       │
                    │              ▲                        │
   Library ◄───────►│  SQLite DB   │ CUDA (optional)       │
  (filesystem)      │  + WAV stems │                       │
                    └──────────────────────────────────────┘
```

**External interfaces:**

| Interface | Protocol | Direction |
|-----------|----------|-----------|
| Audio hardware I/O | JACK client API (via PipeWire or native JACK) or ALSA via `cpal` | Bidirectional |
| Display | Wayland/X11 via UI toolkit (Vulkan/OpenGL backend) | Output |
| Filesystem | POSIX file I/O | Bidirectional |
| Python ML workers | Unix domain socket + MessagePack | Bidirectional |
| SQLite database | libsqlite3 (WAL mode) | Bidirectional |
| GPU compute (optional) | CUDA via PyTorch | Internal to Python worker |

### 3.2 System States and Modes (§6.4.3.3.b.2)

```
                    ┌───────────┐
                    │   Idle    │◄──────────────────────────┐
                    └─────┬─────┘                           │
                          │ load_song()                     │
                          ▼                                 │
                    ┌───────────┐                           │
              ┌────►│  Loaded   │──── unload() ────────────►│
              │     └─────┬─────┘                           │
              │           │ play()                          │
              │           ▼                                 │
              │     ┌───────────┐                           │
              │     │  Playing  │──── stop() ──────────────►│
              │     └─────┬─────┘                           │
              │           │ pause()                         │
              │           ▼                                 │
              │     ┌───────────┐                           │
              └─────│  Paused   │──── stop() ──────────────►┘
                    └───────────┘
```

**Background processing states (orthogonal):**

| State | Description |
|-------|-------------|
| Separating | M3 Python worker active; progress reported via IPC |
| Analyzing | M4 beat tracking Python worker active; progress reported via IPC |
| Transcribing | M4 offline pitch/onset detection running on stem file |

### 3.3 Functional Requirements (§6.4.3.3.b)

#### 3.3.1 M1: Audio Engine

| ID | Requirement | Traces To | Phase | Verification |
|----|-------------|-----------|-------|--------------|
| FR-M1-01 | The audio engine shall abstract the audio backend behind an `AudioBackend` trait with implementations for JACK (via `jack` crate) and ALSA (via `cpal` crate) | SR-6 | 1 | Test: instantiate both backends; confirm audio callback invoked |
| FR-M1-02 | The audio engine shall select the backend at runtime: JACK if PipeWire or JACK daemon is available, ALSA otherwise | SR-6 | 1 | Test: start with JACK available → JACK selected; start with JACK unavailable → ALSA selected |
| FR-M1-03 | The audio engine shall support buffer sizes of 64, 128, 256, 512 samples at sample rates of 44.1, 48, and 96 kHz | SR-3 | 1 | Test: configure each combination; confirm audio callback receives correct buffer size |
| FR-M1-04 | The audio engine shall maintain an internal audio graph with named buses: `input`, `stems[]`, `fx_return`, `master` | SR-2, SR-3 | 1 | Test: route signal through each bus; confirm output |
| FR-M1-05 | The audio engine shall provide transport controls: play, pause, stop, seek with sample-accurate position tracking | SR-2 | 1 | Test: seek to known position; verify sample count matches expected |
| FR-M1-06 | The audio engine shall report current transport position as both sample count and (bar, beat, tick) when a beat grid is available | SR-2 | 1 | Test: with beat grid loaded, verify (bar, beat) matches expected for given sample position |
| FR-M1-07 | The audio engine shall perform zero-copy buffer passing between audio graph nodes with no heap allocations in the real-time thread | CPM-5, CPM-6 | 1 | Test: instrumented allocator reports zero allocations during 60-second playback |
| FR-M1-08 | The audio engine shall expose per-bus peak and RMS metering via lock-free ring buffer | SR-7 | 1 | Test: play known signal; verify peak/RMS values within 0.1 dB of expected |
| FR-M1-09 | The audio engine shall support loop regions defined by start and end sample positions; when loop is enabled and the transport reaches the loop end, it shall wrap to the loop start | SR-2, SR-7 | 1 | Test: set loop region; play; verify transport wraps at end position and audio is continuous |
| FR-M1-10 | The audio engine shall support playback rate adjustment for tempo override (with pitch correction) | SR-9 | 2 | Test: adjust rate; verify tempo change without pitch shift |

#### 3.3.2 M2: Effects Engine

| ID | Requirement | Traces To | Phase | Verification |
|----|-------------|-----------|-------|--------------|
| FR-M2-01 | The effects engine shall model the effect chain as an ordered list of nodes, each implementing an `AudioProcessor` trait | SR-3 | 1 | Test: chain 3 effects; verify signal passes through all in order |
| FR-M2-02 | Phase 1 built-in effects shall include: tuner, noise gate, compressor, overdrive/distortion, 4-band parametric EQ, and reverb | SR-3 | 1 | Test: instantiate each effect; process known signal; verify output differs from input |
| FR-M2-03 | Phase 2 built-in effects shall add: chorus, phaser, flanger (LFO-modulated delay lines), delay (tempo-synced and free), and convolution cabinet simulation | SR-3 | 2 | Test: same as FR-M2-02 |
| FR-M2-04 | The effects engine shall support named presets stored as TOML or JSON files containing full chain state | SR-3 | 1 | Test: save preset; reload; verify all parameter values match |
| FR-M2-05 | Parameter changes shall be applied via crossfade (default 10ms) to avoid audible clicks | SR-3 | 2 | Test: sweep parameter; verify no clicks via peak-to-peak analysis of output |
| FR-M2-06 | The effects engine shall support crossfade transitions between presets triggered by the cue engine | SR-9 | 2 | Test: trigger crossfade; verify smooth transition via amplitude envelope analysis |
| FR-M2-07 | (Stretch) The effects engine shall support loading LV2 plugins as effect nodes | SR-10 | 3 | Test: load a known LV2 plugin; process signal; verify output |
| FR-M2-08 | All effects shall implement an `EffectDescriptor` interface exposing: effect_type_id, parameter descriptors (id, name, unit, min, max, default, kind), and current parameter values | SR-12 | 1 | Test: query each built-in effect for descriptors; verify metadata matches actual parameter behavior |
| FR-M2-09 | The effects engine shall maintain an effect registry listing all available effect types (built-in and user-created) | SR-11 | 2 | Test: registry returns all built-in effects; after adding a script file, registry includes the scripted effect |
| FR-M2-10 | Users shall be able to define custom effects as Rhai scripts declaring: a name, parameter descriptors, and a processing function | SR-11 | 2 | Test: write a Rhai script; load it; verify name and params discoverable; process audio through it |
| FR-M2-11 | Rhai scripts shall be pre-compiled to AST at load time; the audio thread shall not invoke script compilation | SR-11, CPM-5 | 2 | Test: verify no compilation during playback via profiling |
| FR-M2-12 | The Rhai scripting sandbox shall enforce execution limits (max operations, max stack depth) to prevent runaway scripts from blocking the audio thread | SR-11, CPM-6 | 2 | Test: load script with infinite loop; verify it terminates and effect is bypassed without xrun |
| FR-M2-13 | Scripted effect files shall be stored in `library/effects/*.rhai` and discovered by scanning that directory | SR-11 | 2 | Test: place a .rhai file in the directory; restart; verify effect appears in registry |

#### 3.3.3 M3: Stem Separator

| ID | Requirement | Traces To | Phase | Verification |
|----|-------------|-----------|-------|--------------|
| FR-M3-01 | The stem separator shall invoke Meta Demucs `htdemucs` (4-stem) via an out-of-process Python worker communicating over Unix domain sockets with MessagePack serialization | SR-1 | 1 | Test: send separation job; receive completion; verify 4 stem WAV files written |
| FR-M3-02 | The stem separator shall support `htdemucs_6s` (6-stem: vocals, drums, bass, guitar, piano, other) | SR-1 | 2 | Test: same as FR-M3-01 with 6 output files |
| FR-M3-03 | The stem separator shall use CUDA acceleration when an NVIDIA GPU with CUDA is available, falling back to CPU otherwise | SR-1, CPM-4 | 1 | Test: run on CUDA-capable system → GPU utilization observed; run without CUDA → completes on CPU |
| FR-M3-04 | The Python worker shall emit progress updates (percentage) over the Unix socket during separation | SR-7 | 1 | Test: monitor socket; verify monotonically increasing percentage values 0–100 |
| FR-M3-05 | Separation results shall be cached in the song library directory; re-separation triggered only by explicit user request or model change | SR-1 | 1 | Test: import same song twice; second import reuses cached stems (no Python worker invoked) |
| FR-M3-06 | The stem separator shall support cancellation of in-progress jobs | SR-1 | 1 | Test: start job; cancel; verify worker terminates and partial files are cleaned up |

#### 3.3.4 M4: Analysis Engine

| ID | Requirement | Traces To | Phase | Verification |
|----|-------------|-----------|-------|--------------|
| FR-M4-01 | The analysis engine shall perform real-time pitch detection on live input using the YIN algorithm with configurable hop size (default 256 samples at 48 kHz ≈ 5.3ms per frame) | SR-4, CPM-2 | 1 | Test: feed known sine wave; verify detected frequency within ±1 Hz |
| FR-M4-02 | Real-time pitch detection output shall include: fundamental frequency (Hz), confidence (0.0–1.0), MIDI note number, and cents deviation | SR-4 | 1 | Test: feed A4 (440 Hz); verify output: 440 Hz, confidence > 0.9, MIDI 69, cents ≈ 0 |
| FR-M4-03 | Pitch detection frequency range shall cover 30 Hz–2000 Hz | SR-4 | 1 | Test: feed signals at boundary frequencies; verify detection |
| FR-M4-04 | Real-time pitch detection shall complete within the audio callback period (no additional latency beyond one analysis frame) | CPM-2 | 1 | Test: measure wall-clock time of `detect_pitch_rt()`; verify < buffer period |
| FR-M4-05 | The analysis engine shall perform offline pitch detection on stem files, producing timestamped (time_seconds, frequency_hz, confidence) sequences | SR-5 | 1 | Test: run on known stem; verify note onsets and pitches against manual annotation |
| FR-M4-06 | The analysis engine shall perform spectral flux-based onset detection on stem audio, producing a list of onset timestamps | SR-5 | 1 | Test: run on stem with known transients; verify detected onsets within ±20ms |
| FR-M4-07 | Beat/tempo tracking shall be performed offline by a Python `madmom` worker via the same IPC pattern as M3 | SR-2 | 1 | Test: run on song with known BPM; verify detected BPM within ±1 BPM |
| FR-M4-08 | Beat tracking output shall be a BeatGrid: list of beat timestamps with BPM and time signature, supporting variable tempo | SR-2 | 1 | Test: run on song with tempo change; verify two distinct BPM regions |
| FR-M4-09 | The analysis engine shall combine pitch and onset detection into discrete NoteEvent records: `{midi_note, onset_seconds, duration_seconds, average_cents_deviation, confidence}` | SR-5 | 1 | Test: run on known monophonic stem; verify note sequence matches reference |
| FR-M4-10 | (Phase 2) The analysis engine shall perform sinusoidal decomposition via STFT → peak picking → partial tracking (McAulay-Quatieri) → harmonic grouping → note segmentation | SR-5 | 2 | Test: run on known stem; verify partial tracks and note segmentation |

#### 3.3.5 M5: Practice Engine

| ID | Requirement | Traces To | Phase | Verification |
|----|-------------|-----------|-------|--------------|
| FR-M5-01 | The practice engine shall accept a reference note sequence (from M4 offline transcription or MIDI file import) | SR-5 | 1 | Test: load reference; verify note count and content |
| FR-M5-02 | During playback, the practice engine shall receive live PitchFrame data from M4 and transport position from M1, and compare against the reference at the current song position | SR-5 | 1 | Test: feed known pitch frames at known positions; verify comparison output |
| FR-M5-03 | Pitch accuracy shall be computed as cents deviation from reference note, bucketed as: perfect (±5¢), good (±15¢), acceptable (±25¢), off (>25¢) | SR-5 | 1 | Test: feed pitch frames with known deviations; verify bucket assignment |
| FR-M5-04 | Timing accuracy shall be computed as onset time difference, bucketed as: tight (±20ms), good (±50ms), loose (±100ms), missed (>100ms) | SR-5 | 1 | Test: feed note onsets with known offsets; verify bucket assignment |
| FR-M5-05 | Note correctness shall be a binary determination: player hit the right note within ±50¢ of reference | SR-5 | 1 | Test: feed correct and incorrect pitches; verify boolean output |
| FR-M5-06 | The practice engine shall output a continuous stream of ComparisonFrame events for the UI | SR-5, SR-7 | 1 | Test: verify stream produces frames at analysis rate during playback |
| FR-M5-07 | Per-session scoring shall aggregate pitch accuracy, timing accuracy, and note correctness into a 0.0–100.0 score with configurable weights | SR-5 | 1 | Test: complete session with known data; verify score calculation |
| FR-M5-08 | Each practice session shall be persisted to SQLite: song ID, timestamp, per-note results, aggregate scores | SR-8 | 2 | Test: complete session; query database; verify all fields present |
| FR-M5-09 | Historical progress per song shall be queryable, broken down by section | SR-8 | 2 | Test: complete multiple sessions; query history; verify trend data |

#### 3.3.6 M6: Cue Engine

| ID | Requirement | Traces To | Phase | Verification |
|----|-------------|-----------|-------|--------------|
| FR-M6-01 | The cue engine shall support cue types: EffectPresetSwitch, LoopRegion, SectionMarker, TempoOverride | SR-9 | 2 | Test: create each cue type; verify serialization round-trip |
| FR-M6-02 | Cue positions shall be expressible as (bar, beat) when a beat grid is available, or absolute time (seconds) otherwise | SR-9 | 2 | Test: create cue at bar 4 beat 1; verify trigger at correct sample position |
| FR-M6-03 | The cue engine shall subscribe to transport position updates from M1 and dispatch actions when the playhead crosses a cue point | SR-9 | 2 | Test: set cue; play through; verify action dispatched within one audio buffer of cue position |
| FR-M6-04 | EffectPresetSwitch cues shall trigger M2's crossfade_to_preset() | SR-9 | 2 | Test: set effect cue; play through; verify effect change audible at cue point |
| FR-M6-05 | LoopRegion cues shall instruct M1's transport to loop between start and end positions | SR-9 | 2 | Test: set loop cue; verify transport wraps |
| FR-M6-06 | Cues shall be stored per-song and persisted to the library | SR-9 | 2 | Test: save cues; reload song; verify cues restored |

#### 3.3.7 M7: UI Shell

| ID | Requirement | Traces To | Phase | Verification |
|----|-------------|-----------|-------|--------------|
| FR-M7-01 | The UI shall render via a wgpu-backed toolkit on Wayland and X11, using GPU acceleration when available and software rasterization as fallback | SR-7 | 1 | Test: launch on Wayland; launch on X11; verify rendering. Test on system without GPU: verify software rendering at >= 30 FPS |
| FR-M7-02 | The UI shall present an arrangement view with: horizontal timeline, beat grid overlay, one lane per stem, one lane for live input, playhead synchronized to transport | SR-7 | 1 | Test: load song; verify all lanes visible; playhead moves during playback |
| FR-M7-03 | Each stem lane shall display a waveform overview rendered from the stem audio | SR-7 | 1 | Test: load song; verify waveforms render for all stems |
| FR-M7-04 | The sidebar shall provide per-track: name, color, solo (S), mute (M), volume slider, pan knob | SR-2 | 1 | Test: toggle solo/mute; adjust volume; verify audio output changes |
| FR-M7-05 | The toolbar shall include: transport controls (play/pause/stop/seek/loop toggle), BPM display, song selector, global tuner indicator, metronome toggle | SR-7 | 1 | Test: verify each control is present and functional |
| FR-M7-06 | The bottom drawer shall support switchable views: Piano Roll, Accuracy, Effects Editor | SR-7 | 1 (Piano Roll), 2 (Accuracy, Effects Editor) | Test: switch between views; verify correct content displayed |
| FR-M7-07 | Piano Roll view shall display reference note events as semi-transparent rectangles and player's detected notes as solid rectangles, color-coded: green (correct+in tune), yellow (slightly off), orange (significantly off), red (wrong note), grey outline (missed) | SR-5, SR-7 | 1 | Test: play known sequence; verify color coding matches accuracy data |
| FR-M7-08 | The status bar shall display: latency, CPU usage, current pitch, accuracy, and session score | SR-7 | 1 | Test: during playback, verify all metrics update |
| FR-M7-09 | The UI shall support zoom and scroll on the arrangement timeline (horizontal: time; vertical: track height) | SR-7 | 1 | Test: zoom in/out; scroll; verify view updates |
| FR-M7-10 | Click on the arrangement timeline shall seek the transport. Drag shall create a visible loop region with a highlighted bracket and draggable start/end handles. Handles can be repositioned independently. Loop bracket shall snap to the beat grid when available. The toolbar loop toggle enables/disables playback looping over the selected region | SR-2, SR-7 | 1 | Test: drag on timeline → loop bracket appears with handles; drag a handle → region resizes; enable loop toggle → transport wraps at loop end; snap to beat verified when beat grid present |
| FR-M7-11 | The cue lane shall display color-coded markers for section labels, effect switches, and loop regions | SR-9 | 2 | Test: create cues; verify markers visible with correct colors |
| FR-M7-12 | Accuracy view shall show: pitch deviation time-series (±50¢), timing deviation per onset, aggregate stats, and historical comparison ghost line | SR-8 | 2 | Test: complete session; verify all plots render with correct data |
| FR-M7-13 | Effects Editor view shall show the effect chain as a left-to-right signal flow. Clicking an effect node shall display parameter controls auto-generated from the effect's `param_descriptors()`: float params as sliders, bool params as toggles, enum params as dropdowns. Support drag-to-reorder, add/remove effects, and preset save/load | SR-3, SR-12 | 2 | Test: open editor; verify auto-generated controls match effect descriptors; reorder effects; save/load preset |
| FR-M7-14 | The Effects Editor shall include an "Add Effect" dialog listing all effects from the effect registry, grouped by source (Built-in, User Script) | SR-11 | 2 | Test: open dialog; verify all registered effects listed; select one; verify it appears in chain |
| FR-M7-15 | For scripted effects, the Effects Editor shall show an "Edit Script" action that opens the .rhai file in the system's default text editor | SR-11 | 2 | Test: click "Edit Script"; verify file opens in editor |

### 3.4 Non-Functional Requirements (§6.4.3.3.b.4, b.5)

#### 3.4.1 Performance

| ID | Requirement | Traces To | Verification |
|----|-------------|-----------|--------------|
| NFR-PERF-01 | Input-to-output latency through the effects chain shall be ≤ 12ms at 48 kHz with 256-sample buffers | CPM-1 | Loopback measurement |
| NFR-PERF-02 | Input-to-output latency through the effects chain shall be ≤ 6ms at 48 kHz with 128-sample buffers | CPM-1 | Loopback measurement |
| NFR-PERF-03 | The real-time audio thread shall perform zero heap allocations, acquire no locks, and make no syscalls | CPM-5 | Instrumented allocator; strace audit |
| NFR-PERF-04 | All inter-thread communication shall use lock-free ring buffers or atomic operations | CPM-5, CPM-6 | Code review; lock detector |
| NFR-PERF-05 | Stem separation shall complete in ≤ 2× real-time on an NVIDIA GPU with CUDA | CPM-4 | Wall-clock benchmark |
| NFR-PERF-06 | UI shall update at 30–60 FPS, decoupled from the audio thread | CPM-3 | Frame time profiling |
| NFR-PERF-07 | Memory usage for a 5-minute song with 6 stems at 48 kHz/32-bit (≈330 MB of audio data) shall remain within 2 GB total process memory | — | Memory profiling (RSS measurement) |

#### 3.4.2 Platform and Portability (Linux Scope)

| ID | Requirement | Verification |
|----|-------------|--------------|
| NFR-PLAT-01 | The system shall run on Linux x86_64: Ubuntu 22.04+, Pop!_OS, Fedora 38+, Arch Linux | Test: install and run on each distribution |
| NFR-PLAT-02 | Audio backend: JACK via PipeWire (primary), native JACK (secondary), ALSA via `cpal` (fallback) | Test: verify on PipeWire, JACK, and ALSA-only systems |
| NFR-PLAT-03 | Display: Wayland (primary) and X11 (fallback) via the UI toolkit's cross-platform abstraction | Test: launch on Wayland compositor; launch on X11 |
| NFR-PLAT-04 | GPU acceleration for ML: NVIDIA CUDA optional; CPU fallback always available | Test: run stem separation with and without CUDA |
| NFR-PLAT-05 | Platform-specific code shall be limited to audio backend selection in M1 (behind `#[cfg(target_os)]` gates); M2–M6 shall contain no platform-specific code | Code review |
| NFR-PLAT-06 | Packaging: AppImage or Flatpak, with system Python for ML workers | Test: install via AppImage on clean system |

#### 3.4.3 Reliability and Data Integrity

| ID | Requirement | Verification |
|----|-------------|--------------|
| NFR-REL-01 | Practice session data shall be stored in SQLite with WAL mode for crash safety | Test: kill process during session write; restart; verify database integrity |
| NFR-REL-02 | Stem files shall be immutable once generated; re-separation creates new files | Test: verify file modification timestamps unchanged after re-load |
| NFR-REL-03 | The system shall report audio xruns to the user via the status bar | Test: induce xrun by CPU load; verify status bar notification |

#### 3.4.4 Usability

| ID | Requirement | Verification |
|----|-------------|--------------|
| NFR-USE-01 | A new user shall be able to complete the import-to-practice workflow (import song → separate → play along → see feedback) within 5 minutes, excluding separation processing time | Usability test |
| NFR-USE-02 | The UI shall not expose production-DAW complexity: no clip/session view, no MIDI editing, no mixer window, no plugin browser, no audio recording to tracks, no automation lanes | Design review |

### 3.5 Implementation Constraints (§6.4.3.3.b.3)

| ID | Constraint | Rationale |
|----|-----------|-----------|
| IC-01 | Primary implementation language: Rust | Real-time safety (no GC), performance, memory safety without runtime overhead |
| IC-02 | DSP libraries: `fundsp`, `biquad`, `rustfft` (pure Rust) | Portability; no C/C++ DSP library dependencies |
| IC-03 | Audio I/O: `cpal` crate (ALSA) + `jack` crate (JACK) | Native Linux audio access without abstraction layer overhead |
| IC-04 | Stem separation: Python + Demucs (`htdemucs`, `htdemucs_6s`) via out-of-process worker | Demucs requires PyTorch; isolation prevents Python GIL from affecting real-time thread |
| IC-05 | Beat tracking: Python + `madmom` via out-of-process worker | Same isolation rationale as IC-04 |
| IC-06 | IPC: Unix domain sockets + MessagePack serialization | Low overhead; available on all Linux systems; no network exposure |
| IC-07 | Persistence: SQLite (WAL mode) + WAV/FLAC files on disk | Embedded database; no server process; crash-safe |
| IC-08 | UI toolkit: `egui` via `eframe`/`wgpu` | wgpu-backed Rust-native toolkit with Wayland/X11 support; GPU preferred, software fallback acceptable |
| IC-09 | No Electron, no Wine, no web runtime | Native performance; Linux-native audio access |
| IC-10 | Build: single Rust workspace; `cargo build` produces main binary | Simplicity; standard Rust toolchain |
| IC-11 | Scripted effects: Rhai scripting engine | Rust-native, sandboxable, no FFI, Rust-like syntax accessible to target audience |
| IC-12 | Scripted effect scripts: pre-compiled AST, no runtime parsing in audio thread | Real-time safety: parsing allocates and is unbounded in time |

### 3.6 Data Model (§6.4.3.3.b.5.i)

#### 3.6.1 Library Directory Structure

```
library/
├── songs/
│   └── {song_id}/
│       ├── original.wav
│       ├── stems/
│       │   ├── vocals.wav
│       │   ├── drums.wav
│       │   ├── bass.wav
│       │   ├── guitar.wav
│       │   ├── piano.wav          # 6-stem only
│       │   └── other.wav          # 6-stem only
│       ├── analysis/
│       │   ├── beat_grid.json
│       │   ├── bass_notes.json
│       │   └── sections.json
│       ├── cues.json
│       └── metadata.json
├── effects/
│   ├── my_fuzz.rhai            # User-defined effect scripts
│   └── ...
├── presets/
│   ├── bass_clean.toml
│   └── ...
├── sessions/
│   └── {session_id}.json
└── rifflab.db
```

#### 3.6.2 Core Data Entities

| Entity | Key Fields | Storage |
|--------|-----------|---------|
| Song | id, title, artist, bpm, key, time_signature, stems[], beat_grid, cues[] | SQLite + JSON + WAV files |
| BeatGrid | beats[]: {time_seconds, bar, beat, bpm} | JSON file per song |
| NoteEvent | midi_note, onset_seconds, duration_seconds, average_cents, confidence | JSON file per stem |
| EffectPreset | name, chain_state (effect order + all parameter values) | TOML file |
| Cue | id, position (bar/beat or seconds), action (preset switch / loop / marker / tempo) | JSON file per song |
| SessionResult | session_id, song_id, timestamp, duration, notes_total/correct/missed/extra, avg_cents, avg_timing, per_note_results[], score | SQLite + JSON file |

### 3.7 Requirements Traceability Matrix (§6.4.3.3.d.2)

| Stakeholder Req | System/Software Requirements |
|-----------------|------------------------------|
| SR-1 (Stem separation) | FR-M3-01, FR-M3-02, FR-M3-03, FR-M3-04, FR-M3-05, FR-M3-06 |
| SR-2 (Stem playback) | FR-M1-04, FR-M1-05, FR-M1-06, FR-M7-04 |
| SR-3 (Effects chain) | FR-M2-01, FR-M2-02, FR-M2-03, FR-M2-04, FR-M2-05, FR-M2-06 |
| SR-4 (Pitch detection) | FR-M4-01, FR-M4-02, FR-M4-03, FR-M4-04 |
| SR-5 (Accuracy comparison) | FR-M4-05, FR-M4-06, FR-M4-09, FR-M5-01 through FR-M5-07, FR-M7-07 |
| SR-6 (Linux audio backends) | FR-M1-01, FR-M1-02 |
| SR-7 (Practice-focused UI) | FR-M1-08, FR-M7-01 through FR-M7-10, NFR-USE-01, NFR-USE-02 |
| SR-8 (Session history) | FR-M5-08, FR-M5-09, FR-M7-12 |
| SR-9 (Cue-based switching) | FR-M1-09, FR-M1-10, FR-M2-06, FR-M6-01 through FR-M6-06, FR-M7-11 |
| SR-10 (Modular interfaces) | FR-M2-07, IC-01 through IC-12 |
| SR-11 (Custom scripted effects) | FR-M2-09, FR-M2-10, FR-M2-11, FR-M2-12, FR-M2-13, FR-M7-14, FR-M7-15 |
| SR-12 (Self-describing effects) | FR-M2-08, FR-M7-13 |

---

## 4. Phasing and Life Cycle Stages (§5.4)

### 4.1 Phase 1 — Core Loop (MVP)

**Stage:** Development
**Objective:** Functional practice tool — import, separate (4-stem), play along with effects, see pitch accuracy.

**In-scope requirements:** All Phase 1 requirements from §3.3 tables.

| Module | Scope |
|--------|-------|
| M1 | JACK + ALSA backends, stem playback, live input routing, transport, loop regions, master mix, metering |
| M2 | Linear chain: tuner, noise gate, compressor, overdrive, EQ, reverb. Self-describing parameter metadata on all effects (`EffectDescriptor`). Effect registry (built-in only). Preset save/load |
| M3 | Demucs htdemucs (4-stem), CUDA/CPU, progress reporting, caching |
| M4 | Real-time YIN pitch detection, offline beat tracking (madmom), simplified note transcription |
| M5 | Real-time comparison, per-session scoring (no persistence yet) |
| M6 | Not included |
| M7 | Arrangement view, sidebar, tuner, basic piano roll, status bar |

### 4.2 Phase 2 — Automation and Polish

**Stage:** Development (incremental)
**Objective:** Cue automation, historical tracking, 6-stem, improved UI.

**In-scope requirements:** All Phase 2 requirements from §3.3 tables.

| Module | Additions |
|--------|-----------|
| M1 | Tempo override (playback rate adjustment) |
| M2 | Crossfade preset switching, delay, chorus, cabinet sim. User-defined scripted effects (Rhai). Effect registry with script discovery from `library/effects/` |
| M3 | htdemucs_6s (6-stem), model selection |
| M4 | Sinusoidal decomposition |
| M5 | SQLite session history, progress visualization |
| M6 | Full cue engine |
| M7 | Cue lane, accuracy view, effects editor with auto-generated controls and effect browser, section markers |

### 4.3 Phase 3 — Advanced Features

**Stage:** Development (incremental)
**Objective:** Deep editing, extended instrument support, LV2 plugin hosting.

| Module | Additions |
|--------|-----------|
| M2 | LV2 plugin hosting, convolution cabinet with IR import |
| M4 | Polyphonic pitch detection, MIDI export |
| M5 | Difficulty curve analysis, adaptive tempo |
| M6 | Tempo ramp cues |
| M7 | Note editor, drag-to-reassign stems |

---

## 5. Open Questions (§6.3.3 Decision Management)

| ID | Question | Impact | Status | Decision Criteria |
|----|----------|--------|--------|-------------------|
| OQ-1 | UI toolkit: `egui` vs. `iced` vs. `slint` | M7 architecture, rendering performance, Wayland/X11 support | Open | Evaluate: GPU rendering perf, Wayland native support, custom widget capability, licensing |
| OQ-2 | Python workers: long-lived daemon vs. spawned per-job | M3, M4 startup latency vs. lifecycle complexity | Open | Measure: cold-start time of PyTorch import; assess daemon crash recovery |
| OQ-3 | LV2 plugin hosting worth the complexity? | M2 Phase 3 scope | Open | Survey user demand; estimate implementation effort |
| OQ-4 | Cue system OSC/MIDI CC output for external hardware | M6 extensibility | Open | Assess live performer demand |
| OQ-5 | Licensing model: fully open source vs. open core | Distribution, community | Open | — |
| OQ-6 | Final project name | Branding | Open | — |
| OQ-7 | Should scripted effects support per-buffer processing or per-sample only? | M2 scripted effect performance vs. usability | Open | Benchmark: measure overhead of per-sample Rhai calls on 256-sample buffer |
| OQ-8 | Should bundled example Rhai scripts ship with the application? | M2 onboarding | Open | User feedback on effect creation workflow |

---

## 6. Glossary (§3.1)

| Term | Definition |
|------|------------|
| Stem | An isolated audio track for a single instrument or group, derived from source separation |
| YIN | A fundamental frequency estimation algorithm based on autocorrelation, optimized for monophonic signals |
| STFT | Short-Time Fourier Transform — converts time-domain audio into a time-frequency representation |
| Partial | A single sinusoidal component in the spectral decomposition of an audio signal |
| Cue | A time-positioned event on the song timeline that triggers an action |
| Beat grid | A sequence of beat timestamps aligned to the song's tempo, enabling bar/beat addressing |
| Crossfade | A smooth transition between two states over a short duration to avoid audible artifacts |
| xrun | An audio buffer underrun or overrun, causing an audible glitch |
| Lock-free | A concurrent programming pattern that avoids mutex locks, critical for real-time audio |
| PipeWire | A Linux multimedia server that provides JACK and PulseAudio compatibility |
| ALSA | Advanced Linux Sound Architecture — the kernel-level audio subsystem |
| JACK | JACK Audio Connection Kit — a low-latency audio server API |
| MessagePack | A binary serialization format, more compact and faster than JSON |
| WAL mode | Write-Ahead Logging — an SQLite journaling mode providing concurrent reads during writes and crash safety |
| Rhai | An embedded scripting language for Rust with a Rust-like syntax, sandboxing, and no external dependencies |
| Effect descriptor | Metadata exposed by an audio effect declaring its parameters (names, ranges, types), enabling auto-generated UI controls |
| Effect registry | A catalog of all available audio effects (built-in and user-created) that the UI and effect chain can query |
| wgpu | A Rust graphics API that auto-selects the best available GPU backend (Vulkan, OpenGL, Metal) and falls back to software rasterization |
