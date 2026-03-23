---
id: "3.1.2"
title: "Map LV2 ports to ParamDescriptor"
status: pending
crate: "rifflab-fx"
requirement: "FR-M2-07"
---

Read LV2 port metadata (name, range, default value, type, scale points) and convert each control input port to a ParamDescriptor. This enables the auto-generated UI system to create appropriate controls for LV2 plugins without plugin-specific code. Map port types: float ports to Float params, toggled ports to Bool params, enumerated ports to Enum params.
