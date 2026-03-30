# RiffLab Cleanup Plan

Status: Draft
Date: 2026-03-30

## Problem Statement

The `rifflab-app` crate has become a monolithic integration point. `main.rs` is ~6,000 lines with an 80+ field god object (`RiffLabApp`) and a single `update()` method handling all application logic. Supporting crates have secondary issues (inconsistent errors, magic numbers, tight coupling) but the app crate is the bottleneck for maintainability.

---

## Phase 1 — Extract Subsystems from RiffLabApp

**Goal:** Break the 80+ field struct into focused subsystems. Each subsystem owns its state and exposes a small API. `RiffLabApp` becomes a thin coordinator.

### 1.1 Extract MidiController

**Files:** `main.rs` → `midi_controller.rs` (new)

Move these fields into `MidiController`:
- `midi_connection`, `midi_rx`, `midi_port_names`, `midi_selected_port`
- `midi_knob_ccs`, `midi_knob_learning`, `midi_knob_last`
- `midi_fader_ccs`, `midi_fader_learning`
- `midi_learn_node`, `midi_learn`

Move `MidiMapping` (main.rs:19-54) into `midi_controller.rs` — it already handles persistence for MIDI device mappings and has no reason to live in main.

API surface:
```rust
impl MidiController {
    fn poll_events(&mut self) -> Vec<MidiEvent>;
    fn connect(&mut self, port_index: usize);
    fn start_learn(&mut self, target: MidiLearnTarget);
    fn stop_learn(&mut self);
    fn save_mapping(&self);
    fn load_mapping(&mut self, port_name: &str);
}
```

### 1.2 Extract EffectsManager

**Files:** `main.rs` → `effects_manager.rs` (new)

Move these fields:
- `fx_graph`, `node_editor_state`
- `fx_snapshot`, `fx_mb_snapshot`, `fx_snapshot_time`, `fx_snapshot_dirty`

Replace manual dirty flag with version counter or change-detection on the graph itself.

API surface:
```rust
impl EffectsManager {
    fn graph(&self) -> &FxGraph;
    fn graph_mut(&mut self) -> &mut FxGraph;
    fn snapshot_if_dirty(&mut self) -> Option<&[FxSnapCached]>;
    fn draw_node_editor(&mut self, ui: &mut egui::Ui);
}
```

### 1.3 Extract TabManager

**Files:** `main.rs` → `tab_manager.rs` (new)

Move these fields:
- `tab_document`, `tab_view_state`
- `tab_parse_rx`, `tab_parse_log_rx`

Encapsulate the background parse thread spawn + polling.

API surface:
```rust
impl TabManager {
    fn start_parse(&mut self, text: &str, tuning: [u8; 4]);
    fn poll(&mut self) -> Option<TabDocument>;
    fn document(&self) -> Option<&TabDocument>;
    fn draw(&mut self, ui: &mut egui::Ui);
}
```

### 1.4 Extract SessionManager

**Files:** `main.rs` → `session_manager.rs` (new)

Move these fields:
- `session_path`, `original_file_path`
- `save_receiver`, `save_dialog_rx`

Encapsulate save/load threading so `main.rs` never spawns session threads directly.

API surface:
```rust
impl SessionManager {
    fn save(&mut self, app_state: &AppSnapshot);
    fn save_as(&mut self);
    fn load(&mut self, path: &Path) -> Result<AppSnapshot>;
    fn poll_save(&mut self) -> Option<SaveResult>;
    fn poll_dialog(&mut self) -> Option<PathBuf>;
}
```

### 1.5 Group Remaining State

Group simple related fields into plain structs (no complex API needed):

```rust
struct MeteringState { peak_l: f32, peak_r: f32, rms_l: f32, rms_r: f32 }
struct TunerState { note: u8, cents: f32, confidence: f32, hold_count: u32, ... }
struct ViewState { frames_per_pixel: f64, scroll_offset_frames: f64, ... }
```

### 1.6 Resulting RiffLabApp Shape

