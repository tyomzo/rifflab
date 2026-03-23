# RiffLab — Implementation Plan

**Document ID:** RIFFLAB-IMP-001
**Version:** 1.0.0
**Status:** Draft
**Author:** Artiom (with Claude)
**Date:** 2026-03-23
**Source Documents:** RIFFLAB-PRD-001 v1.1.0, RIFFLAB-SRS-001 v1.1.0, RIFFLAB-SAD-001 v1.0.0, RIFFLAB-SDD-001 v1.0.0

---

## Current State

The workspace is scaffolded with 9 Rust crates compiling with zero errors and 5 passing tests. The table below summarizes what exists vs. what's needed.

| Crate | Done | Stub / TODO |
|-------|------|-------------|
| rifflab-core | All types, traits, metering (with tests) | — |
| rifflab-audio | JACK backend, transport (atomics + ring buffer), audio graph, stem player, engine orchestrator | ALSA backend (cpal streams), loop handle tests |
| rifflab-fx | 5 effects (gate, compressor, overdrive, EQ, reverb), chain, preset I/O | EffectDescriptor impl, effect registry, tuner |
| rifflab-ipc | Protocol framing (encode/decode), worker spawn | WorkerClient (socket connect, background reader, job polling) |
| rifflab-stems | Separator (job submission), cache check | Depends on IPC client completion |
| rifflab-analysis | YIN pitch detector (real-time, tested) | Onset detection, offline transcription, beat tracker depends on IPC |
| rifflab-practice | Comparator, session scorer, MIDI + JSON reference loading | Timing offset computation |
| rifflab-ui | egui app shell (panels, placeholder buttons) | All views, all widgets, state wiring |
| rifflab-app | Entry point (launches UI) | CLI args, config, library init, worker spawn, module wiring |
| workers/ | requirements.txt | demucs_worker.py, madmom_worker.py |

---

## Phase 1 — Core Loop (MVP)

**Goal:** Import a song → separate stems → play along with effects → see pitch feedback.

### Milestone 1.1: Audio Playback

**Delivers:** Load a WAV file and hear it through JACK. Transport controls work.

| Task | Crate | Req | Description |
|------|-------|-----|-------------|
| 1.1.1 | rifflab-audio | FR-M1-01 | Complete ALSA backend via cpal (input + output streams, callback wiring) |
| 1.1.2 | rifflab-audio | FR-M1-02 | Runtime backend selection: try JACK, fall back to ALSA (already in create_backend, needs integration test) |
| 1.1.3 | rifflab-app | — | Wire AudioEngine into main.rs: init config, start engine, load a hardcoded test WAV |
| 1.1.4 | rifflab-app | — | Add symphonia-based WAV/FLAC/MP3 decoder: file → Vec<f32> interleaved audio |
| 1.1.5 | rifflab-audio | FR-M1-05 | Verify transport play/pause/stop/seek with integration test (play WAV, seek, verify position) |
| 1.1.6 | rifflab-audio | FR-M1-09 | Verify loop region: set loop, play through, confirm transport wraps |

**Exit criteria:** `cargo run -- test.wav` plays audio through JACK/ALSA. Play/pause/stop work. Loop wraps correctly.

### Milestone 1.2: Stem Separation Pipeline

**Delivers:** Import a song, run Demucs, get 4 stem WAV files.

| Task | Crate | Req | Description |
|------|-------|-----|-------------|
| 1.2.1 | rifflab-ipc | FR-M3-01 | Implement WorkerClient: tokio async socket connect, background reader task, job handle → response channel |
| 1.2.2 | workers/ | FR-M3-01 | Write demucs_worker.py: Unix socket listener, MessagePack framing, run htdemucs, emit progress, write stems |
| 1.2.3 | rifflab-stems | FR-M3-04 | Wire StemSeparator to live WorkerClient: submit job, poll progress, handle completion |
| 1.2.4 | rifflab-stems | FR-M3-05 | Verify caching: re-import same song → skip separation |
| 1.2.5 | rifflab-stems | FR-M3-06 | Implement cancellation: cancel in-progress job, clean up partial files |
| 1.2.6 | rifflab-stems | FR-M3-03 | Verify CUDA path (when GPU available) and CPU fallback |

**Exit criteria:** Run separation from Rust, receive progress updates, 4 stem WAV files appear in library directory. Re-import is a no-op.

### Milestone 1.3: Beat Tracking + Note Transcription

**Delivers:** Import a song → get beat grid + reference note transcription.

