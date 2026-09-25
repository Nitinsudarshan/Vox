//! The dictation pipeline, from a stopped recording to finished text.
//!
//! Two surfaces record dictation: the global hotkey (`hotkeys`), which types
//! the result into whatever field had focus, and the pill's click-to-record
//! (`commands::stop_capture`), which does not. They used to carry two copies
//! of everything between the recorder and the vault — engine choice, prompt,
//! normalisation, snippets, romanisation, cleanup — and the copies had already
//! drifted: the hotkey path never checked for an empty transcript after
//! normalisation, so a recording Whisper heard only filler in was typed and
//! saved as an empty note. Both now call the three steps here, in order:
//!
//! 1. [`transcribe`] — decode with the engine the user chose, and record the
//!    diagnostics snapshot.
//! 2. [`finish_text`] — the deterministic Tier 1 pass. No model runs.
//! 3. [`clean_up`] — the opt-in Tier 2 rewrite (`capture::rewrite`), bounded by
//!    [`CLEANUP_TIMEOUT`] and never fatal.
//!
//! What happens to the text afterwards — injection, a todo, a scribble — stays
//! with the caller, because that is the part the two surfaces genuinely differ
//! in.

use std::time::{Duration, Instant};

use tauri::AppHandle;

use crate::capture::rewrite::CleanupStyle;
use crate::capture::CapturedAudio;
use crate::commands::{emit_capture_status_event, AppState};
use crate::settings::AppSettings;
use crate::sync::MutexExt;

/// How long a cleanup may hold up text the user is waiting on.
///
/// Past this the raw text is used. A cold local model routinely takes longer
/// than this to load, which is one reason cleanup is opt-in: the user who turns
/// it on has chosen that wait.
pub const CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);

/// The model label recorded in diagnostics when Parakeet decoded.
const PARAKEET_MODEL_LABEL: &str = "parakeet-tdt-0.6b-v3";

/// One decoded utterance.
#[derive(Debug, Clone)]
pub struct Transcription {
    /// Exactly what the engine returned, before any normalisation.
    pub text: String,
    /// Wall time of the decode itself, excluding model resolution.
    pub decode: Duration,
    /// When the decode started, for the latency trace.
    pub started_at: Instant,
}

