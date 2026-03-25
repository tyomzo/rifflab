# Tool "Forty Six & 2" — Bass Tone Presets

**Artist:** Justin Chancellor (Tool)
**Album:** Aenima (1996)
**Bass:** Wal Mk II (active EQ/filter, pick playing)
**Reference rig:** Boss BF-2 Flanger, ProCo Rat, GK 2001RB / Mesa Bass 400+

---

## Tone 1: Flanged Verse (signature swirling riff)

**Signal chain:** Input → Gate → Compressor → EQ → Flanger → Light Drive → Cabinet → Reverb → Output

| Effect | Parameter | Value |
|--------|-----------|-------|
| **Noise Gate** | Threshold | -36 dB |
| | Attack | 1 ms |
| | Release | 80 ms |
| **Compressor** | Threshold | -45 dB |
| | Ratio | 3.5 |
| | Attack | 35 ms (slow — lets pick transient punch through) |
| | Release | 60 ms |
| | Makeup | +3 dB |
| **Parametric EQ** | Band 1 (Low Shelf 100 Hz) | +2 dB |
| | Band 2 (Mid-Low 800 Hz) | +5 dB, Q=1.5 |
| | Band 3 (Mid-High 2.5 kHz) | +3 dB, Q=2.0 (pick presence) |
| | Band 4 (High Shelf 8 kHz) | -2 dB |
| **Flanger** | Rate | 0.12 Hz (very slow sweep) |
| | Depth | 4.0 ms |
| | Feedback | 0.65 (high resonance, metallic BF-2) |
| | Mix | 0.5 |
| **Light Overdrive** | Drive | 0.15 (harmonic sustain, not grit) |
| | Tone | 0.5 |
| | Mix | 0.3 |
| **Cabinet** | Type | Guitar 4x12 (dark, full body) |
| | Tone | 0.35 (dark) |
| | Resonance | 0.4 |
| **Reverb** | Room Size | 0.3 (small room / cab ambience) |
| | Damping | 0.5 |
| | Wet | 0.15 (subtle — space, not wash) |
| | Dry | 0.85 |

---

## Tone 2: Driven Heavy Sections (no flanger, aggressive)

**Signal chain:** Input → Compressor → EQ → Overdrive → Cabinet → Output

| Effect | Parameter | Value |
|--------|-----------|-------|
| **Compressor** | Threshold | -18 dB |
| | Ratio | 4.0 |
| | Attack | 10 ms |
| | Release | 60 ms |
| **Parametric EQ** | Band 2 (Mid-Low 800 Hz) | +6 dB |
| | Band 3 (Mid-High 1.5 kHz) | +4 dB |
| **Overdrive** | Drive | 0.45 (moderate grit, not fuzz) |
| | Tone | 0.55 (slightly bright for pick attack) |
| | Mix | 0.7 |
| | Shaper | Tanh |
| **Cabinet** | Type | Guitar 4x12 |
| | Tone | 0.4 |
| | Resonance | 0.35 |

---

## Tone 2 (Multiband Variant): Clean lows + driven mids

**Signal chain:** Input → Crossover Split → per-band processing → Merge → Cabinet → Output

| Band | Range | Mode | Amount | Gain |
|------|-------|------|--------|------|
| **Low** | below 250 Hz | Compressed | 0.6 | +2 dB |
| **Mid** | 250–2500 Hz | Drive | 0.4 | +1 dB |
| **High** | above 2500 Hz | Clean | — | -2 dB |

Preserves sub-bass punch while mids get Rat-style grit (Darkglass/Chancellor approach).

---

## Notes

- **Mid boost at 800 Hz–1.2 kHz is critical** — simulates the Wal bass onboard filter. Without it the tone sounds generic.
- **Flanger, not chorus** — the feedback/resonance gives the metallic, vocal sweep. Chorus is too smooth.
- **Pick playing** matters — Chancellor's sharp transients drive how the flanger and distortion respond.
- The Boss BF-2 flanger has a specific metallic character. High feedback (0.5–0.7) with slow rate (0.1–0.2 Hz) approximates it.
- The ProCo Rat is mid-focused distortion — not scooped. The overdrive in RiffLab with tanh shaper + mid-heavy EQ before it gets close.
