---
id: "2.2.2"
title: "crossfade_to_preset()"
status: pending
crate: "rifflab-fx"
requirement: "FR-M2-06"
---

Blend the full effect chain state from the current configuration to a target preset over a configurable duration. During the transition, run both the old and new chains in parallel and crossfade their outputs. Handle structural changes (effect add/remove) during the transition by fading out removed effects and fading in added effects. The crossfade duration should be configurable (default ~50ms).