/// Decodes a recording with the dictation engine the user chose.
///
/// Parakeet when it is selected *and* installed, Whisper otherwise, primed with
/// the dictionary and (when enabled) the custom initial prompt. The decode runs
/// on the blocking pool. A diagnostics snapshot is recorded whether or not the
/// decode succeeded, so a failure is inspectable afterwards.
///
/// If the Whisper model the settings resolve to is not on disk, it is
/// downloaded first — the first dictation on a fresh install pays for that —
/// and the pill is told so rather than sitting on "Transcribing" in silence.
pub async fn transcribe(
    app: &AppHandle,
    state: &AppState,
    captured: &CapturedAudio,
) -> Result<Transcription, String> {
    let settings = state.settings.lock_or_recover().clone();
    let models_dir = state.config_dir.join("models");

    let parakeet_dir = models_dir.join("parakeet");
    let use_parakeet = settings.stt.dictation_engine.as_deref() == Some("parakeet")
        && crate::capture::parakeet::ModelFiles::is_installed_in(&parakeet_dir);

    let model_path = if use_parakeet {
        None
    } else {
        match crate::capture::stt::installed_dictation_model_path(&models_dir, &settings.stt) {
            Some(path) => Some(path),
            None => {
                emit_capture_status_event(
                    app,
                    false,
                    Some(captured.mode.clone()),
                    "TRANSCRIBING",
                    Some("Downloading speech model…".to_string()),
                );
                crate::capture::stt::resolve_dictation_model_path(&models_dir, &settings.stt).await
            }
        }
    };

    let language_config = crate::capture::SttLanguageConfig::from_settings(
        &settings.language,
        crate::capture::stt::SttWindow::ShortForm,
    );
    let mut decoding_config = crate::capture::stt::WhisperDecodingConfig::for_dictation(&settings.stt);
    if let Some(prompt) = settings.build_stt_prompt() {
        decoding_config.initial_prompt = Some(prompt);
    }

    let started_at = Instant::now();
    let stt = state.stt.clone();
    let samples = captured.samples.clone();
    let decode = {
        let model_path = model_path.clone();
        let language_config = language_config.clone();
        let decoding_config = decoding_config.clone();
        tauri::async_runtime::spawn_blocking(move || {
            if use_parakeet {
                stt.transcribe_parakeet(&parakeet_dir, &samples)
            } else {
                stt.transcribe_with_config(
                    model_path.as_deref(),
                    &samples,
                    &language_config,
                    &decoding_config,
                )
            }
            .map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| format!("transcription task failed: {e}"))
        .and_then(|result| result)
    };
    let decode_time = started_at.elapsed();

    let (text, diag, error) = match decode {
        Ok((text, diag)) => (text, Some(diag), None),
        Err(message) => (String::new(), None, Some(message)),
    };

    let model_label = if use_parakeet {
        PARAKEET_MODEL_LABEL
    } else {
        model_path
            .as_deref()
            .unwrap_or(crate::capture::stt::DEFAULT_MODEL_FILENAME)
    };
    let snapshot = crate::capture::build_diagnostic_snapshot(
        &captured.mode,
        Some(captured.audio_path.clone()),
        captured,
        &settings.language,
        &language_config,
        &decoding_config,
        model_label,
        &text,
        diag.as_ref(),
        error.clone(),
    );
    crate::commands::record_stt_diagnostics(app, state, snapshot);

    match error {
        Some(message) => Err(message),
        None => Ok(Transcription {
            text,
            decode: decode_time,
            started_at,
        }),
    }
}

/// The deterministic pass: normalisation, snippet expansion, then the output
/// script.
///
/// Normalisation runs before snippets so a trigger is matched against tidied
/// text and a snippet's own replacement is never re-normalised. The `Dictated`
/// profile leaves sentence boundaries alone — this text is going into
/// whatever field has focus, and a period Vox appended is a period the user
/// has to delete.
///
/// `None` when nothing is left. Whisper often returns a lone filler or a
/// bracketed tag for a marginal recording, and normalisation removes exactly
/// that; an empty result must never be typed, saved or turned into a todo.
pub fn finish_text(settings: &AppSettings, raw: &str) -> Option<String> {
    use crate::capture::text_normalize::{normalize_text, TextProfile, Vocabulary};

    let normalized = normalize_text(
        raw,
        Vocabulary::new(&settings.dictionary, &settings.vocabulary_corrections),
        TextProfile::Dictated,
    )
    .text;
    if normalized.trim().is_empty() {
        return None;
    }

    let expanded = settings.expand_snippets(&normalized);
    let text = if expanded.trim().is_empty() { normalized } else { expanded };

    let script = crate::capture::romanize::OutputScript::from_setting(&settings.language.output_script);
    let projected = crate::capture::romanize::project(&text, script).into_owned();
    (!projected.trim().is_empty()).then_some(projected)
}

/// The result of the Tier 2 pass.
#[derive(Debug, Clone, PartialEq)]
pub struct Cleanup {
    /// What the user gets: the rewrite when one landed, the input otherwise.
    pub text: String,
    /// The text as it was before cleanup — `finish_text`'s output, not the raw
    /// engine output. This is the "before" the voice note's diff shows, so it
    /// has to be exactly what the rewrite was given.
    pub before: String,
    /// The style that changed the text, or `None` when nothing changed it.
    pub applied_style: Option<CleanupStyle>,
    /// How long the model was waited on. `None` when no model was asked.
    pub elapsed: Option<Duration>,
    /// Whether the wait ran out.
    pub timed_out: bool,
}

impl Cleanup {
    fn untouched(text: String) -> Self {
        Self {
            before: text.clone(),
            text,
            applied_style: None,
            elapsed: None,
            timed_out: false,
        }
    }

    /// The pre-cleanup text, when it differs from what the user got.
    pub fn before_if_changed(&self) -> Option<&str> {
        (self.before != self.text).then_some(self.before.as_str())
    }

    /// The applied style as the vault records it.
    pub fn applied_style_name(&self) -> Option<&'static str> {
        self.applied_style.map(|style| style.as_str())
    }
}

