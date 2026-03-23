---
id: "2.3.5"
title: "EffectRegistry serves built-in + scripted"
status: pending
crate: "rifflab-fx"
requirement: "FR-M2-09"
---

Update the EffectRegistry to hold both built-in effect factories and ScriptedEffectDef entries from Rhai scripts. The `list_effects()` method returns all available effects grouped by source (Built-in vs User Script). Effect creation uses the registry to instantiate either a built-in effect or a ScriptedEffect depending on the selected entry.
