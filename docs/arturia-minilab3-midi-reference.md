# Arturia MiniLab 3 — MIDI Implementation Reference

## Hardware Overview

- **Keys:** 25 slim velocity-sensitive keys, default range C3–C5 (MIDI 48–72), ±4 octave shift
- **Pads:** 8 velocity/pressure-sensitive RGB pads, 2 banks (A/B) = 16 pads
- **Knobs:** 8 endless rotary encoders
- **Faders:** 4 sliders
- **Touch Strips:** 2 capacitive (pitch bend left, modulation right)
- **Main Encoder:** 1 clickable rotary with OLED display
- **Connection:** USB-C, class-compliant (no drivers on Linux/macOS), 5-pin DIN MIDI out

## USB MIDI Ports

1. **MiniLab 3 MIDI** — main port (notes, CCs, pitch bend)
2. **MiniLab 3 DIN THRU** — routes host MIDI to DIN connector
3. **MiniLab 3 MCU** — Mackie Control transport (separate from notes/CCs)
4. **MiniLab 3 ALV** — Analog Lab display feedback

## Default MIDI Channels

- Keyboard, knobs, faders, touch strips: **Channel 1**
- Pads: **Channel 10** (GM drum channel)

## Knob CC Map (Default Arturia Preset)

| Knob | CC# | Arturia Function |
|------|-----|-----------------|
| 1    | 74  | Brightness / Filter Cutoff |
| 2    | 71  | Timbre / Filter Resonance |
| 3    | 76  | Time |
| 4    | 77  | Movement |
| 5    | 93  | FX A Dry/Wet |
| 6    | 18  | FX B Dry/Wet |
| 7    | 19  | Delay Volume |
| 8    | 16  | Reverb Volume |

**Used in RiffLab:** `MINILAB3_KNOB_CCS: [u8; 8] = [74, 71, 76, 77, 93, 18, 19, 16]`

## Fader CC Map

| Fader | CC# | Function |
|-------|-----|----------|
| 1     | 82  | Master Bass |
| 2     | 83  | Master Midrange |
| 3     | 85  | Master Treble |
| 4     | 17  | Master Volume |

## Touch Strips

| Strip | Message | Notes |
|-------|---------|-------|
| Left (Pitch) | MIDI Pitch Bend | Spring-loaded, snaps to center |
| Right (Mod) | CC 1 | Holds position |

## Main Encoder

| Action | CC# |
|--------|-----|
| Turn | 114 |
| Shift + Turn | 112 |
| Click | 15 |
| Shift + Click | 113 |

## Pad Notes (Channel 10)

**Bank A:** Pads 1–8 → MIDI notes 36–43 (C1–G1, GM kick/snare/hats)
**Bank B:** Pads 1–8 → MIDI notes 44–51 (G#1–D#2, GM toms/cymbals)

Pads can be configured to send Note, CC, Program Change, or Mackie commands.

## Shift Button

- Press: CC 9, value 127
- Release: CC 9, value 0

## Transport (Shift + Pads, via MCU port)

| Combo | Function |
|-------|----------|
| Shift + Pad 4 | Loop On/Off |
| Shift + Pad 5 | Stop |
| Shift + Pad 6 | Play/Pause |
| Shift + Pad 7 | Record |
| Shift + Pad 8 | Tap Tempo |

## Pedal Input

1/4" TRS jack, configurable as sustain (CC 64), expression (any CC), or footswitch.

## Customization

All CCs fully remappable via **Arturia MIDI Control Center** (free software). 5 user preset slots stored on device. Template files: `.minilab3` extension.

## Complete CC Summary

```
Knob 1: CC 74    Fader 1: CC 82    Mod Strip: CC 1
Knob 2: CC 71    Fader 2: CC 83    Main Turn: CC 114
Knob 3: CC 76    Fader 3: CC 85    Main Shift: CC 112
Knob 4: CC 77    Fader 4: CC 17    Main Click: CC 15
Knob 5: CC 93    Shift Btn: CC 9   Main S+Click: CC 113
Knob 6: CC 18    Sustain: CC 64
Knob 7: CC 19
Knob 8: CC 16
```
