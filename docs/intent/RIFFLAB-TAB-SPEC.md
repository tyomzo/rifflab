# RiffLab Tab Notation System — Implementation Specification

**Document:** RIFFLAB-TAB-SPEC  
**Version:** 1.0.0  
**Date:** 2026-03-28  
**Status:** DRAFT  
**Parent:** RiffLab PRD  

---

## 1. Purpose

This specification defines the Tab Notation System for RiffLab — a pipeline that ingests bass tablature from multiple sources (ASCII text, audio analysis, or both), produces a structured internal representation, and renders interactive, playback-synced notation in the UI.

The system must work in three input modes:

1. **Audio-only** — user provides a track; Demucs separates the bass stem; ML transcription produces note events. No tab needed.
2. **ASCII tab import** — user pastes or uploads an ASCII bass tab; LLM-based parser extracts structured note data.
3. **Fused** — both audio and ASCII tab are available; cross-validation produces the highest-accuracy result.

---

## 2. Data Model

### 2.1 Core Types

```rust
/// Represents a single note event in a bass tab.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabNote {
    /// Unique ID for this note event.
    pub id: Uuid,
    /// String index: 0=E (lowest), 1=A, 2=D, 3=G.
    pub string: u8,
    /// Fret number (0 = open string, max 24).
    pub fret: u8,
    /// Onset time in seconds from track start.
    pub time_secs: f64,
    /// Duration in seconds.
    pub duration_secs: f64,
    /// Beat position (1-indexed beat within the measure).
    pub beat: Option<f64>,
    /// Measure number (1-indexed).
    pub measure: Option<u32>,
    /// Playing technique, if detected.
    pub technique: Option<Technique>,
    /// Confidence score from transcription/parsing (0.0–1.0).
    pub confidence: f32,
    /// Source that produced this note event.
    pub source: NoteSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Technique {
    Normal,
    HammerOn,
    PullOff,
    SlideUp,
    SlideDown,
    Slap,
    Pop,
    Mute,
    Bend,
    Vibrato,
    Ghost,       // ghost note (parenthesized in ASCII)
    Harmonic,    // natural or artificial harmonic
    TapOn,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NoteSource {
    /// Parsed from ASCII tab text.
    AsciiParse,
    /// ML transcription from audio.
    AudioTranscription,
    /// Fused from both ASCII and audio sources.
    Fused,
    /// Manually edited by user.
    UserEdit,
}
```

### 2.2 Tab Document

```rust
/// A complete tab for a single track/song.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabDocument {
    pub id: Uuid,
    pub title: String,
    pub artist: Option<String>,
    /// Tuning as MIDI note numbers for each string, low to high.
    /// Standard 4-string bass: [28, 33, 38, 43] (E1, A1, D2, G2).
    /// Always 4 elements. Alternative tunings (drop D, etc.) change values.
    pub tuning: [u8; 4],
    /// Tempo in BPM. May be a single value or a tempo map.
    pub tempo: TempoMap,
    /// Time signature.
    pub time_signature: TimeSignature,
    /// All note events, ordered by time_secs.
    pub notes: Vec<TabNote>,
    /// Measure markers (start time of each measure).
    pub measures: Vec<MeasureMarker>,
    /// Metadata about how this tab was produced.
    pub provenance: TabProvenance,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TempoMap {
    /// BPM at the start of the track.
    pub initial_bpm: f64,
    /// Tempo changes: (time_secs, new_bpm). Empty if constant tempo.
    pub changes: Vec<(f64, f64)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeSignature {
    pub beats_per_measure: u8,
    pub beat_unit: u8,  // 4 = quarter note, 8 = eighth note
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeasureMarker {
    pub measure_number: u32,
    pub time_secs: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabProvenance {
    /// Source of the ASCII tab (URL, filename, "pasted").
    pub ascii_source: Option<String>,
    /// Audio file used for transcription.
    pub audio_source: Option<String>,
    /// Pipeline mode used to produce this tab.
    pub mode: PipelineMode,
    /// Timestamp of creation.
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PipelineMode {
    AudioOnly,
    AsciiOnly,
    Fused,
}
```

### 2.3 Serialization Format

Tabs are persisted as JSON files with extension `.rltab` (RiffLab Tab). The format is a direct JSON serialization of `TabDocument`. Example:

```
~/.rifflab/tabs/{uuid}.rltab
```

---

## 3. Pipeline Architecture

### 3.1 Overview

