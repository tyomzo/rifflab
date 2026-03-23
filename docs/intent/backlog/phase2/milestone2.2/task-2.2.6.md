---
id: "2.2.6"
title: "Delay effect"
status: pending
crate: "rifflab-fx"
requirement: "FR-M2-03"
---

Implement a delay effect supporting both tempo-synced and free-running modes. Parameters: time_ms (delay time in milliseconds for free mode, or beat division for sync mode), feedback (0.0-1.0), mix (dry/wet blend 0.0-1.0), sync (toggle between free and tempo-synced). In sync mode, compute delay time from BPM and the selected beat division. Implement EffectDescriptor with param_descriptors().
