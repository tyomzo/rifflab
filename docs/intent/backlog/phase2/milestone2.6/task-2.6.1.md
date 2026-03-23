---
id: "2.6.1"
title: "Effects Editor view with auto-generated controls"
status: pending
crate: "rifflab-ui"
requirement: "FR-M7-13"
---

Implement the Effects Editor view with a left-to-right signal flow visualization showing the effect chain. Clicking an effect node expands its parameter controls, auto-generated from `EffectDescriptor.param_descriptors()`. Map parameter types to widgets: Float to slider, Bool to toggle, Enum to dropdown. Display the effect name, bypass toggle, and remove button for each node.