| Task | Crate | Req | Description |
|------|-------|-----|-------------|
| 1.3.1 | workers/ | FR-M4-07 | Write madmom_worker.py: Unix socket listener, run BeatDetectionProcessor, write beat_grid.json |
| 1.3.2 | rifflab-analysis | FR-M4-07 | Wire BeatTracker to live WorkerClient, verify beat grid output |
| 1.3.3 | rifflab-analysis | FR-M4-06 | Implement spectral flux onset detection using rustfft |
| 1.3.4 | rifflab-analysis | FR-M4-05 | Implement offline pitch detection (batch YIN over full stem file) |
| 1.3.5 | rifflab-analysis | FR-M4-09 | Combine pitch + onset into Vec<NoteEvent> (transcribe_notes) |

**Exit criteria:** Given a bass stem WAV, produce beat_grid.json and bass_notes.json with reasonable accuracy.

### Milestone 1.4: Effects Chain + EffectDescriptor

**Delivers:** Live instrument input processed through effects chain. All effects self-describing.

| Task | Crate | Req | Description |
|------|-------|-----|-------------|
| 1.4.1 | rifflab-core | FR-M2-08 | Add EffectDescriptor trait, ParamDescriptor, ParamKind to audio.rs |
| 1.4.2 | rifflab-fx | FR-M2-08 | Implement EffectDescriptor for all 5 existing effects (return param metadata) |
| 1.4.3 | rifflab-fx | FR-M2-08 | Update EffectChain to use Box<dyn EffectDescriptor> instead of Box<dyn AudioProcessor> |
| 1.4.4 | rifflab-fx | FR-M2-09 | Create EffectRegistry with built-in effect factories |
| 1.4.5 | rifflab-fx | FR-M2-02 | Implement tuner effect (pitch display, audio passthrough) |
| 1.4.6 | rifflab-audio | FR-M1-04 | Wire EffectChain into AudioGraph.fx_chain via rifflab-app |
| 1.4.7 | rifflab-fx | FR-M2-04 | Verify preset save/load round-trip with EffectDescriptor metadata |

**Exit criteria:** Plug in instrument → hear effects on input. Query any effect for its parameter names/ranges. Save/load presets.

### Milestone 1.5: Practice Comparison (Real-Time)

**Delivers:** Play along and see pitch accuracy feedback in real time.

| Task | Crate | Req | Description |
|------|-------|-----|-------------|
| 1.5.1 | rifflab-analysis | FR-M4-04 | Integrate YinDetector into audio callback path (process input buffer, emit PitchFrame via ring buffer) |
| 1.5.2 | rifflab-practice | FR-M5-01 | Load reference from transcribed notes or MIDI file |
| 1.5.3 | rifflab-practice | FR-M5-02 | Wire Comparator: receive PitchFrame + SongPosition, output ComparisonFrame via ring buffer |
| 1.5.4 | rifflab-practice | FR-M5-07 | Per-session scoring: feed all ComparisonFrames, compute final score |
| 1.5.5 | rifflab-practice | FR-M5-04 | Implement timing offset computation (onset alignment between played and reference notes) |

**Exit criteria:** Play along with a song → terminal prints running accuracy score. ComparisonFrames flowing on ring buffer for UI.

### Milestone 1.6: UI — Arrangement View + Sidebar

**Delivers:** Visual song arrangement with waveforms, transport controls, track mixer.

| Task | Crate | Req | Description |
|------|-------|-----|-------------|
| 1.6.1 | rifflab-ui | FR-M7-05 | Implement toolbar: transport buttons wired to AudioEngine.transport, BPM display, song selector (file picker), loop toggle |
| 1.6.2 | rifflab-ui | FR-M7-02 | Implement arrangement view: horizontal timeline, playhead synchronized to transport position |
| 1.6.3 | rifflab-ui | — | Implement waveform widget: pre-compute peak overview from stem audio, render as line segments per zoom level |
| 1.6.4 | rifflab-ui | FR-M7-03 | Render stem waveforms in arrangement lanes |
| 1.6.5 | rifflab-ui | FR-M7-04 | Implement sidebar: per-track name, color, solo (S), mute (M), volume slider |
| 1.6.6 | rifflab-ui | FR-M7-09 | Zoom and scroll on arrangement timeline |
| 1.6.7 | rifflab-ui | FR-M7-10 | Click to seek. Drag to create loop region with visible handles. Snap to beat grid. |
| 1.6.8 | rifflab-ui | — | Implement meter widget: peak/RMS bars consuming MeterData from ring buffer |
| 1.6.9 | rifflab-ui | FR-M7-08 | Status bar: latency, CPU usage, current pitch (from PitchFrame ring buffer), session score |

**Exit criteria:** Launch app → import song → see waveforms → play/pause/seek works → solo/mute stems → drag loop handles → meters animate.

