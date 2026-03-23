---
id: "1.8.6"
title: "Import progress UI"
status: pending
crate: "rifflab-ui"
requirement: "—"
---

Modal dialog during import showing: current step (Separating stems... / Tracking beats... / Transcribing notes...), progress bar (from worker Progress messages), cancel button. File: crates/rifflab-ui/src/views/import_dialog.rs (new). Acceptance: progress bar updates during separation, cancel stops the pipeline.
