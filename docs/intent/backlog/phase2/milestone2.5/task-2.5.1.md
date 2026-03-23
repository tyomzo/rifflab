---
id: "2.5.1"
title: "Update demucs worker for 6-stem"
status: pending
crate: "workers/"
requirement: "FR-M3-02"
---

Update the demucs worker to accept a model parameter in the Separate request. Support the `htdemucs_6s` model which produces 6 stems (vocals, bass, drums, guitar, piano, other) in addition to the existing 4-stem model. Route the model selection to the appropriate demucs invocation and handle the additional output stems.
