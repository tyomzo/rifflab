---
id: "2.7.2"
title: "Sinusoidal decomposition"
status: pending
crate: "rifflab-analysis"
requirement: "FR-M4-10"
---

Implement sinusoidal decomposition of audio: STFT analysis, peak picking in the magnitude spectrum, partial tracking using McAulay-Quatieri algorithm, harmonic grouping of partials into fundamental frequencies, and note segmentation from harmonic groups. Output `Vec<NoteEvent>` with detailed partial information (frequency, amplitude, phase per partial per frame).
