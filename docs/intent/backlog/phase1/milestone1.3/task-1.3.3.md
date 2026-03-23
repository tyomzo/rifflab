---
id: "1.3.3"
title: "Spectral flux onset detection"
status: pending
crate: "rifflab-analysis"
requirement: "FR-M4-06"
---

Implement onset detection using spectral flux. Compute the STFT of the input audio using rustfft with a configurable window size and hop length. Calculate spectral flux as the positive half-wave rectified difference between the magnitude spectra of consecutive frames. Peak-pick the flux signal above an adaptive threshold (median-based or moving average) to identify onset times. Return `Vec<f64>` of onset timestamps in seconds. File: `crates/rifflab-analysis/src/onset.rs`. Acceptance: detected onsets are within +/-20ms of known transients on reference test audio.
