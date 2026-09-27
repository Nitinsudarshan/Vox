# Vox — User Flows

## Flow 1: Universal Dictation (Type Anywhere)
1. User places their text cursor in any other application (email, Slack, an IDE, a browser form field).
2. User presses and holds `Ctrl+Space` (configurable), or presses it once with toggle-to-talk on.
3. The always-on-top, non-focus-stealing dictation pill shows it is listening; Vox's main window never needs to be visible. Audio capture starts immediately, and the window that had focus is remembered.
4. User speaks and releases the hotkey (or presses it again with toggle-to-talk).
5. The audio is transcribed locally — Whisper, or Parakeet TDT when selected and installed — then tidied by the deterministic pass: bracketed tags and decoder stutters removed, the user's dictionary and learned corrections applied, snippets expanded, the output script applied.
6. Only if the user chose a cleanup style (Faithful, Clean, Polished, Concise) does the configured model rewrite the text, for at most five seconds; the default, Raw, asks no model.
7. The text is put into the remembered field — pasted through the clipboard by default, or typed key by key with the keystrokes method. If focus moved to another window or tab, Vox waits up to 15 seconds for the user to come back; otherwise it leaves the text on the clipboard and says so in a notification.
8. The dictation is saved as a Voice Note, keeping the pre-cleanup text beside the cleaned text when a style changed it, so the change can be reviewed.

## Flow 2: Global Show/Hide Hotkey
1. User presses `Ctrl+Shift+Space` (configurable) while any other application is focused.
2. The OS-level `tauri-plugin-global-shortcut` hotkey fires regardless of which window has focus.
3. Vox's main window is shown and given focus if it was hidden, or hidden if it was already visible.

## Flow 3: Speaking a Todo
1. On the TODOs page, the user clicks the microphone, says the task, and clicks again to stop.
2. The recording goes through the same pipeline as dictation, without the cleanup rewrite.
3. The transcript is saved as a Voice Note and becomes one todo — a Kanban card in `<vault>/kanban/` whose title is the spoken line (truncated if long, with the full text kept as its description) and whose source points back at that Voice Note.
4. If nothing was heard, or the transcript came back empty, no todo is created and the page says which of those happened.

## Flow 4: From a Voice Note to the Knowledge Graph
1. The user opens a Voice Note — one dictated through the hotkey, or recorded with the pill's click-to-record — and chooses to add it to Scribbles.
2. `promote_voice_note_to_scribble` saves it as a Scribble in `<vault>/scribbles/`, and enrichment runs in the background: title, summary, topics and entities from the configured provider, with a deterministic fallback when no model is configured.
3. The Scribble appears in Scribbles and in the knowledge graph, linked to the topics and entities it shares with other notes.

## Flow 5: Recording a Meeting
1. The user starts a recording from the Meetings page, from a reminder's **Record**, or with **Join and record** on a calendar event.
2. Vox records the microphone and the system audio together; the meeting pill on the screen edge shows the elapsed time and a live waveform, with pause and stop.
3. The transcript fills in while the meeting runs, labelled You / Others by capture channel, and a checkpoint is written every 30 seconds so a crash costs seconds rather than the meeting.
4. On stop, the transcript is completed and a report is generated from the chosen template through the configured provider. See `docs/meetings.md`.

## Flow 6: Capturing a Web Page or AI Conversation
1. One-time setup: the user builds the extension (`npm --prefix native run build:extension`), loads `native/browser-extension` unpacked, turns **Browser capture** on in Vox's Capture settings, and pastes the port and pairing token into the extension's Options.
2. On any page, the user presses the extension's shortcut (`Ctrl+Shift+Y`) or clicks the Vox toolbar button. That gesture is what grants the extension access to this one tab — nothing before it did.
3. The extension injects its extractor into the tab, which reads the rendered document: a site-specific extractor for ChatGPT, Claude or GitHub; otherwise the generic article extractor; otherwise the page's visible text. The toolbar badge shows `…`.
4. The extractor returns a structured, text-only payload — blocks or conversation turns, plus `<head>` metadata and a coverage verdict saying how much of the page it could honestly claim. Nothing is sent if there was nothing readable; the badge shows `✕` with the reason.
5. The extension posts the payload to `http://127.0.0.1:<port>/v1/capture` with the pairing token in a header. Vox checks the origin, the token and the size before reading the body.
6. Vox derives provenance from the URL itself — application, domain, capture type — never from what the payload claimed. It then sanitizes every string, normalizes into markdown, and writes the artifact **and the raw payload** into `<vault>/captures/<id>/`. The badge shows `✓`, and Vox's Captures surface shows *Saved*.
7. Only now does analysis run, in the background: a summary, topics and entities, using the configured provider. If it fails, the surface says the capture is intact and offers *Analyse* again — the captured content is never at risk.
8. The user can open the capture to read it, see exactly where it came from and what was left out, read the raw stored payload, or add it to Scribbles — which is what carries it into search and the knowledge graph.
