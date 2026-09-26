# Decision Log — Push-to-Talk Pill Redesign & Interaction Refinement

## Context
Vox's push-to-talk (PTT) interface required an interaction and visual refinement inspired by Oscar's minimalist desktop pill experience while retaining 100% of Vox's existing Rust backend, local Whisper, global hotkey registration, and LLM transformation capabilities.

---

## Decisions

### Decision 1: Oscar-Style Edge-Attached Horizontal Notch
- **Choice**: Replace the legacy large circular logo dot with a slim, horizontal notch attached directly to the screen edge when idle/hidden (`64px x 10px`).
- **Why**: Eliminates screen clutter and prevents the pill from acting like an annoying floating button over user content.
- **Alternatives Considered**: Keeping a floating circular badge.
- **Reason Rejected**: Floating circular badge frequently swallowed mouse clicks meant for underlying application windows and looked out of place.

### Decision 2: Removal of Heavy "Vox" Branding Text
- **Choice**: Remove "Vox" brand text from the collapsed pill label. Optionally display the active foreground application name (e.g. `● Chrome` or `● Snipping Tool`) or a simple state dot `● Click to dictate`.
- **Why**: Desktop utilities should be quiet and functional, prioritizing utility over branding clutter.

### Decision 3: Floating Hotkey Hint Bar (`Hold to record [Ctrl] [Space]`)
- **Choice**: Add an off-white floating hint pill directly above the main pill upon activation/hover displaying `Hold to record [Ctrl] [Space]`.
- **Why**: Matches Oscar's visual reference while providing clear keyboard affordance without forcing text into the main pill body.

### Decision 4: Smooth Hover State Transitions with Intent & Grace Delays
- **Choice**: 120ms enter intent delay, 1000ms hover-out grace delay, smooth width/opacity/shadow transitions.
- **Why**: Prevents sudden UI jumping and allows easy navigation between pill controls and the settings popover.

### Decision 5: Real Settings in Dropdown Surface
- **Choice**: Include Auto-paste (Toggle), Text transform (Toggle), Cleanup style (Faithful/Clean/Professional/Concise), Prompt mode (Toggle with "Rewrite speech into a prompt"), and Speech Language.
- **Why**: All dropdown controls map directly to Vox's capabilities without fake or simulated state.

### Decision 6: The Popover as Built — Cleanup Style Replaced Text Transform, and There Is No Prompt Mode
- **Choice**: `PillSettingsPopover.tsx` carries six rows: **Auto-paste after dictation** (toggle, `clipboard.auto_paste`); **Dictation sounds** (toggle, `sound.dictation_sounds`); **Cleanup style**, a sub-page offering Raw (Default), Faithful, Clean, Polished and Concise (`stt.cleanup_style`); **Language**, a sub-page offering Auto-detect, English (US), Hinglish, Hindi and Español (written as `language.primary_dictation_language` plus `language.spoken_languages`); **Open Vox**; and **Open All Settings in App**. Decision 5's Text transform and Prompt mode toggles are not in it.
- **Why**: Decision 5 records what was planned; this records what shipped. Cleanup style is the one control for the Tier 2 rewrite: Raw means no model runs, and it is the default (`docs/decisions.md` Decision 71). The Text transform toggle, a second on/off switch for the same rewrite, left the popover in `07ea67e`, and its `stt.text_transform` setting was removed later. No version of the popover in this repository's history has carried a Prompt mode control, and no code implements one.
- **Alternatives Considered**: Keeping a Text transform toggle beside the style list.
- **Reason Rejected**: Two controls for one behaviour can disagree — the toggle off with Polished chosen, or on with Raw — and Raw already means off.

---

## Trade-Offs & Mitigation
- **Native Window Sizing**: Dynamic window resizing in Tauri requires sending IPC calls (`set_pill_window_mode`) when transitioning between NOTCH, COLLAPSED, EXPANDED, and POPOVER modes.
- **Mitigation**: Rust overlay subsystem uses transparent, hit-region-optimized window bounding boxes to prevent dead-zone click interception.