```
┌─────────────────────────────────────────────────────────┐
│                    Tab Pipeline                         │
│                                                         │
│  ┌──────────────┐          ┌────────────────────────┐   │
│  │ ASCII Import │          │   Audio Transcription   │   │
│  │              │          │                         │   │
│  │  Raw text    │          │  Audio file / stem      │   │
│  │     │        │          │       │                 │   │
│  │     ▼        │          │       ▼                 │   │
│  │  LLM Parser  │          │  Demucs (stem sep.)    │   │
│  │     │        │          │       │                 │   │
│  │     ▼        │          │       ▼                 │   │
│  │  TabNote[]   │          │  Basic Pitch / YIN      │   │
│  │  (no timing) │          │       │                 │   │
│  │              │          │       ▼                 │   │
│  │              │          │  Onset + Pitch + Beat   │   │
│  │              │          │       │                 │   │
│  │              │          │       ▼                 │   │
│  │              │          │  TabNote[] (timed)      │   │
│  └──────┬───────┘          └───────────┬────────────┘   │
│         │                              │                │
│         └──────────┬───────────────────┘                │
│                    ▼                                    │
│           ┌────────────────┐                            │
│           │ Alignment &    │                            │
│           │ Fusion Engine  │                            │
│           │ (DTW + merge)  │                            │
│           └───────┬────────┘                            │
│                   ▼                                     │
│            TabDocument (.rltab)                         │
└─────────────────────────────────────────────────────────┘
```

### 3.2 Module: ASCII Import (`rifflab-tab-ascii`)

**Responsibility:** Accept raw ASCII bass tab text and produce a sequence of `TabNote` structs with string, fret, technique, and relative ordering — but without accurate absolute timing.

**Implementation approach:** LLM-based parsing via Anthropic Claude API.

**Rationale:** ASCII tab formatting is wildly inconsistent across sources (variable spacing, inconsistent technique notation, missing/extra lines, annotations mixed with notation, partial tabs). A rule-based parser would require hundreds of heuristics. An LLM handles format variation naturally. Tab parsing is a one-time import operation (not real-time), so API latency is acceptable.

#### 3.2.1 LLM Parsing Contract

**Input:** Raw ASCII tab text (string, max 50KB).

**System prompt:**

```
You are a bass guitar tablature parser. You receive ASCII bass tab and
output a JSON array of note events.

Rules:
- Bass tab has 4 lines labeled G, D, A, E (top to bottom = highest to
  lowest string). Sometimes labels are lowercase or missing — infer from
  context (4 parallel lines of dashes and numbers).
- Each number on a line is a fret number. Multi-digit frets (10-24) are
  two adjacent digits.
- Dashes (-) are rests/sustain.
- Technique notation:
  h = hammer-on (between two fret numbers)
  p = pull-off (between two fret numbers)
  / = slide up
  \ = slide down
  x = muted note (dead note)
  () = ghost note
  * = harmonic
  ~ = vibrato
  b = bend
  t = tap
  s = slap (sometimes)
- Notes at the same horizontal position across strings are simultaneous
  (chord/double stop).
- Preserve the sequential order of notes as they appear left-to-right.
- Assign each note a sequential `position` integer (0-indexed) representing
  its left-to-right order. Notes at the same horizontal position share the
  same `position` value.
- If the tab contains section labels (Intro, Verse, Chorus, etc.),
  include them as `section` on the first note of that section.
- If you encounter notation you cannot parse, skip it and continue.

Output ONLY a JSON array with no markdown fencing, no explanation:
[
  {
    "string": 0,        // 0=E, 1=A, 2=D, 3=G
    "fret": 5,
    "position": 0,      // sequential left-to-right position
    "technique": null,   // or "hammer_on", "pull_off", "slide_up",
                         //    "slide_down", "mute", "ghost",
                         //    "harmonic", "bend", "vibrato", "tap"
    "section": null      // or "Intro", "Verse", etc.
  }
]
```

**User prompt:** The raw ASCII tab text, verbatim.

**Parsing the response:**
1. Strip any markdown code fences if present (`\`\`\`json`, `\`\`\``).
2. Parse as `Vec<AsciiNote>` where `AsciiNote` maps to the JSON schema above.
3. Convert to `Vec<TabNote>` with `source: NoteSource::AsciiParse`, `confidence: 0.8` (default; no timing info yet), and `time_secs`/`duration_secs` set to `0.0` (to be filled by alignment).
4. On parse failure: retry once with a simplified prompt. On second failure: fall back to a rule-based parser (see §3.2.2).

#### 3.2.2 Rule-Based Fallback Parser

A basic rule-based parser as fallback when LLM parsing fails or is unavailable (offline mode).

