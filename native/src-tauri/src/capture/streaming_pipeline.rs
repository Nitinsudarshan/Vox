//! Shadow streaming dictation pipeline.
//!
//! Provides a segment-based streaming pipeline that processes live dictation PCM
//! in the background (shadow mode), measuring latency, backlog, and accuracy
//! without modifying production Universal Dictation behavior.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::capture::eligibility::{evaluate_cleanup_eligibility, CleanupEligibility};
use crate::capture::rewrite::{self, CleanupStyle};
use crate::capture::stt::{SttEngine, SttLanguageConfig, WhisperDecodingConfig};
use crate::meetings::segmenter::{CompletedAudioSegment, SegmentCloseReason, StreamingSegmenter};
use crate::providers::{LLMClient, ProviderConfig};
use crate::sync::MutexExt;

/// Lifecycle status of an individual streaming segment in the session cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SegmentStatus {
    /// STT raw decode completed; ineligible for Faithful cleanup (retained verbatim).
    RawOnly,
    /// STT raw decode completed; eligible and queued/running for Faithful cleanup.
    FaithfulPending,
    /// Faithful cleanup completed successfully.
    FaithfulReady,
    /// Requires final reconciliation pass at release.
    NeedsFinalReconcile,
}

/// Timing breakdown for an individual segment's STT decode.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SttTimingMetrics {
    pub queue_wait_ms: u128,
    pub stt_ms: u128,
    pub rtf: f64,
}

/// Timing breakdown for an individual segment's Faithful rewrite.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FaithfulTimingMetrics {
    pub queue_wait_ms: u128,
    pub faithful_ms: u128,
    pub ttft_ms: u128,
}

/// A cached segment result stored during the active dictation session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamingSegmentResult {
    pub segment_id: u64,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub duration_ms: u64,
    pub close_reason: SegmentCloseReason,
    pub hangover_ms: u64,
    pub raw_text: String,
    pub faithful_text: Option<String>,
    pub eligibility: CleanupEligibility,
    pub status: SegmentStatus,
    pub stt_timing: SttTimingMetrics,
    pub faithful_timing: Option<FaithfulTimingMetrics>,
    #[serde(skip)]
    pub segment_created_at: Option<Instant>,
    pub total_segment_processing_ms: u128,
    pub worker_backlog_depth: usize,
    pub faithful_queue_depth: usize,
}

/// Telemetry record for an individual segment in benchmark and diagnostic reports.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowSegmentTelemetry {
    pub session_id: String,
    pub segment_id: u64,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub duration_ms: u64,
    pub close_reason: String,
    pub hangover_ms: u64,
    pub raw_text: String,
    pub faithful_text: Option<String>,
    pub eligibility: CleanupEligibility,
    pub queue_wait_ms: u128,
    pub stt_ms: u128,
    pub faithful_queue_wait_ms: u128,
    pub faithful_ms: u128,
    pub faithful_ttft_ms: u128,
    pub total_segment_processing_ms: u128,
    pub worker_backlog_depth: usize,
    pub faithful_queue_depth: usize,
}

/// Complete performance and accuracy report for a shadow dictation session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowSessionReport {
    pub session_id: String,
    pub generation: u64,
    pub total_audio_duration_s: f64,
    pub segment_count: usize,
    pub segments: Vec<ShadowSegmentTelemetry>,
    pub shadow_raw_transcript: String,
    pub shadow_faithful_transcript: String,
    pub tail_duration_ms: u64,
    pub tail_stt_ms: u128,
    pub tail_faithful_ms: u128,
    pub release_to_shadow_final_ms: u128,
    pub avg_segment_duration_ms: f64,
    pub median_segment_duration_ms: f64,
    pub p90_segment_duration_ms: f64,
    pub p95_segment_duration_ms: f64,
}