### Milestone 1.7: UI — Piano Roll + Tuner

**Delivers:** Visual pitch accuracy feedback during practice.

| Task | Crate | Req | Description |
|------|-------|-----|-------------|
| 1.7.1 | rifflab-ui | FR-M7-06 | Implement bottom drawer with switchable views (Piano Roll active by default in Phase 1) |
| 1.7.2 | rifflab-ui | FR-M7-07 | Piano roll view: reference notes as semi-transparent rectangles, played notes as solid rectangles |
| 1.7.3 | rifflab-ui | FR-M7-07 | Color coding: green (correct+in tune), yellow (slightly off), orange (significantly off), red (wrong note), grey outline (missed) |
| 1.7.4 | rifflab-ui | FR-M7-05 | Tuner indicator in toolbar: note name + cents deviation needle, updated from PitchFrame stream |

**Exit criteria:** Play along → piano roll scrolls with playhead, showing reference vs. played notes in real time with color-coded accuracy. Tuner shows current note.

### Milestone 1.8: Import Workflow + App Wiring

**Delivers:** End-to-end: import song → separate → analyze → practice → score.

| Task | Crate | Req | Description |
|------|-------|-----|-------------|
| 1.8.1 | rifflab-app | — | CLI arg parsing with clap: `rifflab [song_file]` or interactive import |
| 1.8.2 | rifflab-app | — | Library directory init (XDG: ~/.local/share/rifflab/), SQLite DB creation |
| 1.8.3 | rifflab-app | — | Config loading from ~/.config/rifflab/config.toml (with defaults) |
| 1.8.4 | rifflab-app | — | Worker process management: spawn demucs + madmom workers on startup, shutdown on exit |
| 1.8.5 | rifflab-app | — | Import pipeline: decode audio (symphonia) → copy to library → trigger separation → trigger beat tracking → trigger transcription → load into UI |
| 1.8.6 | rifflab-ui | — | Import progress UI: modal dialog with progress bar for separation + analysis |
| 1.8.7 | rifflab-app | — | Full integration wiring: AudioEngine ↔ EffectChain ↔ YinDetector ↔ Comparator ↔ UI ring buffers |

**Exit criteria:** `rifflab song.mp3` launches the full practice workflow end-to-end. NFR-USE-01: new user completes import-to-practice in < 5 minutes (excluding separation time).

---

## Phase 2 — Automation and Polish

**Goal:** Cue automation, session history, 6-stem, scripted effects, improved UI.

### Milestone 2.1: Cue Engine

**Delivers:** Define cue points on the timeline that auto-switch effect presets and control loops.

| Task | Crate | Req | Description |
|------|-------|-----|-------------|
| 2.1.1 | — | FR-M6-01 | Create rifflab-cue crate (or module in rifflab-audio) with Cue, CuePosition, CueAction types |
| 2.1.2 | — | FR-M6-03 | CueEngine.tick(): subscribe to transport position, dispatch actions when playhead crosses a cue |
| 2.1.3 | — | FR-M6-04 | EffectPresetSwitch cues → trigger M2's crossfade_to_preset() |
| 2.1.4 | — | FR-M6-05 | LoopRegion cues → instruct transport to loop |
| 2.1.5 | — | FR-M6-02 | Cue positions as (bar, beat) when beat grid available, else absolute time |
| 2.1.6 | — | FR-M6-06 | Persist cues per-song to library/songs/{id}/cues.json |

**Exit criteria:** Define cue points → play through → effect preset switches automatically at cue positions. Cues survive app restart.

### Milestone 2.2: Crossfade Preset Switching + Phase 2 Effects

**Delivers:** Smooth effect transitions. Additional effects: delay, chorus, flanger, phaser, cabinet sim.

| Task | Crate | Req | Description |
|------|-------|-----|-------------|
| 2.2.1 | rifflab-fx | FR-M2-05 | Parameter crossfade engine: 10ms default fade on any parameter change |
| 2.2.2 | rifflab-fx | FR-M2-06 | crossfade_to_preset(): blend full chain state over configurable duration |
| 2.2.3 | rifflab-fx | FR-M2-03 | Implement chorus effect (LFO-modulated delay) |
| 2.2.4 | rifflab-fx | FR-M2-03 | Implement phaser effect (LFO-modulated allpass cascade) |
| 2.2.5 | rifflab-fx | FR-M2-03 | Implement flanger effect (LFO-modulated short delay) |
| 2.2.6 | rifflab-fx | FR-M2-03 | Implement delay effect (tempo-synced and free, feedback, mix) |
| 2.2.7 | rifflab-fx | FR-M2-03 | Implement cabinet simulation (convolution with impulse response) |

