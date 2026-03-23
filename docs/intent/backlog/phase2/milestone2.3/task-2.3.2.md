---
id: "2.3.2"
title: "Pre-compile scripts to AST"
status: pending
crate: "rifflab-fx"
requirement: "FR-M2-11"
---

Compile Rhai scripts at load time rather than interpreting from source on each call. Store the compiled AST alongside the ScriptedEffect. The audio thread only calls pre-compiled functions, avoiding parse overhead during real-time processing. Recompile when the script file changes (detected via file watcher or manual reload).
