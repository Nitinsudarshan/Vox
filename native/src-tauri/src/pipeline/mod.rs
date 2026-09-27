use crate::providers::ProviderError;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod analysis;
mod enrichment;
pub mod source_boundary;
pub use enrichment::{
    enrich_content, enrich_content_from, enrich_scribble, enrich_vault_file,
    extract_deterministic_entities,
    extract_deterministic_knowledge, extract_deterministic_questions, extract_deterministic_title,
    extract_deterministic_topics, summarize_content, summarize_content_from,
    summarize_scribble, summarize_vault_file, bound_summary_content,
    AiEnrichmentResponse, CANONICAL_ANALYSIS_SYSTEM_PROMPT, CANONICAL_SUMMARY_PROMPT_INSTRUCTIONS,
    CANONICAL_SUMMARY_SYSTEM_PROMPT,
};
pub(crate) use enrichment::find_ignoring_ascii_case;

#[derive(Error, Debug)]
pub enum PipelineError {
    #[error("LLM Provider error: {0}")]
    ProviderError(#[from] ProviderError),

    #[error("Failed to parse JSON response: {0}")]
    JsonParseError(#[from] serde_json::Error),

    #[error("Vault operation failed: {0}")]
    VaultError(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessedPipelineResult {
    pub mode: String,
    pub transcript: String,
    pub note_id: Option<String>,
    pub kanban_cards_created: usize,
    pub output_markdown: String,
    /// Vault note titles used as grounding context.
    ///
    /// Retained for the frontend's `ProcessedPipelineResult` contract;
    /// grounded answers now come from Talkback, which carries full
    /// provenance (`talkback::ContextItem`) rather than bare titles.
    #[serde(default)]
    pub sources: Vec<String>,
    /// Base64 WAV of the answer spoken aloud, if a local TTS engine is configured.
    #[serde(default)]
    pub spoken_audio_base64: Option<String>,
}

