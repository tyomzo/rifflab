---
id: "1.2.6"
title: "Verify CUDA and CPU fallback"
status: pending
crate: "rifflab-stems"
requirement: "FR-M3-03"
---

Test stem separation on both a CUDA-capable system and a CPU-only system. On GPU systems, verify that the worker utilizes the GPU (check torch.cuda usage or GPU memory allocation). On CPU-only systems, verify the separation completes successfully using the CPU fallback path. Acceptance: both CUDA and CPU paths produce valid, complete stem files.