**Algorithm:**
1. Split input into lines. Identify groups of 4 consecutive lines that match the pattern `^[GgDdAaEe]?\|?[-0-9hpx/\\()~*tb\s]+$`.
2. Within each 4-line group, scan left-to-right character by character.
3. At each column position, check all 4 lines for digit characters. Multi-digit: if `line[col]` is a digit and `line[col+1]` is also a digit, combine them.
4. Emit a `TabNote` for each detected fret number with the appropriate string index.
5. Technique detection: check characters immediately before/after/between fret numbers for `h`, `p`, `/`, `\`, `x`, `(`, `)`, `*`.
6. `position` is the column index (normalized to remove leading whitespace).

**Limitations:** This fallback will not handle unusual formatting, missing string labels, or inconsistent spacing. It exists as a safety net, not a primary path.

#### 3.2.3 Configuration

```toml
[tab.ascii_import]
# Use LLM for parsing (recommended). Falls back to rule-based if false or API unavailable.
use_llm = true
# Anthropic API model for parsing.
llm_model = "claude-sonnet-4-20250514"
# Max input size in bytes.
max_input_bytes = 51200
# Default confidence score for ASCII-parsed notes.
default_confidence = 0.8
```

### 3.3 Module: Audio Transcription (`rifflab-tab-audio`)

**Responsibility:** Given an audio file (full mix or pre-separated bass stem), produce a timed sequence of `TabNote` structs with pitch, onset, duration, and beat information.

#### 3.3.1 Sub-pipeline

```
Audio input
    │
    ├─ Is full mix? ──yes──▶ Demucs stem separation ──▶ bass stem
    │                                                       │
    └─ Is bass stem? ──────────────────────────────────────▶│
                                                            ▼
                                                   Basic Pitch (ONNX)
                                                            │
                                                            ▼
                                                  Raw note events:
                                                  (onset, offset, pitch_hz, confidence)
                                                            │
                                                            ▼
                                                   Beat Tracker (aubio)
                                                            │
                                                            ▼
                                                   Pitch → Fret Mapper
                                                            │
                                                            ▼
                                                   TabNote[] with timing