```rust
struct RiffLabApp {
    engine: Arc<Mutex<AudioEngine>>,
    midi: MidiController,
    effects: EffectsManager,
    tabs: TabManager,
    session: SessionManager,
    metering: MeteringState,
    tuner: TunerState,
    view: ViewState,
    preset_bank: PresetBank,
    preset_nav: PresetGraph,
    config: AppConfig,
    // ... remaining UI-only fields
}
```

---

## Phase 2 — Split update() Into Methods

**Goal:** Break the monolithic `update()` into focused methods called sequentially.

### 2.1 Extract Poll Methods

```rust
impl RiffLabApp {
    fn poll_background(&mut self) {
        self.poll_file_load();
        self.session.poll_save();
        self.session.poll_dialog();
        self.tabs.poll();
    }

    fn poll_audio_state(&mut self) {
        self.drain_meters();
        self.read_transport();
        self.tick_cue_engine();
    }
}
```

### 2.2 Extract Input Handling

```rust
impl RiffLabApp {
    fn handle_midi(&mut self) {
        let events = self.midi.poll_events();
        for event in events {
            self.route_midi_event(event);
        }
    }

    fn handle_pitch(&mut self, frame: PitchFrame) {
        self.tuner.update(frame);
        if let Some(ref mut comparator) = self.comparator {
            comparator.process(frame);
        }
    }
}
```

### 2.3 Extract UI Rendering

Split the UI portion of `update()` into panel methods:

```rust
impl RiffLabApp {
    fn draw_top_bar(&mut self, ui: &mut egui::Ui);
    fn draw_transport(&mut self, ui: &mut egui::Ui);
    fn draw_waveform(&mut self, ui: &mut egui::Ui);
    fn draw_stem_mixer(&mut self, ui: &mut egui::Ui);
    fn draw_effects_panel(&mut self, ui: &mut egui::Ui);
    fn draw_tab_panel(&mut self, ui: &mut egui::Ui);
    fn draw_settings_dialog(&mut self, ui: &mut egui::Ui);
}
```

### 2.4 Resulting update() Shape

```rust
fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
    self.poll_background();
    self.poll_audio_state();
    self.handle_midi();
    self.handle_pitch();

    egui::TopBottomPanel::top("top").show(ctx, |ui| self.draw_top_bar(ui));
    egui::CentralPanel::default().show(ctx, |ui| {
        self.draw_transport(ui);
        self.draw_waveform(ui);
        self.draw_stem_mixer(ui);
        self.draw_effects_panel(ui);
        self.draw_tab_panel(ui);
    });

    if self.show_settings {
        self.draw_settings_dialog(ctx);
    }
}
```

---

## Phase 3 — Standardize Error Handling

**Goal:** One error strategy per layer.

### 3.1 Library Crates: `thiserror` Enums

Each library crate defines its own error enum:

```rust
// rifflab-tab/src/error.rs
#[derive(Debug, thiserror::Error)]
pub enum TabError {
    #[error("parse failed: {0}")]
    Parse(String),
    #[error("LLM API error: {0}")]
    LlmApi(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
```

Crates to add error types: `rifflab-tab`, `rifflab-ipc`, `rifflab-audio`, `rifflab-stems`.

### 3.2 App Crate: `anyhow::Result`

The app crate uses `anyhow` for all fallible operations. Library errors convert via `From` impls.

### 3.3 Fix Panic-Risk unwrap() Calls

| Location | Issue | Fix |
|----------|-------|-----|
| `audio.rs:99` | `partial_cmp().unwrap()` | `.unwrap_or(Ordering::Equal)` |
| `reference.rs:68` | `partial_cmp().unwrap()` | `.unwrap_or(Ordering::Equal)` |
| `midi_input.rs:39` | Returns `Result<T, String>` | Return `Result<T, MidiError>` |

---

## Phase 4 — Move Stem Separation Out of main.rs

**Goal:** The 200-line `load_file()` method with 3 nested thread spawns belongs in `rifflab-stems`.

### 4.1 Create StemLoader in rifflab-stems