**Exit criteria:** Crossfade between presets is audibly smooth (no clicks). All Phase 2 effects produce correct output (verified with test signals).

### Milestone 2.3: Rhai Scripted Effects

**Delivers:** Users can write custom effects in Rhai scripts that appear in the effect chain.

| Task | Crate | Req | Description |
|------|-------|-----|-------------|
| 2.3.1 | rifflab-fx | FR-M2-10 | Add rhai dependency, implement ScriptedEffect wrapper (loads .rhai, calls fn process/params/name) |
| 2.3.2 | rifflab-fx | FR-M2-11 | Pre-compile scripts to AST at load time (no parsing in audio thread) |
| 2.3.3 | rifflab-fx | FR-M2-12 | Sandbox: max_operations, max_call_stack_depth; bypass + log on overrun |
| 2.3.4 | rifflab-fx | FR-M2-13 | Scan library/effects/*.rhai on startup, register in EffectRegistry |
| 2.3.5 | rifflab-fx | FR-M2-09 | Update EffectRegistry to serve both built-in and scripted effects |

**Exit criteria:** Write a .rhai effect script → restart app → effect appears in registry → add to chain → hear processed audio → tweak params in UI.

### Milestone 2.4: Session History + Progress Tracking

**Delivers:** Practice sessions persisted to SQLite. Historical progress visible per song.

| Task | Crate | Req | Description |
|------|-------|-----|-------------|
| 2.4.1 | rifflab-practice | FR-M5-08 | Persist SessionResult + per-note NoteResult to SQLite on session end |
| 2.4.2 | rifflab-practice | FR-M5-09 | Query historical sessions per song, aggregate by section |
| 2.4.3 | rifflab-app | — | Initialize SQLite schema on first run (songs, sessions, note_results tables) |

**Exit criteria:** Complete a practice session → data in SQLite → query shows history with per-session scores and trends.

### Milestone 2.5: 6-Stem Separation + Model Selection

**Delivers:** Demucs 6-stem model support. User chooses 4-stem or 6-stem.

| Task | Crate | Req | Description |
|------|-------|-----|-------------|
| 2.5.1 | workers/ | FR-M3-02 | Update demucs_worker.py to accept model parameter (htdemucs or htdemucs_6s) |
| 2.5.2 | rifflab-stems | FR-M3-02 | Pass model selection through StemSeparator to worker |
| 2.5.3 | rifflab-ui | — | Model selection dropdown in import dialog |

**Exit criteria:** Import with 6-stem model → 6 stem WAV files. Guitar and piano stems visible as separate tracks.

### Milestone 2.6: UI — Effects Editor + Accuracy View + Cue Lane

**Delivers:** Full effects editor with auto-generated controls. Accuracy history plots. Cue lane in arrangement.

| Task | Crate | Req | Description |
|------|-------|-----|-------------|
| 2.6.1 | rifflab-ui | FR-M7-13 | Effects Editor view: left-to-right signal flow, click to expand auto-generated parameter controls from EffectDescriptor |
| 2.6.2 | rifflab-ui | FR-M7-13 | Drag-to-reorder effects in chain |
| 2.6.3 | rifflab-ui | FR-M7-14 | "Add Effect" dialog: list effects from EffectRegistry grouped by Built-in / User Script |
| 2.6.4 | rifflab-ui | FR-M7-15 | "Edit Script" action for Rhai effects → open .rhai in system editor |
| 2.6.5 | rifflab-ui | FR-M7-12 | Accuracy view: pitch deviation time-series (±50¢), timing deviation per onset, aggregate stats, historical comparison ghost line |
| 2.6.6 | rifflab-ui | FR-M7-11 | Cue lane in arrangement view: color-coded markers for sections, effect switches, loop regions |
| 2.6.7 | rifflab-ui | — | Implement knob and fader widgets for effect parameters |

**Exit criteria:** Open effects editor → see signal chain → click effect → sliders/toggles match param descriptors → add/remove/reorder effects. Accuracy view shows post-session analysis. Cue markers visible and clickable.

### Milestone 2.7: Tempo Override + Sinusoidal Decomposition

**Delivers:** Slow down playback for practice. Better pitch analysis.

| Task | Crate | Req | Description |
|------|-------|-----|-------------|
| 2.7.1 | rifflab-audio | FR-M1-10 | Playback rate adjustment with pitch correction (time-stretch without pitch shift) |
| 2.7.2 | rifflab-analysis | FR-M4-10 | Sinusoidal decomposition: STFT → peak picking → partial tracking (McAulay-Quatieri) → harmonic grouping → note segmentation |

**Exit criteria:** Slow playback to 75% → pitch stays correct. Sinusoidal decomposition produces more accurate NoteEvents than simplified transcription.

---

## Phase 3 — Advanced Features

**Goal:** Deep editing, extended instrument support, plugin hosting.

### Milestone 3.1: LV2 Plugin Hosting

**Delivers:** Load external LV2 plugins as effect nodes.

| Task | Crate | Req | Description |
|------|-------|-----|-------------|
| 3.1.1 | rifflab-fx | FR-M2-07 | LV2 host integration: scan for installed LV2 plugins, instantiate, wrap as EffectDescriptor |
| 3.1.2 | rifflab-fx | FR-M2-07 | Map LV2 port metadata to ParamDescriptor for auto-generated UI |
| 3.1.3 | rifflab-fx | — | Convolution cabinet with IR file import |

**Exit criteria:** System LV2 plugin appears in effect registry → add to chain → hear processed audio → parameters editable in UI.

### Milestone 3.2: Polyphonic Pitch Detection + MIDI Export

**Delivers:** Detect chords. Export detected notes as MIDI.

| Task | Crate | Req | Description |
|------|-------|-----|-------------|
| 3.2.1 | rifflab-analysis | FR-M4 (Phase 3) | Polyphonic pitch detection (multiple simultaneous notes) |
| 3.2.2 | rifflab-analysis | FR-M4 (Phase 3) | Export Vec<NoteEvent> as Standard MIDI File |

**Exit criteria:** Detect a chord → multiple NoteEvents at same onset. Export MIDI → opens correctly in external DAW.

### Milestone 3.3: Adaptive Practice + Note Editor

**Delivers:** Smart practice features. Manual note correction UI.

| Task | Crate | Req | Description |
|------|-------|-----|-------------|
| 3.3.1 | rifflab-practice | FR-M5 (Phase 3) | Difficulty curve analysis: identify sections with lowest accuracy scores |
| 3.3.2 | rifflab-practice | FR-M5 (Phase 3) | Adaptive tempo: auto-slow difficult passages, gradually increase on improvement |
| 3.3.3 | rifflab-ui | FR-M7 (Phase 3) | Note editor: select, move, resize, delete, add notes in piano roll |
| 3.3.4 | rifflab-ui | FR-M7 (Phase 3) | Drag-to-reassign: move notes between stems |
| 3.3.5 | — | FR-M6 (Phase 3) | Tempo ramp cues (gradual tempo increase between two cue points) |

**Exit criteria:** App identifies difficult section → auto-loops it at reduced tempo → tempo increases as player improves. Notes editable in piano roll.

---

## Milestone Dependency Graph

```
Phase 1:
  1.1 Audio Playback
    │
    ├──► 1.2 Stem Separation ──► 1.3 Beat Tracking + Transcription
    │                                     │
    ├──► 1.4 Effects + Descriptor         │
    │         │                           │
    │         └──────────┐                │
    │                    ▼                ▼
    │              1.5 Practice Comparison
    │                    │
    ├──► 1.6 UI Arrangement + Sidebar     │
    │         │                           │
    │         └──► 1.7 UI Piano Roll ◄────┘
    │                    │
    └────────────────────┴──► 1.8 Import Workflow + Wiring
                                     │
                                     ▼
                               ══ MVP RELEASE ══

Phase 2:
  2.1 Cue Engine ──────────┐
  2.2 Crossfade + Effects ─┤
  2.3 Rhai Scripting ──────┤
  2.4 Session History ─────┼──► 2.6 UI (Effects Editor + Accuracy + Cues)
  2.5 6-Stem ─────────────┘
  2.7 Tempo Override + Sinusoidal (independent)

Phase 3:
  3.1 LV2 Plugins
  3.2 Polyphonic + MIDI Export
  3.3 Adaptive Practice + Note Editor
```

---

## Verification Gates

Each milestone has exit criteria above. Additionally:

| Gate | Scope | Criteria |
|------|-------|---------|
| **Phase 1 Gate** | Full MVP | `cargo test --workspace` passes. Manual test: import MP3, separate, play along, see accuracy in piano roll, loop a section. NFR-USE-01 met. |
| **Phase 2 Gate** | Automation + Polish | All Phase 2 FR-* requirements verified per SRS verification column. Cue-based preset switching works hands-free. Session history queryable. Rhai script loads and processes audio. |
| **Phase 3 Gate** | Advanced Features | LV2 plugin loads. Polyphonic detection on a chord. MIDI export opens in external DAW. Adaptive tempo adjusts based on performance. |
