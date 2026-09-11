//! Generating a meeting report in the background, cancellably, without ever
//! destroying the one that was already there.
//!
//! Summarising a long meeting on a local model takes minutes. That shapes
//! everything here: the work is a detached task, the UI reads its state from
//! disk, and every failure mode has to leave the user with *something*.
//!
//! ## Backup and restore
//!
//! Regeneration copies the existing report aside before it starts and puts it
//! back if the run fails or is cancelled. This is the one piece of meetily's
//! summary service worth copying verbatim, and it is the difference between
//! "the regeneration failed" and "the regeneration failed and your report is
//! gone".
//!
//! ## The English cache
//!
//! Reports are generated in English and translated afterwards
//! (see [`super::processor`]). The English original is stored alongside the
//! translated one under a fingerprint of everything that could change it — the
//! transcript, the template's content, the user's instructions, the provider
//! and model. Asking for the same meeting in a second language then costs one
//! translation pass instead of a full re-summarisation.
//!
//! ## Retries
//!
//! Meetily issues exactly one HTTP request per summary with no retry and no
//! backoff, and reports a 300-second timeout as "timed out after 60 seconds".
//! A local model that is still loading, or a cloud provider returning 429, is
//! an ordinary event over a meeting's lifetime, so each pass here is attempted
//! up to [`MAX_ATTEMPTS`] times with a widening delay.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter};

use crate::providers::{CompletionOptions, LLMClient, ProviderConfig};
use crate::sync::MutexExt;

use super::super::model::{MeetingSummary, SummaryStatus};
use super::super::store::{MeetingStore, MeetingStoreError};
use super::super::transcription::render_transcript;
use super::processor::{self, LanguageAction, MeetingContext};
use super::templates::{Template, TemplateLibrary, DEFAULT_TEMPLATE_ID};

/// Progress and completion, for the meeting detail surface.
pub const SUMMARY_PROGRESS_EVENT: &str = "meeting-summary-progress";

/// Attempts per model call before a pass is called failed.
const MAX_ATTEMPTS: u32 = 3;

/// Delay before the first retry; doubled for each subsequent one.
const RETRY_BASE_DELAY: Duration = Duration::from_secs(2);

/// Output ceiling for a report. Large enough for a long meeting's worth of
/// sections; meetily pins Claude to 2048 and truncates long reports mid-document.
const REPORT_MAX_OUTPUT_TOKENS: u32 = 4_096;

/// Output ceiling for one chunk's notes.
const CHUNK_MAX_OUTPUT_TOKENS: u32 = 1_600;

/// Temperature for every pass. A meeting report is extraction, not writing:
/// the failure mode of a warm model here is an invented owner for an action
/// item.
const SUMMARY_TEMPERATURE: f32 = 0.2;

