---
id: "2.3.4"
title: "Scan library/effects/*.rhai on startup"
status: pending
crate: "rifflab-fx"
requirement: "FR-M2-13"
---

On application initialization, scan the `library/effects/` directory for `.rhai` files. For each file found, compile the script to AST and register it in the EffectRegistry as a ScriptedEffectDef. Log errors for scripts that fail to compile but continue scanning the remaining files. Provide a count of successfully loaded scripts at startup.
