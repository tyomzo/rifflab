---
id: "3.1.1"
title: "LV2 plugin host integration"
status: pending
crate: "rifflab-fx"
requirement: "FR-M2-07"
---

Scan system LV2 plugins using the lilv library (via lilv or lv2 crate). Enumerate available plugins, instantiate them with the correct sample rate and buffer size, and wrap each as an EffectDescriptor. Handle plugin activation/deactivation lifecycle. Connect audio input/output ports and control ports for parameter manipulation.
