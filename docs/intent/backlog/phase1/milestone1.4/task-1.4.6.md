---
id: "1.4.6"
title: "Wire EffectChain into AudioGraph"
status: pending
crate: "rifflab-audio"
requirement: "FR-M1-04"
---

In the rifflab-app wiring code, create an `EffectChain` populated with default effects (e.g., NoiseGate and Compressor), wrap it as a `Box<dyn AudioProcessor>`, and assign it to `AudioGraph.fx_chain`. Ensure the instrument input signal is routed through the effect chain before reaching the output mix bus. Acceptance: effects are audible on live instrument input when running the application.