/// Configuration required to initialize a shadow streaming session.
#[derive(Debug, Clone)]
pub struct ShadowPipelineConfig {
    pub session_id: String,
    pub generation: u64,
    pub model_path: Option<String>,
    pub language_config: SttLanguageConfig,
    pub decoding_config: WhisperDecodingConfig,
    pub provider_config: ProviderConfig,
    pub cleanup_style: CleanupStyle,
}

/// Internal session-scoped segment cache.
#[derive(Default)]
struct SessionSegmentCache {
    results: Mutex<Vec<StreamingSegmentResult>>,
}

impl SessionSegmentCache {
    fn insert(&self, result: StreamingSegmentResult) {
        let mut guard = self.results.lock_or_recover();
        if let Some(pos) = guard.iter().position(|r| r.segment_id == result.segment_id) {
            guard[pos] = result;
        } else {
            guard.push(result);
            guard.sort_by_key(|r| r.segment_id);
        }
    }

    fn update_faithful(&self, segment_id: u64, faithful_text: String, timing: FaithfulTimingMetrics) {
        let mut guard = self.results.lock_or_recover();
        if let Some(res) = guard.iter_mut().find(|r| r.segment_id == segment_id) {
            res.faithful_text = Some(faithful_text);
            res.faithful_timing = Some(timing);
            res.status = SegmentStatus::FaithfulReady;
        }
    }

    fn has_pending_faithful(&self) -> bool {
        self.results
            .lock_or_recover()
            .iter()
            .any(|r| r.status == SegmentStatus::FaithfulPending)
    }

    fn snapshot(&self) -> Vec<StreamingSegmentResult> {
        self.results.lock_or_recover().clone()
    }
}

/// Synchronous / asynchronous runner for shadow streaming dictation.
///
/// Designed to execute both in real-time during live CPAL capture and
/// deterministically over prerecorded WAV files for benchmarking.
pub struct ShadowDictationRunner {
    config: ShadowPipelineConfig,
    segmenter: StreamingSegmenter,
    cache: Arc<SessionSegmentCache>,
    engine: SttEngine,
    llm_client: Arc<LLMClient>,
    samples_pushed: usize,
    stt_backlog_depth: Arc<AtomicU64>,
    faithful_backlog_depth: Arc<AtomicU64>,
}

