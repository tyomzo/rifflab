---
id: "2.3.3"
title: "Rhai sandbox with execution limits"
status: pending
crate: "rifflab-fx"
requirement: "FR-M2-12"
---

Configure the `rhai::Engine` with `max_operations` and `max_call_stack_depth` to prevent runaway scripts from blocking the audio thread. On limit overrun: bypass the effect (pass audio through unprocessed), log a warning with the script name and violation type. The audio thread must never stall due to a misbehaving script -- no xruns allowed.
