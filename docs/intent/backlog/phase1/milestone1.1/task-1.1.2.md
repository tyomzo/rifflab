---
id: "1.1.2"
title: "Integration test: backend selection"
status: pending
crate: "rifflab-audio"
requirement: "FR-M1-02"
---

Write an integration test verifying that `create_backend()` selects JACK when available and falls back to ALSA otherwise. Test both paths: mock or conditionally detect JACK availability, then assert the returned backend type. Acceptance: test passes on systems with and without JACK.