/// Runs the opt-in rewrite, if the user opted in.
///
/// `Raw` — the default — asks no model at all. Every other style gets at most
/// `timeout`; a timeout, an error or an unchanged answer all resolve to the
/// input, exactly as `rewrite::propose` promises. Nothing here can fail the
/// dictation.
pub async fn clean_up(
    completer: &dyn crate::providers::Completer,
    text: String,
    style: CleanupStyle,
    timeout: Duration,
) -> Cleanup {
    if style == CleanupStyle::Raw {
        return Cleanup::untouched(text);
    }

    let started = Instant::now();
    let outcome = tokio::time::timeout(timeout, crate::capture::rewrite::propose(completer, &text, style)).await;
    let elapsed = Some(started.elapsed());

    match outcome {
        Ok(proposal) if proposal.changed => Cleanup {
            text: proposal.rewritten,
            before: text,
            applied_style: Some(style),
            elapsed,
            timed_out: false,
        },
        Ok(_) => Cleanup {
            elapsed,
            ..Cleanup::untouched(text)
        },
        Err(_) => {
            tracing::warn!(
                "Dictation cleanup timed out after {:?}; using the text as transcribed",
                timeout
            );
            Cleanup {
                elapsed,
                timed_out: true,
                ..Cleanup::untouched(text)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::{
        BoxFuture, CompletionOptions, Completer, LLMResponse, ProviderError, ProviderType,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A completer that answers with a fixed reply after an optional delay,
    /// and counts how often it was asked.
    struct FixedCompleter {
        reply: Result<String, String>,
        delay: Duration,
        calls: AtomicUsize,
    }

    impl FixedCompleter {
        fn replying(reply: &str) -> Self {
            Self {
                reply: Ok(reply.to_string()),
                delay: Duration::ZERO,
                calls: AtomicUsize::new(0),
            }
        }

        fn failing() -> Self {
            Self {
                reply: Err("no provider".to_string()),
                delay: Duration::ZERO,
                calls: AtomicUsize::new(0),
            }
        }

        fn slow(reply: &str, delay: Duration) -> Self {
            Self {
                delay,
                ..Self::replying(reply)
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl Completer for FixedCompleter {
        fn default_options(&self) -> CompletionOptions {
            CompletionOptions::default()
        }

        fn provider_type(&self) -> &ProviderType {
            &ProviderType::Ollama
        }

        fn model_name(&self) -> String {
            "fixed".to_string()
        }

        fn complete_verified<'a>(
            &'a self,
            _prompt: &'a str,
            _system_prompt: Option<&'a str>,
            _options: CompletionOptions,
        ) -> BoxFuture<'a, Result<LLMResponse, ProviderError>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let reply = self.reply.clone();
            let delay = self.delay;
            Box::pin(async move {
                if !delay.is_zero() {
                    tokio::time::sleep(delay).await;
                }
                match reply {
                    Ok(text) => Ok(LLMResponse {
                        text,
                        model: "fixed".to_string(),
                        prompt_tokens: None,
                        completion_tokens: None,
                    }),
                    Err(message) => Err(ProviderError::NoCompletion(message)),
                }
            })
        }
    }

    #[tokio::test]
    async fn raw_style_never_asks_a_model() {
        let completer = FixedCompleter::replying("something else entirely");
        let result = clean_up(&completer, "send it today".into(), CleanupStyle::Raw, CLEANUP_TIMEOUT).await;

        assert_eq!(completer.calls(), 0);
        assert_eq!(result.text, "send it today");
        assert_eq!(result.before_if_changed(), None);
        assert_eq!(result.applied_style, None);
        assert_eq!(result.elapsed, None);
    }

    #[tokio::test]
    async fn a_changed_rewrite_is_used_and_records_its_style() {
        let completer = FixedCompleter::replying("Send it today.");
        let result = clean_up(&completer, "um send it today".into(), CleanupStyle::Faithful, CLEANUP_TIMEOUT).await;

        assert_eq!(completer.calls(), 1);
        assert_eq!(result.text, "Send it today.");
        assert_eq!(result.before_if_changed(), Some("um send it today"));
        assert_eq!(result.applied_style_name(), Some("faithful"));
        assert!(!result.timed_out);
    }

    #[tokio::test]
    async fn an_unchanged_rewrite_records_no_style() {
        let completer = FixedCompleter::replying("send it today");
        let result = clean_up(&completer, "send it today".into(), CleanupStyle::Clean, CLEANUP_TIMEOUT).await;

        assert_eq!(result.text, "send it today");
        assert_eq!(result.applied_style, None);
        assert!(result.elapsed.is_some(), "a model was asked, so the wait is reported");
    }

    #[tokio::test]
    async fn a_failed_provider_falls_back_to_the_input() {
        let completer = FixedCompleter::failing();
        let result = clean_up(&completer, "send it today".into(), CleanupStyle::Concise, CLEANUP_TIMEOUT).await;

        assert_eq!(result.text, "send it today");
        assert_eq!(result.applied_style, None);
        assert!(!result.timed_out);
    }

    #[tokio::test]
    async fn a_slow_provider_times_out_to_the_input() {
        let completer = FixedCompleter::slow("Send it today.", Duration::from_secs(5));
        let result = clean_up(
            &completer,
            "send it today".into(),
            CleanupStyle::Faithful,
            Duration::from_millis(20),
        )
        .await;

        assert_eq!(result.text, "send it today");
        assert_eq!(result.applied_style, None);
        assert!(result.timed_out);
    }

    #[test]
    fn finish_text_drops_a_transcript_that_normalises_to_nothing() {
        let settings = AppSettings::default();
        assert_eq!(finish_text(&settings, ""), None);
        assert_eq!(finish_text(&settings, "   "), None);
        assert_eq!(finish_text(&settings, "[BLANK_AUDIO]"), None);
    }

    #[test]
    fn finish_text_expands_snippets_after_normalising() {
        let settings = AppSettings {
            snippets: vec![crate::settings::SnippetItem {
                id: "sig".into(),
                trigger: "my signature".into(),
                snippet_text: "Best, Asha".into(),
                label: None,
                enabled: true,
            }],
            ..Default::default()
        };

        let finished = finish_text(&settings, "thanks my signature").expect("text survives");
        assert!(finished.contains("Best, Asha"), "{finished}");
    }

    #[test]
    fn finish_text_keeps_ordinary_speech() {
        let settings = AppSettings::default();
        let finished = finish_text(&settings, "send the report to Pragati").expect("text survives");
        assert!(finished.contains("send the report to Pragati"), "{finished}");
    }
}
