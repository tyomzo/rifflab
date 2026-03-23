---
id: "2.6.4"
title: "Edit Script action for Rhai effects"
status: pending
crate: "rifflab-ui"
requirement: "FR-M7-15"
---

Add an "Edit Script" button on scripted effect nodes in the Effects Editor. Clicking the button opens the `.rhai` source file in the system default editor via `xdg-open` (Linux). The button is only shown for effects originating from user scripts, not for built-in effects.
