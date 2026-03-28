# Tab Notation System — Implementation Tasks

Source spec: `docs/intent/RIFFLAB-TAB-SPEC.md`

## Task Dependency Graph

```
01-data-model
  ├── 02-ascii-parser-fallback
  │     └── 03-ascii-parser-llm
  ├── 04-pitch-to-fret
  │     └── 05-audio-transcription
  ├── 06-fusion-engine (needs 04)
  ├── 07-tab-renderer-scrolling
  │     ├── 08-ascii-text-view
  │     └── 09-tab-editing
  └── 10-app-integration (needs all above)
```

## Execution Order

Implement in this sequence (respects dependencies, maximizes parallelism):

| Phase | Tasks | Description |
|-------|-------|-------------|
| 1 | 01 | Data model crate |
| 2 | 02, 04, 07 | Parser, fret mapper, renderer (parallel) |
| 3 | 03, 05, 08 | LLM parser, audio transcription, ASCII view (parallel) |
| 4 | 06, 09 | Fusion engine, editing |
| 5 | 10 | App integration |

## Task Summary

| # | Title | Spec Section | Status |
|---|-------|-------------|--------|
| 01 | Data Model Crate | 2.1–2.3 | TODO |
| 02 | Rule-Based ASCII Parser | 3.2.2 | TODO |
| 03 | LLM ASCII Parser | 3.2.1 | TODO |
| 04 | Pitch-to-Fret Mapper | 3.3.5 | TODO |
| 05 | Audio Transcription Pipeline | 3.3 | TODO |
| 06 | Alignment & Fusion (DTW) | 3.4 | TODO |
| 07 | Scrolling Tablature Renderer | 4.2.1 | TODO |
| 08 | ASCII Text View | 4.2.2 | TODO |
| 09 | Tab Note Editing | 4.4–4.5 | TODO |
| 10 | App Integration | 5.1–5.2 | TODO |

## Deferred (Phase 2)

- Highway View (spec 4.2.3) — Rocksmith-style vertical note highway
- Basic Pitch ONNX model (spec 3.3.3) — replaces YIN fallback for better transcription
- String/Fret Spectral Classifier (spec 3.5) — ML-based string identification
- Accuracy integration (spec 5.2) — color notes by play accuracy