#[derive(Debug, thiserror::Error)]
pub enum SummaryError {
    #[error("meeting storage: {0}")]
    Store(#[from] MeetingStoreError),

    #[error("this meeting has no transcript to summarise")]
    EmptyTranscript,

    #[error("a summary for this meeting is already being generated")]
    AlreadyRunning,

    #[error("summary generation was cancelled")]
    Cancelled,

    #[error("the language model did not answer: {0}")]
    Provider(String),

    /// The configured provider cannot answer, and the message says what to do.
    ///
    /// Separate from [`Self::Provider`] because they call for different things
    /// from the user. A provider that is not installed or not configured is
    /// fixed in Settings; a request that failed is retried.
    #[error("{0}")]
    ProviderUnavailable(String),
}

/// What the caller asked for.
#[derive(Debug, Clone)]
pub struct SummaryOptions {
    pub template_id: String,
    /// BCP-47 code for the report's language. `None` means English.
    pub language: Option<String>,
    /// A free-text steer from the user.
    pub user_instructions: Option<String>,
    /// Ignore the English cache and regenerate from the transcript.
    pub force: bool,
}

impl Default for SummaryOptions {
    fn default() -> Self {
        Self {
            template_id: DEFAULT_TEMPLATE_ID.to_string(),
            language: None,
            user_instructions: None,
            force: false,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SummaryProgress {
    pub meeting_id: String,
    pub status: SummaryStatus,
    /// A short phrase for the UI: "Reading part 2 of 5", "Translating".
    pub stage: String,
    /// `0.0..=1.0`, or `None` while the total is unknown.
    pub fraction: Option<f32>,
}

/// Runs and tracks summary generation.
///
/// One instance lives in app state. The cancellation registry is what lets a
/// second call to [`SummaryService::cancel`] reach a task that was started by
/// a different command invocation.
pub struct SummaryService {
    store: Arc<MeetingStore>,
    running: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl SummaryService {
    pub fn new(store: Arc<MeetingStore>) -> Self {
        Self {
            store,
            running: Mutex::new(HashMap::new()),
        }
    }

    /// Whether a run is in flight for this meeting.
    pub fn is_running(&self, meeting_id: &str) -> bool {
        self.running.lock_or_recover().contains_key(meeting_id)
    }

    /// Asks the in-flight run for this meeting to stop.
    ///
    /// Returns whether there was one. The task notices between passes and on
    /// its next await point, restores the previous report, and marks the
    /// record `Cancelled`.
    pub fn cancel(&self, meeting_id: &str) -> bool {
        match self.running.lock_or_recover().get(meeting_id) {
            Some(flag) => {
                flag.store(true, Ordering::SeqCst);
                true
            }
            None => false,
        }
    }

    /// Starts generating a report, returning as soon as the task is spawned.
    ///
    /// Everything after this point is visible through the stored
    /// [`MeetingSummary`] and the [`SUMMARY_PROGRESS_EVENT`] stream.
    pub fn start(
        self: &Arc<Self>,
        app: Option<AppHandle>,
        meeting_id: &str,
        options: SummaryOptions,
        provider: ProviderConfig,
        templates: TemplateLibrary,
    ) -> Result<(), SummaryError> {
        if self.is_running(meeting_id) {
            return Err(SummaryError::AlreadyRunning);
        }

        let segments = self.store.load_transcript(meeting_id)?;
        if segments.iter().all(|segment| segment.text.trim().is_empty()) {
            return Err(SummaryError::EmptyTranscript);
        }

        let cancel = Arc::new(AtomicBool::new(false));
        self.running
            .lock_or_recover()
            .insert(meeting_id.to_string(), cancel.clone());

        let service = Arc::clone(self);
        let meeting_id = meeting_id.to_string();
        tauri::async_runtime::spawn(async move {
            let outcome = service
                .run(app.clone(), &meeting_id, options, provider, templates, &cancel)
                .await;
            service.running.lock_or_recover().remove(&meeting_id);
            if let Err(err) = outcome {
                tracing::warn!("meeting {}: summary failed: {}", meeting_id, err);
            }
        });
        Ok(())
    }

    async fn run(
        &self,
        app: Option<AppHandle>,
        meeting_id: &str,
        options: SummaryOptions,
        provider: ProviderConfig,
        templates: TemplateLibrary,
        cancel: &Arc<AtomicBool>,
    ) -> Result<(), SummaryError> {
        let started = Instant::now();
        let meeting = self.store.load_meeting(meeting_id)?;
        let segments = self.store.load_transcript(meeting_id)?;
        let transcript = render_transcript(&segments);
        let template = templates.get_or_default(Some(&options.template_id));

        // Back up before touching anything: from here on, every exit path
        // either completes or restores.
        let existing = self.store.load_summary(meeting_id)?;
        let previous_markdown = existing.as_ref().and_then(|s| s.markdown.clone());

        let context = MeetingContext {
            title: meeting.title.clone(),
            recorded_at: meeting.created_at.clone(),
            duration_seconds: meeting.duration_seconds,
            system_audio_captured: meeting.system_audio_captured,
            user_instructions: options.user_instructions.clone(),
        };
        // Before any work is queued against it. Without this, a machine with no
        // Ollama installed spent three attempts and a minute of backoff on
        // `http://localhost:11434` and then stored the transport error as the
        // meeting's report status — every word true, and no help at all.
        //
        // Unconditional, including where the English report is already cached:
        // the language pass still needs a provider, and the check costs one
        // local ping or, for a cloud provider, nothing at all.
        if let Err(reason) = crate::providers::check_ready(&provider).await {
            return Err(SummaryError::ProviderUnavailable(reason.to_string()));
        }

        let client = LLMClient::new(provider.clone());
        let fingerprint = fingerprint(&transcript, &template, &options, &provider, &client);

        let mut record = MeetingSummary::pending(meeting_id, &template.id);
        record.status = SummaryStatus::Processing;
        record.previous_markdown = previous_markdown.clone();
        record.provider = Some(format!("{:?}", client.provider_type()));
        record.model = Some(client.model_name());
        record.language = options.language.clone();
        record.fingerprint = Some(fingerprint.clone());
        // Carry the cached English original forward so a cache hit can use it.
        record.english_markdown = existing.as_ref().and_then(|s| s.english_markdown.clone());
        self.store.save_summary(&record)?;
        emit(&app, meeting_id, SummaryStatus::Processing, "Starting", Some(0.0));

        let cached_english = existing
            .as_ref()
            .filter(|_| !options.force)
            .filter(|s| s.fingerprint.as_deref() == Some(fingerprint.as_str()))
            .and_then(|s| s.english_markdown.clone())
            .filter(|text| !text.trim().is_empty());

        let result = self
            .generate(
                &app,
                meeting_id,
                &client,
                &template,
                &context,
                &transcript,
                cached_english,
                &options,
                cancel,
                &mut record,
            )
            .await;

        match result {
            Ok(markdown) => {
                record.status = SummaryStatus::Completed;
                record.markdown = Some(markdown.clone());
                record.previous_markdown = None;
                record.error = None;
                record.completed_at = Some(chrono::Utc::now().to_rfc3339());
                record.processing_ms = started.elapsed().as_millis() as u64;
                self.store.save_summary(&record)?;

                // Rename the meeting from the report's own title. Only when
                // the meeting still carries its generated name — a title the
                // user typed is theirs, and silently replacing it is what
                // meetily does (`summary/service.rs:527`).
                if let Some(title) = processor::extract_title(&markdown) {
                    if is_generated_title(&meeting.title) {
                        let _ = self.store.update_meeting(meeting_id, |record| {
                            record.title = title;
                        });
                    }
                }

                emit(&app, meeting_id, SummaryStatus::Completed, "Done", Some(1.0));
                Ok(())
            }
            Err(err) => {
                let cancelled = matches!(err, SummaryError::Cancelled);
                record.status = if cancelled {
                    SummaryStatus::Cancelled
                } else {
                    SummaryStatus::Failed
                };
                // Put back what was there. A failed regeneration must not be
                // how someone loses last week's report.
                record.markdown = previous_markdown;
                record.previous_markdown = None;
                record.error = Some(err.to_string());
                record.completed_at = Some(chrono::Utc::now().to_rfc3339());
                record.processing_ms = started.elapsed().as_millis() as u64;
                self.store.save_summary(&record)?;
                emit(
                    &app,
                    meeting_id,
                    record.status,
                    if cancelled { "Cancelled" } else { "Failed" },
                    None,
                );
                Err(err)
            }
        }
    }

    /// The passes themselves: English report, then language.
    #[allow(clippy::too_many_arguments)]
    async fn generate(
        &self,
        app: &Option<AppHandle>,
        meeting_id: &str,
        client: &LLMClient,
        template: &Template,
        context: &MeetingContext,
        transcript: &str,
        cached_english: Option<String>,
        options: &SummaryOptions,
        cancel: &Arc<AtomicBool>,
        record: &mut MeetingSummary,
    ) -> Result<String, SummaryError> {
        let english = match cached_english {
            Some(cached) => {
                tracing::info!(
                    "meeting {}: reusing the cached English report; only the language pass will run",
                    meeting_id
                );
                emit(
                    app,
                    meeting_id,
                    SummaryStatus::Processing,
                    "Reusing the existing report",
                    Some(0.7),
                );
                cached
            }
            None => {
                let budget = processor::transcript_budget_chars(client.context_tokens());
                let chunks =
                    processor::chunk_transcript(transcript, budget, processor::CHUNK_OVERLAP_CHARS);
                record.chunk_count = chunks.len() as u32;

                if chunks.len() <= 1 {
                    check_cancelled(cancel)?;
                    emit(
                        app,
                        meeting_id,
                        SummaryStatus::Processing,
                        "Writing the report",
                        Some(0.3),
                    );
                    let prompt = processor::build_report_prompt(template, context, transcript);
                    self.call(client, &prompt, REPORT_MAX_OUTPUT_TOKENS, cancel).await?
                } else {
                    // Long meeting: notes per chunk, then one assembling pass.
                    let mut notes = Vec::with_capacity(chunks.len());
                    for (index, chunk) in chunks.iter().enumerate() {
                        check_cancelled(cancel)?;
                        emit(
                            app,
                            meeting_id,
                            SummaryStatus::Processing,
                            &format!("Reading part {} of {}", index + 1, chunks.len()),
                            Some(0.1 + 0.6 * (index as f32 / chunks.len() as f32)),
                        );
                        let prompt =
                            processor::build_chunk_prompt(context, chunk, index, chunks.len());
                        match self.call(client, &prompt, CHUNK_MAX_OUTPUT_TOKENS, cancel).await {
                            Ok(text) => notes.push(text),
                            Err(SummaryError::Cancelled) => return Err(SummaryError::Cancelled),
                            Err(err) => {
                                // One unreadable stretch should not lose the
                                // whole meeting — but it is recorded, because
                                // the report is then missing that stretch.
                                tracing::warn!(
                                    "meeting {}: part {} of {} could not be summarised: {}",
                                    meeting_id,
                                    index + 1,
                                    chunks.len(),
                                    err
                                );
                            }
                        }
                    }
                    if notes.is_empty() {
                        return Err(SummaryError::Provider(
                            "no part of the transcript could be summarised".into(),
                        ));
                    }
                    check_cancelled(cancel)?;
                    emit(
                        app,
                        meeting_id,
                        SummaryStatus::Processing,
                        "Assembling the report",
                        Some(0.75),
                    );
                    let prompt = processor::build_combine_prompt(template, context, &notes);
                    self.call(client, &prompt, REPORT_MAX_OUTPUT_TOKENS, cancel).await?
                }
            }
        };

        let english = processor::clean_markdown(&english);
        if english.trim().is_empty() {
            return Err(SummaryError::Provider("the model returned nothing".into()));
        }
        record.english_markdown = Some(english.clone());

        match processor::resolve_language_action(options.language.as_deref()) {
            LanguageAction::KeepEnglish => Ok(english),
            LanguageAction::Translate { name, .. } => {
                check_cancelled(cancel)?;
                emit(
                    app,
                    meeting_id,
                    SummaryStatus::Processing,
                    &format!("Translating into {name}"),
                    Some(0.85),
                );
                let prompt = processor::build_translation_prompt(&english, &name);
                match self.call(client, &prompt, REPORT_MAX_OUTPUT_TOKENS, cancel).await {
                    Ok(translated) => {
                        let cleaned = processor::clean_markdown(&translated);
                        if cleaned.trim().is_empty() {
                            // A failed translation is not a failed meeting.
                            // The English report is a complete, useful answer.
                            tracing::warn!(
                                "meeting {}: translation returned nothing; keeping the English report",
                                meeting_id
                            );
                            Ok(english)
                        } else {
                            Ok(cleaned)
                        }
                    }
                    Err(SummaryError::Cancelled) => Err(SummaryError::Cancelled),
                    Err(err) => {
                        tracing::warn!(
                            "meeting {}: translation failed ({}); keeping the English report",
                            meeting_id,
                            err
                        );
                        Ok(english)
                    }
                }
            }
        }
    }

    /// One model call, retried on transient failure.
    async fn call(
        &self,
        client: &LLMClient,
        prompt: &processor::Prompt,
        max_output_tokens: u32,
        cancel: &Arc<AtomicBool>,
    ) -> Result<String, SummaryError> {
        let options = CompletionOptions {
            temperature: SUMMARY_TEMPERATURE,
            max_output_tokens,
            context_tokens: client.context_tokens(),
            ..CompletionOptions::default()
        };

        let mut last_error = String::from("no attempt was made");
        for attempt in 0..MAX_ATTEMPTS {
            check_cancelled(cancel)?;
            if attempt > 0 {
                let delay = RETRY_BASE_DELAY * 2u32.pow(attempt - 1);
                tracing::info!(
                    "retrying the summary request in {}s (attempt {} of {})",
                    delay.as_secs(),
                    attempt + 1,
                    MAX_ATTEMPTS
                );
                tokio::time::sleep(delay).await;
                check_cancelled(cancel)?;
            }
            // `complete_verified` rather than `complete`: the client
            // substitutes canned heuristic text when a provider is
            // unreachable, and canned text stored as a meeting report is a
            // fabricated record.
            match client
                .complete_verified(&prompt.user, Some(&prompt.system), options)
                .await
            {
                Ok(response) => return Ok(response.text),
                Err(err) => {
                    last_error = err.to_string();
                    tracing::warn!("summary request failed: {}", last_error);
                }
            }
        }
        Err(SummaryError::Provider(last_error))
    }
}

fn check_cancelled(cancel: &Arc<AtomicBool>) -> Result<(), SummaryError> {
    if cancel.load(Ordering::SeqCst) {
        Err(SummaryError::Cancelled)
    } else {
        Ok(())
    }
}

fn emit(
    app: &Option<AppHandle>,
    meeting_id: &str,
    status: SummaryStatus,
    stage: &str,
    fraction: Option<f32>,
) {
    let Some(app) = app else { return };
    let _ = app.emit(
        SUMMARY_PROGRESS_EVENT,
        SummaryProgress {
            meeting_id: meeting_id.to_string(),
            status,
            stage: stage.to_string(),
            fraction,
        },
    );
}

/// Everything that could change the English report, as one hash.
///
/// The transcript is hashed rather than included, so the fingerprint is a
/// fixed size regardless of meeting length.
fn fingerprint(
    transcript: &str,
    template: &Template,
    options: &SummaryOptions,
    provider: &ProviderConfig,
    client: &LLMClient,
) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(transcript.as_bytes());
    hasher.update(template.content_hash().as_bytes());
    hasher.update(
        options
            .user_instructions
            .as_deref()
            .unwrap_or("")
            .trim()
            .as_bytes(),
    );
    hasher.update(client.model_name().as_bytes());
    hasher.update(format!("{:?}", provider.active_provider).as_bytes());
    // The window decides whether the transcript was chunked, which changes
    // the report even when nothing else did.
    hasher.update(provider.context_tokens.to_le_bytes());
    format!("{:x}", hasher.finalize())
}

/// Whether a meeting still carries the name Vox gave it.
///
/// `Meeting 2026-09-11 14:31` is a placeholder and replacing it with the
/// report's title is an improvement; anything else was typed by the user and
/// is not Vox's to overwrite.
pub fn is_generated_title(title: &str) -> bool {
    let trimmed = title.trim();
    if !trimmed.starts_with("Meeting ") {
        return false;
    }
    let rest = trimmed.trim_start_matches("Meeting ").trim();
    !rest.is_empty()
        && rest
            .chars()
            .all(|c| c.is_ascii_digit() || c == '-' || c == ':' || c == ' ' || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::ProviderType;

    fn provider() -> ProviderConfig {
        ProviderConfig {
            active_provider: ProviderType::Ollama,
            ollama_model: "llama3.2:latest".into(),
            ..ProviderConfig::default()
        }
    }

    fn template() -> Template {
        TemplateLibrary::bundled_only().get("general").unwrap()
    }

    #[test]
    fn the_same_inputs_fingerprint_the_same_way() {
        let options = SummaryOptions::default();
        let config = provider();
        let client = LLMClient::new(config.clone());
        let a = fingerprint("transcript", &template(), &options, &config, &client);
        let b = fingerprint("transcript", &template(), &options, &config, &client);
        assert_eq!(a, b);
    }

    #[test]
    fn a_changed_transcript_invalidates_the_cache() {
        let options = SummaryOptions::default();
        let config = provider();
        let client = LLMClient::new(config.clone());
        assert_ne!(
            fingerprint("before", &template(), &options, &config, &client),
            fingerprint("after", &template(), &options, &config, &client)
        );
    }

    #[test]
    fn an_edited_template_invalidates_the_cache() {
        let options = SummaryOptions::default();
        let config = provider();
        let client = LLMClient::new(config.clone());
        let mut edited = template();
        edited.sections[0].instruction.push_str(" Be terse.");
        assert_ne!(
            fingerprint("t", &template(), &options, &config, &client),
            fingerprint("t", &edited, &options, &config, &client)
        );
    }

    #[test]
    fn changing_the_model_invalidates_the_cache() {
        let options = SummaryOptions::default();
        let a = provider();
        let mut b = provider();
        b.ollama_model = "qwen2.5:7b".into();
        assert_ne!(
            fingerprint("t", &template(), &options, &a, &LLMClient::new(a.clone())),
            fingerprint("t", &template(), &options, &b, &LLMClient::new(b.clone()))
        );
    }

    #[test]
    fn changing_the_context_window_invalidates_the_cache() {
        let options = SummaryOptions::default();
        let a = provider();
        let mut b = provider();
        b.context_tokens = 32_768;
        assert_ne!(
            fingerprint("t", &template(), &options, &a, &LLMClient::new(a.clone())),
            fingerprint("t", &template(), &options, &b, &LLMClient::new(b.clone()))
        );
    }

    #[test]
    fn changing_the_output_language_alone_does_not_invalidate_the_english_cache() {
        // This is the whole point of the cache: a second language costs one
        // translation pass, not another summarisation.
        let config = provider();
        let client = LLMClient::new(config.clone());
        let english = SummaryOptions::default();
        let spanish = SummaryOptions {
            language: Some("es".into()),
            ..SummaryOptions::default()
        };
        assert_eq!(
            fingerprint("t", &template(), &english, &config, &client),
            fingerprint("t", &template(), &spanish, &config, &client)
        );
    }

    #[test]
    fn a_different_user_steer_invalidates_the_cache() {
        let config = provider();
        let client = LLMClient::new(config.clone());
        let plain = SummaryOptions::default();
        let steered = SummaryOptions {
            user_instructions: Some("Focus on budget".into()),
            ..SummaryOptions::default()
        };
        assert_ne!(
            fingerprint("t", &template(), &plain, &config, &client),
            fingerprint("t", &template(), &steered, &config, &client)
        );
        // Whitespace around the steer is not a different steer.
        let padded = SummaryOptions {
            user_instructions: Some("  Focus on budget  ".into()),
            ..SummaryOptions::default()
        };
        assert_eq!(
            fingerprint("t", &template(), &steered, &config, &client),
            fingerprint("t", &template(), &padded, &config, &client)
        );
    }

    #[test]
    fn a_generated_title_may_be_replaced_and_a_typed_one_may_not() {
        assert!(is_generated_title("Meeting 2026-09-11 14:31"));
        assert!(is_generated_title("Meeting 11_09_26_14_31_07"));
        assert!(!is_generated_title("Q3 pricing review"));
        assert!(!is_generated_title("Meeting with Priya"));
        assert!(!is_generated_title("Meeting "));
        assert!(!is_generated_title(""));
    }

    #[test]
    fn cancelling_an_unknown_meeting_reports_that_there_was_nothing_to_cancel() {
        let store = Arc::new(MeetingStore::new(std::env::temp_dir().join("vox-summary-none")));
        let service = SummaryService::new(store);
        assert!(!service.cancel("meeting-nope"));
        assert!(!service.is_running("meeting-nope"));
    }

    #[test]
    fn a_meeting_with_no_transcript_is_refused_before_any_model_is_asked() {
        let dir = std::env::temp_dir().join(format!(
            "vox-summary-empty-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Arc::new(MeetingStore::new(&dir));
        let meeting = crate::meetings::Meeting::new(
            "meeting-empty".into(),
            "Nothing".into(),
            crate::meetings::MeetingSource::Recorded,
        );
        store.create(&meeting).unwrap();

        let service = Arc::new(SummaryService::new(Arc::clone(&store)));
        let result = service.start(
            None,
            "meeting-empty",
            SummaryOptions::default(),
            provider(),
            TemplateLibrary::bundled_only(),
        );

        assert!(matches!(result, Err(SummaryError::EmptyTranscript)));
        assert!(!service.is_running("meeting-empty"));
    }

    #[test]
    fn a_cancelled_flag_stops_the_next_pass() {
        let cancel = Arc::new(AtomicBool::new(false));
        assert!(check_cancelled(&cancel).is_ok());
        cancel.store(true, Ordering::SeqCst);
        assert!(matches!(check_cancelled(&cancel), Err(SummaryError::Cancelled)));
    }
}