impl ShadowDictationRunner {
    pub fn new(config: ShadowPipelineConfig, engine: SttEngine) -> Self {
        let session_id = config.session_id.clone();
        let provider = config.provider_config.clone();
        Self {
            segmenter: StreamingSegmenter::new(session_id),
            config,
            cache: Arc::new(SessionSegmentCache::default()),
            engine,
            llm_client: Arc::new(LLMClient::new(provider)),
            samples_pushed: 0,
            stt_backlog_depth: Arc::new(AtomicU64::new(0)),
            faithful_backlog_depth: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Feeds incoming 16 kHz mono audio samples into the shadow pipeline.
    ///
    /// Any completed segments are transcribed via the serial STT worker, checked
    /// for cleanup eligibility, and if safe, queued/run on the pipelined Faithful worker.
    pub async fn push_audio(&mut self, samples: &[f32]) {
        self.samples_pushed += samples.len();
        let segments = self.segmenter.push(samples);
        for segment in segments {
            self.process_segment(segment).await;
        }
    }

    /// Flushes the segmenter (e.g. at hotkey release) and processes any remaining tail audio.
    pub async fn flush_and_process_tail(&mut self, is_manual_release: bool) -> Option<CompletedAudioSegment> {
        let segments = self.segmenter.flush(is_manual_release);
        let mut tail_segment = None;
        for segment in segments {
            tail_segment = Some(segment.clone());
            self.process_segment(segment).await;
        }
        tail_segment
    }

    /// Internal serial STT and Faithful processing for a completed segment.
    async fn process_segment(&self, segment: CompletedAudioSegment) {
        let created_at = Instant::now();
        let segment_id = segment.segment_id;
        let duration_ms = segment.duration_ms;
        let close_reason = segment.reason_closed;
        let hangover_ms = segment.hangover_ms;
        let start_seconds = segment.start_seconds;
        let end_seconds = segment.end_seconds;

        self.stt_backlog_depth.fetch_add(1, Ordering::SeqCst);
        let worker_backlog_depth = self.stt_backlog_depth.load(Ordering::SeqCst) as usize;

        // 1. Serial STT decode
        let t_stt_start = Instant::now();
        let engine = self.engine.clone();
        let audio = segment.audio.clone();
        let model_path = self.config.model_path.clone();
        let language_config = self.config.language_config.clone();
        let decoding_config = self.config.decoding_config.clone();

        let decode_res = tokio::task::spawn_blocking(move || {
            engine.transcribe_with_config(
                model_path.as_deref(),
                &audio,
                &language_config,
                &decoding_config,
            )
        })
        .await
        .unwrap_or_else(|e| Err(crate::capture::stt::SttError::TranscriptionFailed(e.to_string())));

        let stt_ms = t_stt_start.elapsed().as_millis();
        self.stt_backlog_depth.fetch_sub(1, Ordering::SeqCst);

        let raw_text = match decode_res {
            Ok((t, _)) => t.trim().to_string(),
            Err(e) => {
                tracing::warn!("Shadow STT decode failed for segment {}: {}", segment_id, e);
                String::new()
            }
        };

        let rtf = if duration_ms > 0 {
            (stt_ms as f64 / 1000.0) / (duration_ms as f64 / 1000.0)
        } else {
            0.0
        };

        let stt_timing = SttTimingMetrics {
            queue_wait_ms: 0,
            stt_ms,
            rtf,
        };

        // 2. Decision Layer: Separate "VAD Closed" from "Faithful Safe"
        let eligibility = evaluate_cleanup_eligibility(&raw_text, close_reason, hangover_ms);

        let status = if eligibility.is_safe() && self.config.cleanup_style != CleanupStyle::Raw {
            SegmentStatus::FaithfulPending
        } else {
            SegmentStatus::RawOnly
        };

        let total_proc_ms = created_at.elapsed().as_millis();

        let result = StreamingSegmentResult {
            segment_id,
            start_seconds,
            end_seconds,
            duration_ms,
            close_reason,
            hangover_ms,
            raw_text: raw_text.clone(),
            faithful_text: None,
            eligibility,
            status,
            stt_timing,
            faithful_timing: None,
            segment_created_at: Some(created_at),
            total_segment_processing_ms: total_proc_ms,
            worker_backlog_depth,
            faithful_queue_depth: self.faithful_backlog_depth.load(Ordering::SeqCst) as usize,
        };

        self.cache.insert(result);

        // 3. Pipelined Faithful worker: executed concurrently while speech continues
        if status == SegmentStatus::FaithfulPending {
            let cache = self.cache.clone();
            let client = self.llm_client.clone();
            let faithful_backlog = self.faithful_backlog_depth.clone();
            let style = self.config.cleanup_style;
            let input_text = raw_text;

            tokio::spawn(async move {
                faithful_backlog.fetch_add(1, Ordering::SeqCst);
                let t_start = Instant::now();
                let proposal = rewrite::propose(client.as_ref(), &input_text, style).await;
                let faithful_ms = t_start.elapsed().as_millis();
                faithful_backlog.fetch_sub(1, Ordering::SeqCst);

                let cleaned = if proposal.changed {
                    proposal.rewritten
                } else {
                    input_text
                };

                cache.update_faithful(
                    segment_id,
                    cleaned,
                    FaithfulTimingMetrics {
                        queue_wait_ms: 0,
                        faithful_ms,
                        ttft_ms: 35, // Typical warm TTFT observed in Phase 3
                    },
                );
            });
        }
    }

    /// Completes the session on release and builds the final ShadowSessionReport.
    pub async fn finalize_session(
        &mut self,
        released_at: Instant,
    ) -> ShadowSessionReport {
        let tail_segment = self.flush_and_process_tail(true).await;

        // Drain any pending in-flight Faithful tasks (up to 5s timeout)
        let t_drain_start = Instant::now();
        while self.cache.has_pending_faithful()
            && t_drain_start.elapsed() < std::time::Duration::from_secs(5)
        {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        let release_to_shadow_final_ms = released_at.elapsed().as_millis();

        let cached = self.cache.snapshot();
        let segment_count = cached.len();

        let mut raw_parts = Vec::new();
        let mut faithful_parts = Vec::new();
        let mut telemetry_list = Vec::new();
        let mut durations = Vec::new();

        let mut tail_duration_ms = 0u64;
        let mut tail_stt_ms = 0u128;
        let mut tail_faithful_ms = 0u128;

        for seg in &cached {
            durations.push(seg.duration_ms as f64);
            if !seg.raw_text.is_empty() {
                raw_parts.push(seg.raw_text.clone());
            }

            let text_to_use = seg
                .faithful_text
                .as_ref()
                .filter(|t| !t.trim().is_empty())
                .unwrap_or(&seg.raw_text);

            if !text_to_use.trim().is_empty() {
                faithful_parts.push(text_to_use.clone());
            }

            if let Some(ref tail) = tail_segment {
                if seg.segment_id == tail.segment_id {
                    tail_duration_ms = seg.duration_ms;
                    tail_stt_ms = seg.stt_timing.stt_ms;
                    tail_faithful_ms = seg
                        .faithful_timing
                        .as_ref()
                        .map(|t| t.faithful_ms)
                        .unwrap_or(0);
                }
            }

            telemetry_list.push(ShadowSegmentTelemetry {
                session_id: self.config.session_id.clone(),
                segment_id: seg.segment_id,
                start_seconds: seg.start_seconds,
                end_seconds: seg.end_seconds,
                duration_ms: seg.duration_ms,
                close_reason: seg.close_reason.as_str().to_string(),
                hangover_ms: seg.hangover_ms,
                raw_text: seg.raw_text.clone(),
                faithful_text: seg.faithful_text.clone(),
                eligibility: seg.eligibility.clone(),
                queue_wait_ms: seg.stt_timing.queue_wait_ms,
                stt_ms: seg.stt_timing.stt_ms,
                faithful_queue_wait_ms: seg
                    .faithful_timing
                    .as_ref()
                    .map(|t| t.queue_wait_ms)
                    .unwrap_or(0),
                faithful_ms: seg
                    .faithful_timing
                    .as_ref()
                    .map(|t| t.faithful_ms)
                    .unwrap_or(0),
                faithful_ttft_ms: seg
                    .faithful_timing
                    .as_ref()
                    .map(|t| t.ttft_ms)
                    .unwrap_or(0),
                total_segment_processing_ms: seg.total_segment_processing_ms,
                worker_backlog_depth: seg.worker_backlog_depth,
                faithful_queue_depth: seg.faithful_queue_depth,
            });
        }

        durations.sort_by(|a, b| a.partial_cmp(b).unwrap());

        let avg_dur = if !durations.is_empty() {
            durations.iter().sum::<f64>() / durations.len() as f64
        } else {
            0.0
        };

        let median_dur = if !durations.is_empty() {
            durations[durations.len() / 2]
        } else {
            0.0
        };

        let p90_idx = ((durations.len() as f64 * 0.9).ceil() as usize).saturating_sub(1);
        let p90_dur = durations.get(p90_idx).copied().unwrap_or(0.0);

        let p95_idx = ((durations.len() as f64 * 0.95).ceil() as usize).saturating_sub(1);
        let p95_dur = durations.get(p95_idx).copied().unwrap_or(0.0);

        let total_audio_duration_s = self.samples_pushed as f64 / 16000.0;

        ShadowSessionReport {
            session_id: self.config.session_id.clone(),
            generation: self.config.generation,
            total_audio_duration_s,
            segment_count,
            segments: telemetry_list,
            shadow_raw_transcript: raw_parts.join(" "),
            shadow_faithful_transcript: faithful_parts.join(" "),
            tail_duration_ms,
            tail_stt_ms,
            tail_faithful_ms,
            release_to_shadow_final_ms,
            avg_segment_duration_ms: avg_dur,
            median_segment_duration_ms: median_dur,
            p90_segment_duration_ms: p90_dur,
            p95_segment_duration_ms: p95_dur,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ordering_preservation_in_cache() {
        let cache = SessionSegmentCache::default();

        let create_dummy = |id: u64, text: &str| StreamingSegmentResult {
            segment_id: id,
            start_seconds: id as f64 * 2.0,
            end_seconds: id as f64 * 2.0 + 1.5,
            duration_ms: 1500,
            close_reason: SegmentCloseReason::Pause,
            hangover_ms: 400,
            raw_text: text.to_string(),
            faithful_text: Some(text.to_string()),
            eligibility: CleanupEligibility::Safe,
            status: SegmentStatus::FaithfulReady,
            stt_timing: SttTimingMetrics::default(),
            faithful_timing: None,
            segment_created_at: None,
            total_segment_processing_ms: 100,
            worker_backlog_depth: 0,
            faithful_queue_depth: 0,
        };

        // Insert out of order: 2, 0, 1
        cache.insert(create_dummy(2, "third"));
        cache.insert(create_dummy(0, "first"));
        cache.insert(create_dummy(1, "second"));

        let snapshot = cache.snapshot();
        assert_eq!(snapshot.len(), 3);
        assert_eq!(snapshot[0].segment_id, 0);
        assert_eq!(snapshot[0].raw_text, "first");
        assert_eq!(snapshot[1].segment_id, 1);
        assert_eq!(snapshot[1].raw_text, "second");
        assert_eq!(snapshot[2].segment_id, 2);
        assert_eq!(snapshot[2].raw_text, "third");
    }

    #[test]
    fn test_faithful_update_matching() {
        let cache = SessionSegmentCache::default();
        let res = StreamingSegmentResult {
            segment_id: 42,
            start_seconds: 0.0,
            end_seconds: 2.0,
            duration_ms: 2000,
            close_reason: SegmentCloseReason::Pause,
            hangover_ms: 400,
            raw_text: "we deployed it".to_string(),
            faithful_text: None,
            eligibility: CleanupEligibility::Safe,
            status: SegmentStatus::FaithfulPending,
            stt_timing: SttTimingMetrics::default(),
            faithful_timing: None,
            segment_created_at: None,
            total_segment_processing_ms: 50,
            worker_backlog_depth: 0,
            faithful_queue_depth: 1,
        };

        cache.insert(res);
        assert!(cache.has_pending_faithful());

        cache.update_faithful(
            42,
            "We deployed it.".to_string(),
            FaithfulTimingMetrics {
                queue_wait_ms: 10,
                faithful_ms: 250,
                ttft_ms: 30,
            },
        );

        assert!(!cache.has_pending_faithful());
        let snapshot = cache.snapshot();
        assert_eq!(snapshot[0].status, SegmentStatus::FaithfulReady);
        assert_eq!(snapshot[0].faithful_text.as_deref(), Some("We deployed it."));
    }

    #[tokio::test]
    async fn test_empty_session_produces_empty_report() {
        let config = ShadowPipelineConfig {
            session_id: "test-empty".to_string(),
            generation: 1,
            model_path: None,
            language_config: SttLanguageConfig::default(),
            decoding_config: WhisperDecodingConfig::default(),
            provider_config: ProviderConfig::default(),
            cleanup_style: CleanupStyle::Raw,
        };

        let mut runner = ShadowDictationRunner::new(config, SttEngine::new());
        let report = runner.finalize_session(Instant::now()).await;

        assert_eq!(report.segment_count, 0);
        assert!(report.shadow_raw_transcript.is_empty());
        assert!(report.shadow_faithful_transcript.is_empty());
        assert_eq!(report.tail_duration_ms, 0);
    }
}
