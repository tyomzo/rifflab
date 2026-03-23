---
id: "2.7.1"
title: "Playback rate adjustment with pitch correction"
status: pending
crate: "rifflab-audio"
requirement: "FR-M1-10"
---

Implement time-stretching for stem playback without pitch shift. Use a phase vocoder or WSOLA (Waveform Similarity Overlap-Add) algorithm to adjust playback rate while maintaining original pitch. Support tempo override from the cue system. The implementation must operate in real-time with acceptable latency for interactive practice.