```rust
// rifflab-stems/src/loader.rs
pub struct StemLoader { ... }

impl StemLoader {
    /// Spawns background thread. Returns receiver for progress/completion.
    pub fn load(path: &Path) -> (StemLoadHandle, mpsc::Receiver<StemLoadMsg>);
}

pub enum StemLoadMsg {
    Progress { stage: &'static str, pct: f32 },
    DemucsLog(String),
    Done(Result<Vec<StemTrack>, StemError>),
}
```

### 4.2 Simplify main.rs load_file()

```rust
fn load_file(&mut self, path: &Path) {
    let (handle, rx) = StemLoader::load(path);
    self.stem_load_handle = Some(handle);
    self.stem_load_rx = Some(rx);
}
```

---

## Phase 5 — Clean Up rifflab-tab

### 5.1 Extract Constants

Create `crates/rifflab-tab/src/constants.rs`:

```rust
pub const MAX_FRET: u8 = 24;
pub const DEFAULT_BPM: f64 = 120.0;
pub const MIN_CONFIDENCE: f32 = 0.3;
pub const DEFAULT_CONFIDENCE: f32 = 0.8;
pub const CONFIDENCE_DAMPING: f32 = 0.7;
pub const LLM_INPUT_TRUNCATE: usize = 51200;
pub const DTW_MIN_WINDOW: usize = 5;
```

### 5.2 Deduplicate strip_markdown()

Move to a shared location (e.g., `crates/rifflab-tab/src/util.rs`) and have both `ascii.rs` and `ascii_llm.rs` import it.

### 5.3 Remove Dead Code

- Delete `all_notes_push_dup()` — replace with direct `notes.push(note)`
- Remove or use items marked `#[allow(dead_code)]`: `ImportResult`, `MultibandSnapshot`, `SpectrogramDisplay`, `PresetBank::len()`, session fields

### 5.4 Create TabError Type

Replace `String` and `Box<dyn Error>` returns with `TabError` enum (see Phase 3.1).

---

## Phase 6 — Audio Engine Fixes

### 6.1 MXCSR RAII Guard

```rust
struct DenormalGuard { saved: u32 }
impl DenormalGuard {
    fn new() -> Self { /* stmxcsr, set FTZ+DAZ, return saved */ }
}
impl Drop for DenormalGuard {
    fn drop(&mut self) { /* ldmxcsr saved */ }
}
```

### 6.2 EffectRegistry → HashMap

Change `Vec<EffectFactory>` to `HashMap<String, EffectFactory>` for O(1) lookup and duplicate prevention.

### 6.3 Break Audio↔Analysis Coupling

Introduce a trait in `rifflab-core`:

```rust
pub trait PitchDetector: Send {
    fn detect(&mut self, samples: &[f32]) -> Option<PitchFrame>;
    fn set_sample_rate(&mut self, rate: u32);
}
```

`rifflab-audio` depends on the trait, not on `rifflab-analysis` directly. The app wires the concrete `YinDetector` at startup.

---

## Execution Order

| Step | Phase | Scope | Risk |
|------|-------|-------|------|
| 1 | 1.1 | Extract MidiController | Low — isolated state |
| 2 | 1.2 | Extract EffectsManager | Medium — UI integration |
| 3 | 1.3 | Extract TabManager | Low — isolated state |
| 4 | 1.4 | Extract SessionManager | Low — isolated state |
| 5 | 1.5 | Group remaining fields | Low — struct renames only |
| 6 | 2.1-2.4 | Split update() | Medium — must preserve call order |
| 7 | 3.1-3.3 | Error standardization | Low — mechanical |
| 8 | 4.1-4.2 | Move stem loading | Medium — thread management |
| 9 | 5.1-5.4 | rifflab-tab cleanup | Low — isolated crate |
| 10 | 6.1-6.3 | Audio engine fixes | High — real-time safety |

Each step should compile and pass `cargo test --workspace` before moving to the next. Steps 1-5 are independent and can be done in any order. Steps within Phase 1 can be parallelized across branches if needed.