```

#### 3.3.2 Demucs Integration

RiffLab already includes Demucs for stem separation (per PRD). The tab pipeline reuses the same stem separation service.

**Interface:**
```rust
/// Request bass stem extraction. Returns path to the separated bass audio file.
pub async fn extract_bass_stem(audio_path: &Path) -> Result<PathBuf, StemError>;
```

If the user has already separated stems for this track (e.g., for practice playback), reuse the cached stem. Do not re-run Demucs.

#### 3.3.3 Note Transcription — Basic Pitch (Primary)

**What:** Spotify's Basic Pitch is a lightweight CNN for monophonic/low-polyphony audio-to-MIDI transcription. Bass guitar is an ideal use case.

**Integration strategy:** Run the Basic Pitch model via ONNX Runtime from Rust. Model is downloaded on first use and cached locally.

**Steps:**
1. Export Basic Pitch's TensorFlow model to ONNX format (one-time, done by maintainers; hosted as a release artifact).
2. On first transcription request, check for cached model at `~/.rifflab/models/basic_pitch.onnx`. If absent, download from configured URL with progress indicator in UI. If download fails (offline), fall back to YIN (see below).
3. Load the ONNX model via `ort` crate (Rust ONNX Runtime bindings).
4. Preprocess: resample bass stem to 22050 Hz mono, compute CQT or mel spectrogram per Basic Pitch's input spec.
5. Inference: run the model, get per-frame note activation, onset, and pitch predictions.
6. Post-process: threshold activations, extract note events as `(onset_secs, offset_secs, midi_note, confidence)`.

**Output struct (intermediate):**
```rust
pub struct TranscribedNote {
    pub onset_secs: f64,
    pub offset_secs: f64,
    pub midi_note: u8,
    pub confidence: f32,
}
```

**Fallback — YIN pitch detection:** If Basic Pitch ONNX is unavailable or produces poor results, fall back to onset detection (aubio spectral flux) + YIN pitch estimation (aubio or custom). This gives per-onset pitch estimates without the full CNN transcription. Less accurate but zero external model dependency.

#### 3.3.4 Beat Tracking

Use `aubio`'s beat tracker (via FFI or the `aubio-rs` crate) on the bass stem to produce:
- Tempo (BPM), possibly varying over time.
- Beat timestamps (downbeat positions).
- Measure boundaries (derived from beats + time signature).

**Output:**
```rust
pub struct BeatGrid {
    pub bpm: f64,
    pub beat_times: Vec<f64>,      // timestamp of each beat
    pub downbeat_indices: Vec<usize>, // which beat_times are downbeats
}
```

Assign each `TranscribedNote` a `beat` and `measure` value by snapping its onset to the nearest beat grid position.

#### 3.3.5 Pitch-to-Fret Mapping

A single pitch (MIDI note number) can be played at multiple string/fret positions on bass. For example, MIDI 43 (G2) = string 3 fret 0 = string 2 fret 5 = string 1 fret 10.

**Default heuristic:** Choose the position that minimizes hand movement from the previous note, preferring lower positions (closer to nut) when tied. This is a simple greedy algorithm.

**Future enhancement (Phase 2):** Train a small classifier on spectral features to distinguish which string was actually played. Different strings have different harmonic profiles for the same fundamental. Training data: user records known exercises (e.g., scales on specific strings). Model: random forest or small MLP on spectral features (first 10 harmonic ratios + spectral centroid + spectral rolloff). This is noted here for future implementation — not required for v1.

**Output:** Each `TranscribedNote` is converted to a `TabNote` with `string` and `fret` assigned.

#### 3.3.6 Configuration

```toml
[tab.audio_transcription]
# Model to use for note transcription.
model = "basic_pitch_onnx"  # or "yin_fallback"
# Basic Pitch ONNX model cache path (downloaded on first use).
onnx_model_path = "~/.rifflab/models/basic_pitch.onnx"
# Download URL for Basic Pitch ONNX model.
onnx_model_url = "https://github.com/spotify/basic-pitch/releases/..."  # exact URL TBD
# Onset detection threshold (0.0–1.0).
onset_threshold = 0.5
# Minimum note confidence to include.
min_confidence = 0.4
# Fret mapping strategy.
fret_mapper = "min_movement"  # or "spectral_classifier" (future)
```

### 3.4 Module: Alignment & Fusion Engine (`rifflab-tab-fusion`)

**Responsibility:** When both ASCII-parsed and audio-transcribed notes are available, align and merge them into a single high-accuracy `TabDocument`.

#### 3.4.1 Alignment via Dynamic Time Warping (DTW)

**Problem:** ASCII-parsed notes have sequence and fret/string info but no timing. Audio-transcribed notes have timing and pitch but potentially ambiguous string/fret assignments.

**Algorithm:**

1. **Convert both sequences to pitch sequences.** For ASCII notes: `pitch = tuning[string] + fret`. For audio notes: already have MIDI note.

2. **Apply DTW** on the two pitch sequences.
   - Distance metric: absolute pitch difference in semitones. Exact match = 0, octave error = 12, etc.
   - Standard DTW with Sakoe-Chiba band constraint (window = ±10% of sequence length) to prevent pathological warping.
   - Output: optimal alignment mapping `ascii_index → audio_index`.

3. **Merge aligned pairs:**
   - `time_secs` and `duration_secs` → from audio transcription (accurate timing).
   - `string` and `fret` → from ASCII parse (explicit fingering).
   - `technique` → from ASCII parse (technique annotations).
   - `confidence` → average of both sources' confidence, boosted by 0.1 if pitch matches exactly.
   - `source` → `NoteSource::Fused`.

4. **Handle unmatched notes:**
   - Audio note with no ASCII match: include it with `source: AudioTranscription`, lower confidence (×0.7). Likely a note the tab omitted (common for passing tones, ghost notes).
   - ASCII note with no audio match: include it with `source: AsciiParse`, low confidence (0.3). Likely a tab error or a note too quiet to detect.

5. **Conflict resolution:**
   - If aligned pitches differ by > 2 semitones: flag as conflict, keep both as alternatives, present to user for resolution in UI.
   - If techniques disagree: prefer ASCII annotation (human-authored intent) but attach the audio-derived spectral evidence for user review.

#### 3.4.2 Output

A fully populated `TabDocument` with all notes timed, assigned to strings/frets, and annotated with techniques. `provenance.mode` = `PipelineMode::Fused`.

#### 3.4.3 Single-Source Modes

- **Audio-only mode:** Skip DTW. Produce `TabDocument` from audio transcription alone. `fret_mapper` assigns string/fret. No technique annotations (unless spectral classifier is implemented).
- **ASCII-only mode:** Skip DTW. Assign synthetic timing: distribute notes evenly across an assumed tempo (default 120 BPM, user-adjustable). This produces a metronomic approximation usable for display but not for synced playback until the user provides audio.

### 3.5 Module: String/Fret Spectral Classifier (Future — Phase 2)

**Scope:** Not required for v1. Documented here for architecture awareness.

**Purpose:** Improve fret/string disambiguation in audio-only mode by classifying which string a note was played on based on spectral features.

**Approach:**
- Extract features per note onset window (~100ms): harmonic ratios (H1–H10 relative amplitudes), spectral centroid, spectral rolloff, spectral flatness, attack transient shape.
- Model: random forest or small 2-layer MLP. Input: ~15 features. Output: string index (0–3).
- **Training (guided):** RiffLab presents structured exercises — chromatic scale per string (frets 0–12), open strings, common patterns. User plays along; RiffLab records audio segments with known string/fret labels.
- **Training data persistence:** Raw audio clips + labels are saved at `~/.rifflab/training/string_classifier/` and survive across sessions, reinstalls, and model retrains. User records exercises once; data is reused indefinitely. Additional exercises can be recorded later to expand the dataset and improve accuracy (e.g., after changing strings, pickup settings, or instrument).
- **Retrain trigger:** Model is retrained automatically when new training data is added. Retrain is fast (~seconds for a random forest on this feature set).
- Expected accuracy: 85-95% based on published research on string identification from spectral features.

---

## 4. UI Rendering

### 4.1 Renderer Trait

```rust
/// Trait for tab rendering backends. All renderers receive the same
/// TabDocument and playback state, but produce different visual outputs.
pub trait TabRenderer {
    /// Initialize the renderer with a tab document.
    fn load(&mut self, tab: &TabDocument);

    /// Update playback position. Called every frame during playback.
    fn set_playback_position(&mut self, time_secs: f64);

    /// Render the current view. Implementation depends on the UI framework.
    fn render(&self, ctx: &mut RenderContext);

    /// Handle scroll/zoom input from the user.
    fn handle_input(&mut self, input: &TabInputEvent);

    /// Get the current viewport (visible time range).
    fn viewport(&self) -> (f64, f64);
}
```

### 4.2 View Modes

Three rendering modes, selectable by the user. Implement in priority order.

#### 4.2.1 Scrolling Tablature View (Primary — implement first)

The main practice view. A horizontal scrolling tablature with a fixed playhead.

**Layout:**
```
  Measure 1          |  Measure 2          |  Measure 3
  ─── playhead ──────┼─────────────────────┼──────────────
G ──────5──────7─────|──────────────────5──|────────────── 
D ────────────────3──|──5───7───5─────────|──3─────5───── 
A ──3──────────────--|──────────────3─────|──────────────  
E ───────────────────|──────────────────── |──3───────────  
```

**Specifications:**
- **Orientation:** Horizontal scroll. Time flows left to right.
- **Strings:** 4 horizontal lines, labeled on the left edge. G on top, E on bottom (standard tab convention).
- **Fret numbers:** Rendered as text at the (time, string) coordinate. Font: monospace, sized for readability at the current zoom level.
- **Playhead:** Vertical line at a fixed horizontal position (default: 25% from left edge, configurable). The tab scrolls behind it.
- **Measure dividers:** Vertical lines at measure boundaries, lighter weight than the playhead.
- **Beat markers:** Small tick marks on the top edge at each beat position.
- **Technique annotations:**
  - Hammer-on: curved arc above/below connecting two notes, labeled "H".
  - Pull-off: curved arc, labeled "P".
  - Slide: angled line connecting two notes.
  - Ghost note: fret number rendered with parentheses and reduced opacity.
  - Mute: "×" instead of fret number.
  - Harmonic: diamond shape around fret number.
  - Slap/Pop: "S"/"T" annotation above the note.
- **Active note highlighting:** The note currently at the playhead position is highlighted (color change + slight size increase).
- **Zoom:** Horizontal zoom controls how many measures are visible. Range: 1 measure (zoomed in) to 16 measures (zoomed out). Default: 4 measures visible.
- **Upcoming notes:** Notes ahead of the playhead are rendered at full opacity. Notes behind the playhead fade to 50% opacity.
- **Section labels:** Rendered above the staff at the start of each section ("Intro", "Verse", etc.).

**Scrolling behavior:**
- During playback: auto-scroll so the playhead stays at its fixed position.
- When paused: user can scroll freely (mouse wheel, touch drag, scroll bar).
- Click on a position in the tab → seek playback to that time.

#### 4.2.2 ASCII Text View (Secondary — simple to implement)

A styled monospace rendering of the tab in traditional ASCII format. Useful for users who prefer reading tabs in the classic format.

**Layout:** Standard 4-line ASCII tab rendered in a monospace font, with:
- A highlight/cursor that tracks the current playback position (background color behind the active column).
- Auto-scroll to keep the cursor visible.
- Color coding for techniques (optional).
- Measures separated by `|` characters.

**Generation:** Convert `TabDocument` to ASCII string by:
1. Quantize note positions to character grid (1 character = 1 sixteenth note at default resolution).
2. Fill dashes for empty positions.
3. Insert measure bars every N characters based on time signature.
4. Add technique characters between/around fret numbers.

This is a read-only view — no editing. It serves users who want the familiar format.

#### 4.2.3 Highway View (Tertiary — Phase 2)

A vertical scrolling "note highway" similar to Rocksmith/Guitar Hero. Notes scroll downward toward a hit zone at the bottom.

**Layout:**
- 4 vertical lanes (one per string), color-coded.
- Notes are rectangular blocks whose vertical length represents duration.
- Fret number is rendered inside the block.
- Notes scroll downward at a speed proportional to the current tempo.
- Hit zone at the bottom: a horizontal line where notes arrive at their play time.
- Accuracy feedback: notes change color (green/yellow/red) based on the user's played accuracy (when pitch analysis is active).

**This view is deferred to Phase 2.** It requires the accuracy analysis module (pitch comparison between played audio and expected notes) which is a separate PRD feature.

### 4.3 Color Scheme

Tab rendering must respect RiffLab's global theme (dark theme primary, per PRD).

```rust
pub struct TabColorScheme {
    pub background: Color,       // dark: #1a1a2e, light: #fafafa
    pub string_lines: Color,     // dark: #444466, light: #cccccc
    pub fret_number: Color,      // dark: #e0e0e0, light: #222222
    pub playhead: Color,         // accent color, e.g. #ff6b35
    pub active_note: Color,      // bright accent, e.g. #00e5ff
    pub measure_divider: Color,  // dark: #333355, light: #dddddd
    pub beat_tick: Color,        // dark: #2a2a4a, light: #eeeeee
    pub past_note: Color,        // faded: 50% opacity of fret_number
    pub technique_arc: Color,    // subtle: #888888
    pub section_label: Color,    // muted accent: #ffab40
    pub confidence_low: Color,   // warning: #ff5252 (for notes with confidence < 0.5)
}
```

### 4.4 Interaction

| Action | Behavior |
|--------|----------|
| Click on note | Select note; show detail panel (string, fret, technique, confidence, source). Note becomes immediately editable. |
| Click on note + type number | Replace fret number on the selected note (inline edit, no mode switch). |
| Click on empty position | Seek playback to that time. If a number key is pressed immediately after, insert a new note at that position. |
| Right-click on note | Context menu: change technique, delete note, view source provenance. |
| Drag note vertically | Move note to a different string (updates string + fret to maintain pitch, or changes pitch if Shift held). |
| Drag note horizontally | Adjust note timing (snaps to beat grid by default; hold Alt for free positioning). |
| Scroll wheel (horizontal) | Scroll timeline (when paused) |
| Scroll wheel (vertical) | Zoom in/out (when modifier key held) |
| Pinch gesture (touch) | Zoom in/out |
| Drag on background (touch) | Scroll timeline (when paused) |
| Keyboard: Space | Play/pause toggle |
| Keyboard: Left/Right | Step backward/forward by one beat |
| Keyboard: Ctrl+Left/Right | Step backward/forward by one measure |
| Keyboard: Delete/Backspace | Delete selected note |
| Keyboard: Tab | Select next note in sequence |
| Keyboard: Shift+Tab | Select previous note in sequence |

### 4.5 Note Editing

Notes are always editable inline — there is no separate "edit mode" toggle. The tablature is a live, interactive document at all times, including during playback.

**Design principle:** Editing should feel like working in a DAW piano roll. Select, modify, move — no mode switches, no dialogs for simple operations.

**Edit operations:**
- Change fret number: click a note to select it, then type a number. First digit replaces; if a second digit follows within 500ms, it forms a two-digit fret (e.g., type "1" then "2" → fret 12).
- Change string assignment: drag note vertically to a different string line.
- Change technique: right-click → context menu with technique options.
- Delete note: select + Delete/Backspace key.
- Insert note: click on an empty position on a specific string line, then type a fret number. The note is inserted at the nearest beat-grid position (or free position if Alt is held).
- Adjust timing: drag note horizontally. Snaps to the beat grid by default; hold Alt for free positioning.
- Multi-select: Shift+click to extend selection; Ctrl+click to toggle individual notes. Drag, delete, and technique changes apply to all selected notes.

**During playback:** Editing is still possible. Edits take effect immediately — if a note is changed while the playhead is approaching it, the updated note is what plays/displays. This allows real-time correction while listening.

**Undo/Redo:** Ctrl+Z / Ctrl+Shift+Z. Edit history is maintained per session. All edits are stored as a diff layer (see below).

**Non-destructive editing:** All user edits are stored as a diff layer over the original transcription data. The original `TabNote` entries (from ASCII parse, audio transcription, or fusion) are preserved with their original `source` and `confidence` values. User edits create new `TabNote` entries with `source: NoteSource::UserEdit` and `confidence: 1.0` that override the originals at render time. The user can "revert to original" on any individual note via right-click context menu.

---

## 5. Integration Points

### 5.1 Cue Engine Integration

The Tab Notation System integrates with RiffLab's Cue Engine (per PRD) for synchronized playback.

**Contract:**
- The Cue Engine provides a real-time playback position callback: `fn on_playback_tick(time_secs: f64)`.
- The Tab Renderer subscribes to this callback and calls `set_playback_position()` on each tick.
- When the user clicks a position in the tab, the Tab Renderer emits a `SeekRequest(time_secs)` event that the Cue Engine handles.
- Loop regions set in the Cue Engine are visually indicated on the tab (highlighted background between loop start/end).

### 5.2 Accuracy Analysis Integration

The accuracy analysis module (pitch/timing comparison) feeds back into the tab display:
- Notes the user played correctly: highlighted green.
- Notes the user played with wrong pitch: highlighted red, with the detected fret shown as a small annotation.
- Notes the user missed: highlighted dim red after the playhead passes them.
- Timing accuracy: subtle horizontal offset indicator (early = shifted left, late = shifted right).

This integration is defined here for architecture awareness. Implementation depends on the accuracy analysis module being available.

### 5.3 Stem Separation Integration

When the user imports a track and runs stem separation:
1. Bass stem becomes available.
2. Tab pipeline is automatically offered: "Generate tab from audio?" (user opt-in).
3. If the user also pastes/uploads an ASCII tab: "ASCII tab detected — merge with audio analysis for best accuracy?" → Fused mode.

### 5.4 Practice Session Integration

Tab data feeds into practice session recording:
- Which notes were attempted vs. skipped.
- Per-note accuracy over repeated practice runs.
- Section-level performance heatmaps ("you consistently miss this passage").

---

## 6. File I/O

### 6.1 Import Formats

| Format | Support | Method |
|--------|---------|--------|
| ASCII text (pasted) | v1 | LLM parser (§3.2) |
| ASCII text file (.txt, .tab) | v1 | Read file → LLM parser |
| Audio file (any format ffmpeg supports) | v1 | Audio transcription pipeline (§3.3) |
| Guitar Pro 5 (.gp5) | v2 | `guitar-pro` crate or custom parser |
| Guitar Pro X (.gpx, .gp) | v2 | `guitar-pro` crate |
| MusicXML (.xml, .musicxml) | v2 | XML parsing, map to TabDocument |
| MIDI (.mid) | v2 | `midly` crate, straightforward mapping |

### 6.2 Export Formats

| Format | Support | Method |
|--------|---------|--------|
| RiffLab Tab (.rltab) | v1 | JSON serialization of TabDocument |
| ASCII text (.txt) | v1 | Render TabDocument to ASCII string |
| MIDI (.mid) | v2 | Convert TabNote[] to MIDI events |

---

## 7. Dependencies

### 7.1 Rust Crates

| Crate | Purpose | Required for |
|-------|---------|-------------|
| `ort` | ONNX Runtime bindings | Basic Pitch inference |
| `aubio-rs` or `aubio-sys` | Onset detection, beat tracking, pitch (YIN) | Audio transcription fallback + beat grid |
| `rustfft` | FFT for spectrogram computation | Basic Pitch preprocessing |
| `serde`, `serde_json` | Serialization | .rltab format |
| `uuid` | Note IDs | Data model |
| `chrono` | Timestamps | Provenance |
| `reqwest` | HTTP client for Anthropic API | LLM-based ASCII parsing |
| `hound` | WAV file I/O | Audio pipeline |
| `rubato` | Audio resampling | Preprocessing for Basic Pitch |

### 7.2 External Models

| Model | Format | Size (approx) | Distribution |
|-------|--------|----------------|-------------|
| Basic Pitch | ONNX | ~20 MB | **Downloaded on first use**, cached locally at `~/.rifflab/models/basic_pitch.onnx` |
| Demucs (htdemucs) | PyTorch → ONNX or subprocess | ~80 MB | Already in RiffLab's dependency tree |

### 7.3 External Services

| Service | Purpose | Required? |
|---------|---------|-----------|
| Anthropic Claude API | ASCII tab parsing | Optional (rule-based fallback available) |

---

## 8. Implementation Phases

### Phase 1 — Core Pipeline & Basic UI

**Goal:** User can import an ASCII tab or provide audio, get a rendered scrolling tablature synced to playback.

**Tasks:**
1. Implement `TabNote`, `TabDocument` data model and .rltab serialization.
2. Implement LLM-based ASCII parser with rule-based fallback.
3. Integrate Basic Pitch ONNX for audio transcription (or YIN fallback).
4. Implement beat tracking via aubio.
5. Implement pitch-to-fret mapper (min-movement heuristic).
6. Implement DTW alignment and fusion engine.
7. Implement Scrolling Tablature View renderer.
8. Implement ASCII Text View renderer.
9. Wire up Cue Engine integration (playback sync).
10. Wire up Demucs integration (stem reuse).
11. Basic note editing (change fret, delete, insert).

**Exit criteria:** Can paste an ASCII tab, load a song, see a scrolling tablature synced to playback, and correct transcription errors by editing notes.

### Phase 2 — Enhanced Accuracy & Views

**Tasks:**
1. Guitar Pro / MusicXML / MIDI import.
2. Highway View renderer.
3. Spectral string classifier (trained on user data).
4. Technique detection from audio (spectral classifier).
5. Accuracy analysis integration (color-coded hit/miss).
6. Practice session tracking per note.
7. MIDI export.

**Exit criteria:** Audio-only mode produces accurate tabs with correct string assignments. Highway view works for gamified practice. Accuracy feedback is shown per-note.

---

## 9. Testing Strategy

### 9.1 Unit Tests

- ASCII parser: 20+ test cases covering common tab formats, edge cases (unusual labels, partial tabs, tabs with lyrics interspersed).
- Pitch-to-fret mapper: test all chromatic pitches in bass range, verify correct string/fret assignment with min-movement heuristic.
- DTW alignment: synthetic test cases with known alignment, including gaps and insertions.
- TabDocument serialization round-trip.

### 9.2 Integration Tests

- End-to-end: ASCII text → LLM parse → TabDocument → ASCII re-render. Verify note count and sequence preservation.
- End-to-end: WAV file → Demucs → Basic Pitch → TabDocument. Verify against manually annotated ground truth for 5 test tracks.
- Fusion: ASCII + audio → fused TabDocument. Verify timing comes from audio, fingering from ASCII.

### 9.3 Test Data

Maintain a `tests/fixtures/tabs/` directory with:
- 10 ASCII tab files covering different formatting conventions.
- 5 short audio clips (CC-licensed bass recordings) with manually annotated ground truth TabDocuments.
- 3 Guitar Pro files (for Phase 2).

### 9.4 Accuracy Metrics

For audio transcription, track:
- **Note-level F1:** precision and recall of detected notes vs. ground truth (onset within ±50ms, pitch within ±1 semitone).
- **String accuracy:** percentage of correctly assigned strings (when ground truth fingering is available).
- **Target:** Note F1 ≥ 0.85 on isolated bass stems. String accuracy ≥ 0.80 with min-movement heuristic, ≥ 0.90 with spectral classifier.

---

## 10. Open Questions

| ID | Question | Impact | Resolution needed by |
|----|----------|--------|---------------------|
| ~~OQ-1~~ | **RESOLVED:** Basic Pitch ONNX model is downloaded on first use (~20 MB). Application ships without the model; on first audio transcription request, the model is fetched and cached locally. Avoids license bundling concerns and reduces initial package size. | — | — |
| ~~OQ-2~~ | **RESOLVED:** Best-effort rule-based fallback is sufficient for Phase 1. The fallback parser handles common tab formats; unusual formatting may produce partial or inaccurate results when offline. Users are informed when LLM parsing is unavailable and results may be lower quality. | — | — |
| ~~OQ-3~~ | **RESOLVED:** Inline editing — notes are always editable directly in the tablature view while working with tracks. No separate edit mode toggle. | — | — |
| ~~OQ-4~~ | **RESOLVED:** 4-string bass only. Tuning is a fixed `[u8; 4]` array (alternative tunings supported by changing values, e.g. drop D). UI, renderers, and parsers are built exclusively for 4-string bass. 5-string and 6-string support is out of scope. | — | — |
| ~~OQ-5~~ | **RESOLVED:** Guided training. RiffLab provides structured exercises (e.g., chromatic scale per string, open strings, common fret positions). Training audio + labels are saved persistently at `~/.rifflab/training/string_classifier/` and reused across model retrains — user only needs to record exercises once. Additional exercises can be added later to improve accuracy. | — | — |
| ~~OQ-6~~ | **RESOLVED:** Download-on-first-use (per revised OQ-1) eliminates bundling license concerns. Standard Apache 2.0 notice in THIRD_PARTY_LICENSES file is sufficient for runtime download. | — | — |
