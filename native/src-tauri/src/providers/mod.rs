mod ollama_manager;

pub mod ollama_install;

pub use ollama_manager::{
    ensure_ollama_ready, model_is_present, set_managed_binary, list_installed_models, test_ollama_prompt, OllamaModelDetails,
    OllamaPromptTestResult, OllamaStatus,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A boxed future, so [`Completer`] needs no `async_trait` dependency.
///
/// The same choice `meetings_v2::processing::llm` made and for the same
/// reason; the alias lives here now so there is one of it.
pub type BoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

/// What an analysis needs from a completion provider, and nothing else.
///
/// A trait rather than the concrete [`LLMClient`] so a caller can substitute a
/// scripted stand-in. That is the difference between a pipeline whose failure
/// paths — a timeout, unparseable JSON, an empty answer — are tested, and one
/// whose failure paths are hoped about.
///
/// It exists because of a specific migration. `meetings_v2::processing` drives
/// its own suites through exactly such a stand-in (`MeetingLlm` and its
/// `ScriptedLlm`), which is why it could not move onto
/// `pipeline::analysis::AnalysisService` — the service took a concrete client,
/// so migrating meant rewriting three test suites alongside the pipeline. This
/// is the seam that removes that coupling.
///
/// Deliberately narrow, and narrow in a specific way: every method here is
/// either *what the completion needs* or *what provenance needs*. What is
/// excluded is anything that would let the analysis layer start **deciding** —
/// streaming, hosts, model selection — none of which an analysis has any
/// business doing.
///
/// `model_name` is the provenance kind, not the deciding kind. It is the exact
/// analogue of [`provider_type`](Self::provider_type): a caller cannot choose
/// the model with it, only record which one was asked. Meetings need it because
/// "which model ran?" has to stay answerable for a meeting whose call *failed*,
/// where there is no response to read a model off.
pub trait Completer: Send + Sync {
    /// The provider's own sampling defaults, which a prompt then narrows.
    fn default_options(&self) -> CompletionOptions;

    /// Which service will answer, for the provenance record.
    fn provider_type(&self) -> &ProviderType;

    /// Which model will answer, for the provenance record.
    ///
    /// The configured model, so it is still answerable when nothing came back.
    /// A successful response reports the model that *actually* answered, which
    /// can differ; prefer that one when you have it.
    fn model_name(&self) -> String;

    /// One completion, with heuristic filler reported as the failure it is
    /// rather than returned as an answer.
    fn complete_verified<'a>(
        &'a self,
        prompt: &'a str,
        system_prompt: Option<&'a str>,
        options: CompletionOptions,
    ) -> BoxFuture<'a, Result<LLMResponse, ProviderError>>;
}

#[derive(Error, Debug)]
pub enum ProviderError {
    #[error("Network error connecting to LLM provider: {0}")]
    Network(#[from] reqwest::Error),

    #[error("Ollama service unavailable at {host}: {message}")]
    OllamaUnavailable { host: String, message: String },

    #[error("Cloud provider error ({code}): {message}")]
    CloudError { code: String, message: String },

    #[error("Invalid configuration: {0}")]
    ConfigError(String),

    /// The request completed, but nothing a model produced came back — the
    /// client substituted filler, or the provider answered with nothing.
    /// Separate from the transport errors above because the network was fine;
    /// what is missing is the completion.
    #[error("No completion available: {0}")]
    NoCompletion(String),

    /// The model was reached and answered with nothing.
    ///
    /// Split from [`NoCompletion`](Self::NoCompletion) because the two deserve
    /// different responses. A provider that could not be reached will not be
    /// reached by a second, differently-shaped request either, so retrying
    /// there only doubles the wait before the fallback. A model that answered
    /// with nothing *has* said something — that this prompt, as posed, got no
    /// engagement — and a shorter contract can genuinely fix it. Meetings'
    /// Stage B relies on exactly that distinction.
    #[error("The provider answered with an empty completion")]
    EmptyCompletion,
}

/// The `model` value [`LLMClient::complete`] reports when it has silently
/// substituted its own canned text for a real completion.
///
/// Public because it is a property of this layer that callers must be able to
/// detect. Filler is the right answer for dictation, where some output beats
/// none, and the wrong one for anything persisted as understanding.
pub const HEURISTIC_FALLBACK_MODEL: &str = "heuristic-fallback";

/// Why the configured provider cannot answer, in words that name the fix.
///
/// This exists because of what the failure looked like without it. A summary
/// on a machine with no Ollama installed spent three attempts and a minute of
/// backoff reaching `http://localhost:11434`, then stored
/// "error sending request for url (http://localhost:11434/api/generate)" as
/// the meeting's report status. Every word of that is true and none of it
/// tells the user they need to install something.
///
/// Meetily has the same shape of bug from the other end: it returns the raw
/// provider response body as the error string and shows it in the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderUnavailable {
    /// The `ollama` binary is not on PATH. Vox can start a server it can find
    /// and cannot install one.
    OllamaNotInstalled,
    /// Configured against a host that did not answer.
    OllamaUnreachable { host: String },
    /// Configured model is not installed in the local Ollama instance.
    OllamaModelNotFound { model: String },
    /// A cloud provider with no key saved for it.
    NoApiKey { provider: &'static str },
    /// `custom_openai` selected with nothing in the endpoint field.
    NoEndpoint,
}

impl std::fmt::Display for ProviderUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderUnavailable::OllamaNotInstalled => write!(
                f,
                "Ollama is not installed on this machine, so there is no local model to write \
                 the report with. Install it from ollama.com, or choose a different provider \
                 under Settings › AI Models & STT."
            ),
            ProviderUnavailable::OllamaUnreachable { host } => write!(
                f,
                "Nothing answered at {host}. Start Ollama, or point Vox at a different host \
                 under Settings › AI Models & STT."
            ),
            ProviderUnavailable::OllamaModelNotFound { model } => write!(
                f,
                "The Ollama model '{model}' is not installed locally. Run `ollama pull {model}` \
                 in your terminal, or select an installed model under Settings › AI Models & STT."
            ),
            ProviderUnavailable::NoApiKey { provider } => write!(
                f,
                "No API key is saved for {provider}. Add one under Settings › AI Models & STT, \
                 or switch to Ollama to keep everything on this machine."
            ),
            ProviderUnavailable::NoEndpoint => write!(
                f,
                "The custom provider has no endpoint URL. Set one under Settings › AI Models & \
                 STT — for a local server it is usually http://localhost:1234/v1."
            ),
        }
    }
}

/// A human name for a provider, for messages.
fn display_name(provider: &ProviderType) -> &'static str {
    match provider {
        ProviderType::Ollama => "Ollama",
        ProviderType::CloudOpenAI => "OpenAI",
        ProviderType::CloudGemini => "Google Gemini",
        ProviderType::CloudAnthropic => "Anthropic",
        ProviderType::Groq => "Groq",
        ProviderType::OpenRouter => "OpenRouter",
        ProviderType::CustomOpenAI => "the custom endpoint",
    }
}

/// Checks the configured provider can be reached before work is queued against it.
///
/// Only the local case costs a request, and it is a cheap one. A cloud
/// provider is checked for configuration rather than reachability: a key that
/// exists and is rejected is a different failure with a different message, and
/// spending a round trip to discover that before every summary is a poor
/// trade when the request itself is about to report it anyway.
pub async fn check_ready(config: &ProviderConfig) -> Result<(), ProviderUnavailable> {
    match config.active_provider {
        ProviderType::Ollama => match ensure_ollama_ready(&config.ollama_host, &config.ollama_model).await {
            OllamaStatus::Running | OllamaStatus::Started => {
                if !model_is_present(&config.ollama_host, &config.ollama_model).await {
                    return Err(ProviderUnavailable::OllamaModelNotFound {
                        model: config.ollama_model.clone(),
                    });
                }
                Ok(())
            }
            OllamaStatus::NotInstalled => Err(ProviderUnavailable::OllamaNotInstalled),
            OllamaStatus::Unreachable { .. } => Err(ProviderUnavailable::OllamaUnreachable {
                host: config.ollama_host.clone(),
            }),
        },
        ProviderType::CustomOpenAI => {
            if config
                .custom_openai_endpoint
                .as_deref()
                .map(str::trim)
                .filter(|e| !e.is_empty())
                .is_none()
            {
                return Err(ProviderUnavailable::NoEndpoint);
            }
            Ok(())
        }
        ref provider => {
            if config.active_api_key().is_none() {
                return Err(ProviderUnavailable::NoApiKey {
                    provider: display_name(provider),
                });
            }
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMResponse {
    pub text: String,
    pub model: String,
    pub prompt_tokens: Option<usize>,
    pub completion_tokens: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderType {
    #[serde(rename = "ollama")]
    Ollama,
    #[serde(rename = "cloud_openai")]
    CloudOpenAI,
    #[serde(rename = "cloud_gemini")]
    CloudGemini,
    #[serde(rename = "cloud_anthropic")]
    CloudAnthropic,
    #[serde(rename = "groq")]
    Groq,
    #[serde(rename = "openrouter")]
    OpenRouter,
    /// Any server that speaks the OpenAI chat-completions API, at an address
    /// the user gives.
    ///
    /// The one genuinely open extension point here: vLLM, LM Studio, LiteLLM,
    /// text-generation-webui and Azure OpenAI all work through it without a
    /// line of code, which is a better answer than adding a variant per
    /// vendor and is why Meetily's equivalent is the part of its provider
    /// layer worth copying.
    #[serde(rename = "custom_openai")]
    CustomOpenAI,
}

impl ProviderType {
    /// The stable string this provider is stored and keyed under.
    ///
    /// Matches the serde rename, and is what `provider_keys` uses — so a key
    /// saved for one provider is still there after switching away and back.
    pub fn slug(&self) -> &'static str {
        match self {
            ProviderType::Ollama => "ollama",
            ProviderType::CloudOpenAI => "cloud_openai",
            ProviderType::CloudGemini => "cloud_gemini",
            ProviderType::CloudAnthropic => "cloud_anthropic",
            ProviderType::Groq => "groq",
            ProviderType::OpenRouter => "openrouter",
            ProviderType::CustomOpenAI => "custom_openai",
        }
    }

    /// Whether this provider runs on the user's own machine.
    ///
    /// Ollama always does. A custom endpoint does when it points at loopback,
    /// which is the usual case — LM Studio and vLLM both listen there. The
    /// distinction is not cosmetic: it decides whether a meeting transcript
    /// leaves the machine, which is the one claim this app makes about itself.
    pub fn is_local(&self, endpoint: Option<&str>) -> bool {
        match self {
            ProviderType::Ollama => true,
            ProviderType::CustomOpenAI => endpoint.map(is_loopback_url).unwrap_or(false),
            _ => false,
        }
    }

    /// Whether this provider needs an API key to answer at all.
    ///
    /// A custom endpoint usually does not — a local vLLM accepts anything —
    /// so requiring one would lock out the configuration the variant exists
    /// to serve.
    pub fn requires_api_key(&self) -> bool {
        !matches!(
            self,
            ProviderType::Ollama | ProviderType::CustomOpenAI
        )
    }
}

/// Whether a URL points at this machine.
///
/// Host-only, and deliberately conservative: anything it cannot parse counts
/// as remote, because the cost of being wrong in that direction is a warning
/// the user does not need, and the cost the other way is telling someone their
/// transcript stays local when it does not.
fn is_loopback_url(url: &str) -> bool {
    let rest = match url.split_once("://") {
        Some((_, rest)) => rest,
        None => url,
    };
    let host = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .rsplit_once(':')
        .map(|(host, _port)| host)
        .unwrap_or_else(|| rest.split(['/', '?', '#']).next().unwrap_or(""));
    let host = host.trim_start_matches('[').trim_end_matches(']');
    matches!(host, "localhost" | "127.0.0.1" | "::1" | "0.0.0.0")
        || host.starts_with("127.")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub active_provider: ProviderType,
    pub ollama_host: String,
    pub ollama_model: String,
    pub cloud_api_key: Option<String>,
    pub cloud_model: Option<String>,
    /// Prompt+output window, in tokens, the local model is told to allocate.
    ///
    /// This exists because Ollama's default is 4096 (2048 on older builds) and
    /// it silently discards whatever does not fit — from the *front* of the
    /// prompt. A meeting transcript is the longest prompt Relay ever sends, so
    /// before this setting existed the first half of any meeting past roughly a
    /// quarter of an hour was dropped before the model read a word of it, with
    /// nothing in the response to say so. Raise it for a model that supports a
    /// larger window; the meeting pipeline sizes its own chunking from this
    /// number rather than assuming one.
    #[serde(default = "default_context_tokens", alias = "contextTokens")]
    pub context_tokens: u32,
    /// API keys, one per provider, keyed by [`ProviderType::slug`].
    ///
    /// `cloud_api_key` above holds exactly one, which was fine with one cloud
    /// provider configured at a time and is not with six: switching from
    /// OpenAI to Groq to compare them meant pasting a key, losing it, and
    /// pasting the first one back. Keys land here; `cloud_api_key` is still
    /// read as a fallback so an install that predates this keeps working.
    #[serde(default, alias = "providerKeys")]
    pub provider_keys: std::collections::BTreeMap<String, String>,
    /// Base URL for [`ProviderType::CustomOpenAI`], e.g. `http://localhost:1234/v1`.
    ///
    /// The path `/chat/completions` is appended, so this is the same value
    /// these servers call `base_url` and the one their own documentation
    /// prints.
    #[serde(default, alias = "customOpenaiEndpoint")]
    pub custom_openai_endpoint: Option<String>,
    /// Model name sent to the custom endpoint. Many ignore it; vLLM does not.
    #[serde(default, alias = "customOpenaiModel")]
    pub custom_openai_model: Option<String>,
}

impl ProviderConfig {
    /// The API key for whichever provider is active.
    ///
    /// Per-provider first, then the legacy single key — so an existing install
    /// keeps working and a new one stops overwriting itself.
    pub fn active_api_key(&self) -> Option<&str> {
        self.api_key_for(&self.active_provider)
    }

    /// The API key stored for one provider, if any.
    pub fn api_key_for(&self, provider: &ProviderType) -> Option<&str> {
        self.provider_keys
            .get(provider.slug())
            .map(String::as_str)
            .or(self.cloud_api_key.as_deref())
            .map(str::trim)
            .filter(|key| !key.is_empty())
    }

    /// Whether the active provider will keep the transcript on this machine.
    pub fn active_provider_is_local(&self) -> bool {
        self.active_provider
            .is_local(self.custom_openai_endpoint.as_deref())
    }
}

/// Chosen to fit a ~20-minute meeting in one pass on the 8k-window models
/// Relay's default local provider actually runs, without demanding memory a
/// laptop does not have.
fn default_context_tokens() -> u32 {
    8192
}

/// Characters one token is worth, for every budget Relay computes against a
/// context window.
///
/// One constant because the four places that used to carry their own were not
/// agreeing: the analysis spine estimated 3, Talkback's retrieval and the
/// context packs estimated 3.6. The failure modes are not symmetric. Too
/// conservative wastes a little of the window and nothing else; too optimistic
/// builds a prompt that does not fit, and Ollama does not refuse an overlong
/// prompt — it truncates it from the *front*, which is where the system
/// instructions live. So the estimate is deliberately low and the error is
/// taken in the direction that only costs headroom.
///
/// 3 rather than the ~4 English prose measures because Relay's content is not
/// only English prose: Devanagari tokenizes far denser, and a Hinglish standup
/// is the workload this app was built for.
///
/// It is an estimate, and tuning it no longer has to be guesswork:
/// `capture::decode_history` retains `char_count` and `word_count` for the last
/// 500 real decodes, so the length and script mix of the text Relay actually
/// sends is measurable. Pairing that with one tokenizer count is what would
/// justify moving this number — not another reading of what English prose
/// averages.
pub const CHARS_PER_TOKEN: usize = 3;

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            active_provider: ProviderType::Ollama,
            ollama_host: "http://localhost:11434".to_string(),
            ollama_model: "llama3.2:latest".to_string(),
            cloud_api_key: None,
            cloud_model: Some("gpt-4o-mini".to_string()),
            context_tokens: default_context_tokens(),
            provider_keys: std::collections::BTreeMap::new(),
            custom_openai_endpoint: None,
            custom_openai_model: None,
        }
    }
}

/// Sampling and budget for one completion.
///
/// Every field here used to be a provider default that Relay never expressed.
/// The consequential one is `temperature`: Ollama defaults to 0.8, so strict
/// JSON extraction from a meeting transcript was running at a creative-writing
/// setting, which is exactly the condition under which a model invents an owner
/// for an action item.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompletionOptions {
    pub temperature: f32,
    /// Cap on generated tokens. Bounds latency and stops a looping local model
    /// from running until the request times out.
    pub max_output_tokens: u32,
    /// The window the provider is asked to allocate for prompt plus output.
    pub context_tokens: u32,
    /// Wall-clock ceiling on the request. Without one a stalled local model
    /// leaves the UI on "Generating…" indefinitely.
    pub timeout_secs: u64,
}

impl Default for CompletionOptions {
    fn default() -> Self {
        Self {
            temperature: 0.3,
            max_output_tokens: 1_500,
            context_tokens: default_context_tokens(),
            timeout_secs: 300,
        }
    }
}

/// Where a cloud completion is sent, and how it authenticates.
///
/// Split out from the sender so the routing itself is testable. It has to be:
/// selecting Gemini or Anthropic used to post an OpenAI-shaped body to
/// `api.openai.com` with the user's key in an OpenAI header. That failed
/// authentication and fell through to canned filler, which is why those two
/// providers appeared to "work badly" rather than not at all — and it meant the
/// setting the user chose was not the service their meeting was sent to.
#[derive(Debug, Clone, PartialEq)]
pub struct CloudRoute {
    pub url: String,
    /// Header name and value carrying the API key.
    pub auth_header: (String, String),
    /// Extra headers the provider requires.
    pub extra_headers: Vec<(String, String)>,
}

pub struct LLMClient {
    config: ProviderConfig,
    client: reqwest::Client,
}

/// `LLMClient` is the one real implementation; every other is a test
/// stand-in. The methods delegate to the inherent ones, so there is a single
/// definition of what a verified completion is.
impl Completer for LLMClient {
    fn default_options(&self) -> CompletionOptions {
        LLMClient::default_options(self)
    }

    fn provider_type(&self) -> &ProviderType {
        LLMClient::provider_type(self)
    }

    fn model_name(&self) -> String {
        LLMClient::model_name(self)
    }

    fn complete_verified<'a>(
        &'a self,
        prompt: &'a str,
        system_prompt: Option<&'a str>,
        options: CompletionOptions,
    ) -> BoxFuture<'a, Result<LLMResponse, ProviderError>> {
        Box::pin(LLMClient::complete_verified(self, prompt, system_prompt, options))
    }
}

impl LLMClient {
    pub fn new(config: ProviderConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }

    pub fn config(&self) -> &ProviderConfig {
        &self.config
    }

    /// Completes without masking a provider failure.
    ///
    /// [`complete`](Self::complete) substitutes canned filler when a provider is
    /// unreachable, which suits dictation — some output beats none — and is
    /// actively wrong for a meeting summary, where filler presented as a model's
    /// work would be validated, persisted, and shown as an AI summary. Callers
    /// that need to know whether a model actually answered use this.
    pub async fn complete_with(
        &self,
        prompt: &str,
        system_prompt: Option<&str>,
        options: CompletionOptions,
    ) -> Result<LLMResponse, ProviderError> {
        match self.config.active_provider {
            ProviderType::Ollama => self.complete_ollama_with(prompt, system_prompt, options).await,
            _ => self.complete_cloud_with(prompt, system_prompt, options).await,
        }
    }

    /// The Ollama request body.
    ///
    /// The `options` object is the whole point. Without it Ollama applies its own
    /// defaults — a 4096-token window (2048 on older builds) and temperature 0.8
    /// — and silently drops the overflowing *front* of the prompt. For a meeting
    /// transcript, the front is the agenda.
    pub fn ollama_request_body(
        model: &str,
        prompt: &str,
        system_prompt: Option<&str>,
        options: CompletionOptions,
    ) -> serde_json::Value {
        let mut body = serde_json::json!({
            "model": model,
            "prompt": prompt,
            "stream": false,
            "options": {
                "temperature": options.temperature,
                "num_ctx": options.context_tokens,
                "num_predict": options.max_output_tokens,
            }
        });
        if let Some(sys) = system_prompt {
            body["system"] = serde_json::Value::String(sys.to_string());
        }
        body
    }

    /// The OpenAI chat-completions body.
    pub fn openai_request_body(
        model: &str,
        prompt: &str,
        system_prompt: Option<&str>,
        options: CompletionOptions,
    ) -> serde_json::Value {
        let mut messages = Vec::new();
        if let Some(sys) = system_prompt {
            messages.push(serde_json::json!({ "role": "system", "content": sys }));
        }
        messages.push(serde_json::json!({ "role": "user", "content": prompt }));

        serde_json::json!({
            "model": model,
            "messages": messages,
            "temperature": options.temperature,
            "max_completion_tokens": options.max_output_tokens,
        })
    }

    /// The Anthropic messages body. The system prompt is a top-level field here,
    /// not a message role.
    pub fn anthropic_request_body(
        model: &str,
        prompt: &str,
        system_prompt: Option<&str>,
        options: CompletionOptions,
    ) -> serde_json::Value {
        let mut body = serde_json::json!({
            "model": model,
            "max_tokens": options.max_output_tokens,
            "temperature": options.temperature,
            "messages": [{ "role": "user", "content": prompt }],
        });
        if let Some(sys) = system_prompt {
            body["system"] = serde_json::Value::String(sys.to_string());
        }
        body
    }

    /// The Gemini `generateContent` body. System instructions and generation
    /// config are both separate objects here.
    pub fn gemini_request_body(
        prompt: &str,
        system_prompt: Option<&str>,
        options: CompletionOptions,
    ) -> serde_json::Value {
        let mut body = serde_json::json!({
            "contents": [{ "role": "user", "parts": [{ "text": prompt }] }],
            "generationConfig": {
                "temperature": options.temperature,
                "maxOutputTokens": options.max_output_tokens,
            }
        });
        if let Some(sys) = system_prompt {
            body["systemInstruction"] = serde_json::json!({ "parts": [{ "text": sys }] });
        }
        body
    }

    /// Resolves which service a cloud completion goes to.
    pub fn cloud_route(
        provider: &ProviderType,
        model: &str,
        api_key: &str,
    ) -> Result<CloudRoute, ProviderError> {
        match provider {
            ProviderType::CloudOpenAI => Ok(CloudRoute {
                url: "https://api.openai.com/v1/chat/completions".to_string(),
                auth_header: (
                    "Authorization".to_string(),
                    format!("Bearer {}", api_key),
                ),
                extra_headers: Vec::new(),
            }),
            ProviderType::CloudAnthropic => Ok(CloudRoute {
                url: "https://api.anthropic.com/v1/messages".to_string(),
                auth_header: ("x-api-key".to_string(), api_key.to_string()),
                extra_headers: vec![(
                    "anthropic-version".to_string(),
                    "2023-06-01".to_string(),
                )],
            }),
            ProviderType::CloudGemini => Ok(CloudRoute {
                url: format!(
                    "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
                    model
                ),
                auth_header: ("x-goog-api-key".to_string(), api_key.to_string()),
                extra_headers: Vec::new(),
            }),
            // Groq and OpenRouter both serve the OpenAI chat-completions
            // shape, so only the address and the key header differ.
            ProviderType::Groq => Ok(CloudRoute {
                url: "https://api.groq.com/openai/v1/chat/completions".to_string(),
                auth_header: ("Authorization".to_string(), format!("Bearer {}", api_key)),
                extra_headers: Vec::new(),
            }),
            ProviderType::OpenRouter => Ok(CloudRoute {
                url: "https://openrouter.ai/api/v1/chat/completions".to_string(),
                auth_header: ("Authorization".to_string(), format!("Bearer {}", api_key)),
                // OpenRouter attributes requests by these and rate-limits
                // unattributed traffic harder. Constant, and nothing about the
                // user is in them.
                extra_headers: vec![
                    ("HTTP-Referer".to_string(), "https://github.com/Nitinsudarshan/Vox".to_string()),
                    ("X-Title".to_string(), "Vox".to_string()),
                ],
            }),
            ProviderType::CustomOpenAI => Err(ProviderError::ConfigError(
                "a custom endpoint is routed from its configured URL, not from here".to_string(),
            )),
            ProviderType::Ollama => Err(ProviderError::ConfigError(
                "Ollama is not a cloud provider".to_string(),
            )),
        }
    }

    /// The route for a user-supplied OpenAI-compatible endpoint.
    ///
    /// Separate from [`cloud_route`](Self::cloud_route) because the address is
    /// data rather than a constant, and because it is the one route that can
    /// be misconfigured — so this is where that is caught, with a message that
    /// says what to type.
    pub fn custom_route(endpoint: &str, api_key: Option<&str>) -> Result<CloudRoute, ProviderError> {
        let base = endpoint.trim().trim_end_matches('/');
        if base.is_empty() {
            return Err(ProviderError::ConfigError(
                "Set the endpoint URL for the custom provider, e.g. http://localhost:1234/v1"
                    .to_string(),
            ));
        }
        if !base.starts_with("http://") && !base.starts_with("https://") {
            return Err(ProviderError::ConfigError(format!(
                "'{base}' is not a URL. It should start with http:// or https://"
            )));
        }
        // Plain HTTP is right for a server on this machine and wrong for one
        // that is not: an API key over unencrypted HTTP to a remote host is
        // the key in clear text on every hop between here and there. Meetily
        // accepts either without comment; this refuses the one that leaks.
        if base.starts_with("http://") && !is_loopback_url(base) && api_key.is_some() {
            return Err(ProviderError::ConfigError(format!(
                "'{base}' is plain HTTP and not on this machine, so the API key would be sent \
                 unencrypted. Use https://, or clear the key if the endpoint needs none."
            )));
        }

        // Both spellings are in the wild — some servers document the `/v1`
        // and some do not — and appending blindly gives `/v1/v1`.
        let url = if base.ends_with("/chat/completions") {
            base.to_string()
        } else {
            format!("{base}/chat/completions")
        };

        Ok(CloudRoute {
            url,
            auth_header: (
                "Authorization".to_string(),
                format!("Bearer {}", api_key.unwrap_or("")),
            ),
            extra_headers: Vec::new(),
        })
    }

    /// Pulls the completion text out of whichever response shape came back.
    pub fn extract_cloud_text(provider: &ProviderType, json: &serde_json::Value) -> String {
        match provider {
            ProviderType::CloudAnthropic => json["content"]
                .as_array()
                .map(|blocks| {
                    blocks
                        .iter()
                        .filter_map(|b| b["text"].as_str())
                        .collect::<Vec<_>>()
                        .join("")
                })
                .unwrap_or_default(),
            ProviderType::CloudGemini => json["candidates"][0]["content"]["parts"]
                .as_array()
                .map(|parts| {
                    parts
                        .iter()
                        .filter_map(|p| p["text"].as_str())
                        .collect::<Vec<_>>()
                        .join("")
                })
                .unwrap_or_default(),
            _ => json["choices"][0]["message"]["content"]
                .as_str()
                .unwrap_or("")
                .to_string(),
        }
    }

    /// The default model for a cloud provider, used when settings name none.
    fn default_cloud_model(provider: &ProviderType) -> &'static str {
        match provider {
            ProviderType::CloudAnthropic => "claude-sonnet-4-5",
            ProviderType::CloudGemini => "gemini-2.0-flash",
            ProviderType::Groq => "llama-3.3-70b-versatile",
            ProviderType::OpenRouter => "openai/gpt-4o-mini",
            // Nothing sensible to guess: a custom server hosts whatever it
            // hosts. Many ignore the field entirely, and the ones that do not
            // reject a name they have never heard of — which is a clearer
            // failure than silently answering from the wrong model.
            ProviderType::CustomOpenAI => "",
            _ => "gpt-4o-mini",
        }
    }

    async fn complete_ollama_with(
        &self,
        prompt: &str,
        system_prompt: Option<&str>,
        options: CompletionOptions,
    ) -> Result<LLMResponse, ProviderError> {
        let url = format!("{}/api/generate", self.config.ollama_host);
        let body = Self::ollama_request_body(
            &self.config.ollama_model,
            prompt,
            system_prompt,
            options,
        );

        let res = self
            .client
            .post(&url)
            .timeout(std::time::Duration::from_secs(options.timeout_secs))
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::OllamaUnavailable {
                host: self.config.ollama_host.clone(),
                message: e.to_string(),
            })?;

        if !res.status().is_success() {
            let status = res.status();
            let msg = if status.as_u16() == 404 {
                format!(
                    "Model '{}' was not found in Ollama. Pull it with `ollama pull {}` or select an installed model in Settings.",
                    self.config.ollama_model, self.config.ollama_model
                )
            } else {
                format!("HTTP {}", status)
            };
            return Err(ProviderError::OllamaUnavailable {
                host: self.config.ollama_host.clone(),
                message: msg,
            });
        }

        let json: serde_json::Value = res.json().await?;
        // Ollama reports how much of the prompt it actually evaluated. When that
        // is short of the window it was given, the prompt was truncated — the
        // failure this whole options object exists to prevent — so say so
        // rather than letting a half-read transcript pass as a full one.
        if let Some(evaluated) = json["prompt_eval_count"].as_u64() {
            if evaluated as u32 >= options.context_tokens.saturating_sub(options.max_output_tokens)
            {
                tracing::warn!(
                    evaluated_tokens = evaluated,
                    context_tokens = options.context_tokens,
                    "provider: prompt filled the model's context window; input may have been truncated"
                );
            }
        }

        Ok(LLMResponse {
            text: json["response"].as_str().unwrap_or("").to_string(),
            model: self.config.ollama_model.clone(),
            prompt_tokens: json["prompt_eval_count"].as_u64().map(|v| v as usize),
            completion_tokens: json["eval_count"].as_u64().map(|v| v as usize),
        })
    }

    /// The route for whichever provider is active.
    ///
    /// One place that knows a custom endpoint is routed from configuration
    /// rather than from a constant, so the completion path and the streaming
    /// path cannot disagree about it.
    fn route_for(
        &self,
        provider: &ProviderType,
        model: &str,
        api_key: Option<&str>,
    ) -> Result<CloudRoute, ProviderError> {
        match provider {
            ProviderType::CustomOpenAI => Self::custom_route(
                self.config.custom_openai_endpoint.as_deref().unwrap_or(""),
                api_key,
            ),
            other => Self::cloud_route(other, model, api_key.unwrap_or("")),
        }
    }

    /// The model name to send, whichever provider is active.
    fn effective_model(&self) -> String {
        let provider = &self.config.active_provider;
        let configured = match provider {
            ProviderType::CustomOpenAI => self.config.custom_openai_model.as_deref(),
            _ => self.config.cloud_model.as_deref(),
        };
        configured
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| Self::default_cloud_model(provider))
            .to_string()
    }

    async fn complete_cloud_with(
        &self,
        prompt: &str,
        system_prompt: Option<&str>,
        options: CompletionOptions,
    ) -> Result<LLMResponse, ProviderError> {
        let provider = &self.config.active_provider;
        let api_key = self.config.active_api_key();
        if provider.requires_api_key() && api_key.is_none() {
            return Err(ProviderError::ConfigError(format!(
                "No API key is set for {}.",
                provider.slug()
            )));
        }

        let model = self.effective_model();
        let route = self.route_for(provider, &model, api_key)?;
        let body = match provider {
            ProviderType::CloudAnthropic => {
                Self::anthropic_request_body(&model, prompt, system_prompt, options)
            }
            ProviderType::CloudGemini => {
                Self::gemini_request_body(prompt, system_prompt, options)
            }
            _ => Self::openai_request_body(&model, prompt, system_prompt, options),
        };

        let mut request = self
            .client
            .post(&route.url)
            .timeout(std::time::Duration::from_secs(options.timeout_secs))
            .header(&route.auth_header.0, &route.auth_header.1);
        for (name, value) in &route.extra_headers {
            request = request.header(name, value);
        }

        let res = request.json(&body).send().await?;
        if !res.status().is_success() {
            let code = res.status().as_u16().to_string();
            let error_text = res.text().await.unwrap_or_default();
            return Err(ProviderError::CloudError {
                code,
                message: error_text,
            });
        }

        let json: serde_json::Value = res.json().await?;
        Ok(LLMResponse {
            text: Self::extract_cloud_text(provider, &json),
            model: model.to_string(),
            prompt_tokens: json["usage"]["prompt_tokens"]
                .as_u64()
                .or_else(|| json["usage"]["input_tokens"].as_u64())
                .map(|v| v as usize),
            completion_tokens: json["usage"]["completion_tokens"]
                .as_u64()
                .or_else(|| json["usage"]["output_tokens"].as_u64())
                .map(|v| v as usize),
        })
    }

    pub async fn complete(
        &self,
        prompt: &str,
        system_prompt: Option<&str>,
    ) -> Result<LLMResponse, ProviderError> {
        match self.config.active_provider {
            ProviderType::Ollama => match self.complete_ollama(prompt, system_prompt).await {
                Ok(resp) => Ok(resp),
                Err(err) => {
                    tracing::warn!(
                        "Ollama unavailable ({}), using local heuristic fallback",
                        err
                    );
                    Ok(Self::heuristic_fallback(prompt, system_prompt))
                }
            },
            // Everything else is a cloud or cloud-shaped endpoint, and they
            // all fail the same way: filler rather than an error, because this
            // entry point serves dictation, where some output beats none.
            // `complete_verified` is the one analysis and summaries use.
            _ => {
                match self.complete_cloud(prompt, system_prompt).await {
                    Ok(resp) => Ok(resp),
                    Err(err) => {
                        tracing::warn!(
                            "Cloud provider failed ({}), using local heuristic fallback",
                            err
                        );
                        Ok(Self::heuristic_fallback(prompt, system_prompt))
                    }
                }
            }
        }
    }

    /// Completes, and fails when no model actually answered.
    ///
    /// [`complete_with`](Self::complete_with) already reports a transport
    /// failure honestly, but that is only half the problem: a caller that built
    /// its own [`LLMClient`] elsewhere, or a future path that routes through
    /// [`complete`](Self::complete), can still be handed filler tagged
    /// [`HEURISTIC_FALLBACK_MODEL`]. This is the one entry point analysis code
    /// should use, because for analysis the two cases are the same case —
    /// nothing was understood, and saying otherwise puts invented content into
    /// the vault under a model's name.
    pub async fn complete_verified(
        &self,
        prompt: &str,
        system_prompt: Option<&str>,
        options: CompletionOptions,
    ) -> Result<LLMResponse, ProviderError> {
        let response = self.complete_with(prompt, system_prompt, options).await?;
        if response.model == HEURISTIC_FALLBACK_MODEL {
            return Err(ProviderError::NoCompletion(
                "provider unreachable; the client substituted heuristic filler".to_string(),
            ));
        }
        if response.text.trim().is_empty() {
            return Err(ProviderError::EmptyCompletion);
        }
        Ok(response)
    }

    pub fn heuristic_fallback(prompt: &str, system_prompt: Option<&str>) -> LLMResponse {
        let is_json = system_prompt.map(|s| s.contains("JSON")).unwrap_or(false);
        if is_json {
            let sys = system_prompt.unwrap_or("");
            if sys.contains("Knowledge & Thinking Assistant") || sys.contains("thought/scribble") || sys.contains("topics") || sys.contains("entities") {
                let structured = crate::pipeline::extract_deterministic_knowledge(prompt);
                let json_text = serde_json::to_string_pretty(&structured).unwrap_or_else(|_| "{}".to_string());
                return LLMResponse {
                    text: json_text,
                    model: HEURISTIC_FALLBACK_MODEL.to_string(),
                    prompt_tokens: None,
                    completion_tokens: None,
                };
            }

            let first_line = prompt.lines().next().unwrap_or(prompt).trim();
            let title = if first_line.is_empty() {
                "Follow up on meeting action items"
            } else {
                first_line
            };

            let json_text = serde_json::json!([
                {
                    "title": title.chars().take(80).collect::<String>(),
                    "assignee": "Unassigned",
                    "priority": "medium",
                    "due_date": serde_json::Value::Null,
                    "description": prompt
                }
            ]).to_string();

            LLMResponse {
                text: json_text,
                model: HEURISTIC_FALLBACK_MODEL.to_string(),
                prompt_tokens: None,
                completion_tokens: None,
            }
        } else {
            let markdown = format!(
                "# Executive Summary\n- {}\n\n## Key Decisions & Context\n- Recorded via Relay push-to-talk voice capture.\n- Saved to local vault (.relay/vault/notes).\n\n## Next Steps\n- Review extracted tasks and notes.",
                if prompt.trim().is_empty() { "Voice scribble captured" } else { prompt.trim() }
            );

            LLMResponse {
                text: markdown,
                model: HEURISTIC_FALLBACK_MODEL.to_string(),
                prompt_tokens: None,
                completion_tokens: None,
            }
        }
    }

    /// The default-options path, kept so existing callers read unchanged.
    ///
    /// Delegates rather than duplicating the request: dictation and Scribble
    /// benefit from the same window, temperature, and timeout the meeting
    /// pipeline needed, and one request builder cannot drift from another.
    async fn complete_ollama(
        &self,
        prompt: &str,
        system_prompt: Option<&str>,
    ) -> Result<LLMResponse, ProviderError> {
        self.complete_ollama_with(prompt, system_prompt, self.default_options())
            .await
    }

    async fn complete_cloud(
        &self,
        prompt: &str,
        system_prompt: Option<&str>,
    ) -> Result<LLMResponse, ProviderError> {
        self.complete_cloud_with(prompt, system_prompt, self.default_options())
            .await
    }

    /// Completion options that honour the user's configured window.
    pub fn default_options(&self) -> CompletionOptions {
        CompletionOptions {
            context_tokens: self.config.context_tokens.max(2_048),
            ..CompletionOptions::default()
        }
    }

    /// The window the caller may fill, in tokens.
    pub fn context_tokens(&self) -> u32 {
        self.config.context_tokens.max(2_048)
    }

    /// The active provider, so callers can report which service answered.
    pub fn provider_type(&self) -> &ProviderType {
        &self.config.active_provider
    }

    /// The model that will actually be asked, for observability.
    ///
    /// Also the [`Completer::model_name`] answer: read from configuration
    /// rather than from a response, so it stays answerable when nothing came
    /// back. `meetings_v2::processing::llm::ProviderLlm` used to derive this
    /// same mapping privately, with `unwrap_or_default()` where this falls back
    /// to the provider's default cloud model — so the migration onto this one
    /// also stops an unset `cloud_model` recording provenance as the empty
    /// string.
    pub fn model_name(&self) -> String {
        match self.config.active_provider {
            ProviderType::Ollama => self.config.ollama_model.clone(),
            // Delegates so provenance records the name that was actually sent.
            // A second copy of this resolution is a provenance entry that
            // disagrees with the request it describes.
            _ => self.effective_model(),
        }
    }

    /// Streams a completion, calling `on_delta` with each fragment as it
    /// arrives.
    ///
    /// Feeds `tts::SpeechQueue`: a phrase pushed from inside this callback is
    /// synthesized while the model is still writing the next one, which makes
    /// time to first audio *first phrase plus one synthesis* rather than
    /// *whole generation plus synthesis*. Talkback, which this was originally
    /// written for, no longer exists (`docs/decisions.md` Decision 69); the
    /// argument for streaming does, and `tts::phrases` is the splitter.
    ///
    /// It exists because waiting for a whole answer before
    /// speaking any of it is the difference between a conversation and a
    /// form submission. Everything else in Relay is batch work — a
    /// meeting summary has nobody waiting on its first sentence — and
    /// keeps using [`complete_with`](Self::complete_with).
    ///
    /// `on_delta` returns `false` to abandon the stream, which is how
    /// barge-in stops a generation the user has already talked over. A
    /// cancelled stream returns whatever had accumulated so far rather
    /// than an error: the partial answer was really said, and the caller
    /// still needs it for the transcript.
    pub async fn complete_streaming<F>(
        &self,
        prompt: &str,
        system_prompt: Option<&str>,
        options: CompletionOptions,
        mut on_delta: F,
    ) -> Result<LLMResponse, ProviderError>
    where
        F: FnMut(&str) -> bool,
    {
        let provider = self.config.active_provider.clone();
        let model = self.model_name();

        let (url, body, headers) = self.streaming_request(&model, prompt, system_prompt, options)?;

        let mut request = self
            .client
            .post(&url)
            .timeout(std::time::Duration::from_secs(options.timeout_secs));
        for (name, value) in &headers {
            request = request.header(name, value);
        }

        let mut response = request.json(&body).send().await.map_err(|e| match provider {
            ProviderType::Ollama => ProviderError::OllamaUnavailable {
                host: self.config.ollama_host.clone(),
                message: e.to_string(),
            },
            _ => ProviderError::Network(e),
        })?;

        if !response.status().is_success() {
            let code = response.status().as_u16().to_string();
            let message = response.text().await.unwrap_or_default();
            return match provider {
                ProviderType::Ollama => Err(ProviderError::OllamaUnavailable {
                    host: self.config.ollama_host.clone(),
                    message: format!("HTTP {code}: {message}"),
                }),
                _ => Err(ProviderError::CloudError { code, message }),
            };
        }

        let mut text = String::new();
        // Bytes, not a String: a chunk boundary can fall inside a
        // multi-byte character, and decoding each chunk on its own would
        // corrupt Devanagari mid-word.
        let mut pending: Vec<u8> = Vec::new();
        let mut finished = false;

        // `Response::chunk` rather than `bytes_stream` deliberately: it
        // needs no `stream` feature on reqwest and no futures-util
        // dependency, so streaming costs Relay zero new crates.
        while let Some(chunk) = response.chunk().await? {
            pending.extend_from_slice(&chunk);
            while let Some(newline) = pending.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = pending.drain(..=newline).collect();
                let line = String::from_utf8_lossy(&line);
                match parse_stream_line(&provider, line.trim_end_matches(['\r', '\n'])) {
                    StreamEvent::Delta(delta) => {
                        text.push_str(&delta);
                        if !on_delta(&delta) {
                            return Ok(LLMResponse {
                                text,
                                model,
                                prompt_tokens: None,
                                completion_tokens: None,
                            });
                        }
                    }
                    StreamEvent::Done => finished = true,
                    StreamEvent::Ignore => {}
                }
            }
            if finished {
                break;
            }
        }

        // A final line with no trailing newline still carries a delta.
        if !pending.is_empty() {
            let line = String::from_utf8_lossy(&pending);
            if let StreamEvent::Delta(delta) =
                parse_stream_line(&provider, line.trim_end_matches(['\r', '\n']))
            {
                text.push_str(&delta);
                on_delta(&delta);
            }
        }

        Ok(LLMResponse {
            text,
            model,
            prompt_tokens: None,
            completion_tokens: None,
        })
    }

    /// URL, body and headers for a streaming request.
    fn streaming_request(
        &self,
        model: &str,
        prompt: &str,
        system_prompt: Option<&str>,
        options: CompletionOptions,
    ) -> Result<StreamingRequest, ProviderError> {
        match self.config.active_provider {
            ProviderType::Ollama => {
                let mut body =
                    Self::ollama_request_body(model, prompt, system_prompt, options);
                body["stream"] = serde_json::Value::Bool(true);
                Ok((
                    format!("{}/api/generate", self.config.ollama_host),
                    body,
                    Vec::new(),
                ))
            }
            ref provider => {
                let api_key = self.config.active_api_key();
                if provider.requires_api_key() && api_key.is_none() {
                    return Err(ProviderError::ConfigError(format!(
                        "No API key is set for {}.",
                        provider.slug()
                    )));
                }
                let route = self.route_for(provider, model, api_key)?;

                let mut body = match provider {
                    ProviderType::CloudAnthropic => {
                        Self::anthropic_request_body(model, prompt, system_prompt, options)
                    }
                    ProviderType::CloudGemini => {
                        Self::gemini_request_body(prompt, system_prompt, options)
                    }
                    _ => Self::openai_request_body(model, prompt, system_prompt, options),
                };

                // Gemini signals streaming in the URL, everyone else in
                // the body.
                let url = if matches!(provider, ProviderType::CloudGemini) {
                    format!(
                        "https://generativelanguage.googleapis.com/v1beta/models/{model}:streamGenerateContent?alt=sse"
                    )
                } else {
                    body["stream"] = serde_json::Value::Bool(true);
                    route.url.clone()
                };

                let mut headers = vec![route.auth_header.clone()];
                headers.extend(route.extra_headers.clone());
                Ok((url, body, headers))
            }
        }
    }
}

/// A streaming request: where to post, what to post, and the headers it
/// needs. Named because the four providers disagree on all three.
type StreamingRequest = (String, serde_json::Value, Vec<(String, String)>);

/// One parsed line of a provider's response stream.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    /// Text to append and speak.
    Delta(String),
    /// The provider said the generation is complete.
    Done,
    /// Keep-alive, metadata, or a line this provider does not use.
    Ignore,
}

/// Parses one line of a provider's stream.
///
/// Pure, and public, because the four providers disagree about everything
/// — NDJSON versus SSE, where the text lives, how completion is signalled
/// — and the only way to be sure Relay reads all four correctly without a
/// network is to test this function directly.
pub fn parse_stream_line(provider: &ProviderType, line: &str) -> StreamEvent {
    let line = line.trim();
    if line.is_empty() {
        return StreamEvent::Ignore;
    }

    match provider {
        // Ollama streams newline-delimited JSON with no `data:` prefix.
        ProviderType::Ollama => {
            let Ok(json) = serde_json::from_str::<serde_json::Value>(line) else {
                return StreamEvent::Ignore;
            };
            if let Some(text) = json["response"].as_str() {
                if !text.is_empty() {
                    return StreamEvent::Delta(text.to_string());
                }
            }
            if json["done"].as_bool().unwrap_or(false) {
                return StreamEvent::Done;
            }
            StreamEvent::Ignore
        }
        // The rest are server-sent events.
        provider => {
            // SSE carries `event:` and comment lines too; only `data:`
            // frames hold content.
            let Some(payload) = line.strip_prefix("data:") else {
                return StreamEvent::Ignore;
            };
            let payload = payload.trim();
            if payload == "[DONE]" {
                return StreamEvent::Done;
            }
            let Ok(json) = serde_json::from_str::<serde_json::Value>(payload) else {
                return StreamEvent::Ignore;
            };

            match provider {
                ProviderType::CloudAnthropic => {
                    match json["type"].as_str() {
                        Some("content_block_delta") => json["delta"]["text"]
                            .as_str()
                            .filter(|t| !t.is_empty())
                            .map(|t| StreamEvent::Delta(t.to_string()))
                            .unwrap_or(StreamEvent::Ignore),
                        Some("message_stop") => StreamEvent::Done,
                        _ => StreamEvent::Ignore,
                    }
                }
                ProviderType::CloudGemini => json["candidates"][0]["content"]["parts"]
                    .as_array()
                    .map(|parts| {
                        parts
                            .iter()
                            .filter_map(|p| p["text"].as_str())
                            .collect::<String>()
                    })
                    .filter(|t| !t.is_empty())
                    .map(StreamEvent::Delta)
                    .unwrap_or(StreamEvent::Ignore),
                // OpenAI and anything else OpenAI-shaped.
                _ => json["choices"][0]["delta"]["content"]
                    .as_str()
                    .filter(|t| !t.is_empty())
                    .map(|t| StreamEvent::Delta(t.to_string()))
                    .unwrap_or(StreamEvent::Ignore),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> CompletionOptions {
        CompletionOptions {
            temperature: 0.1,
            max_output_tokens: 900,
            context_tokens: 16_384,
            timeout_secs: 60,
        }
    }

    #[test]
    fn the_ollama_body_states_the_window_instead_of_accepting_the_default() {
        // The regression this pins: without `options`, Ollama used a 4096-token
        // window and silently dropped the front of any longer prompt.
        let body = LLMClient::ollama_request_body("llama3.2", "transcript", Some("rules"), options());
        assert_eq!(body["options"]["num_ctx"], 16_384);
        assert_eq!(body["options"]["num_predict"], 900);
        assert_eq!(body["options"]["temperature"].as_f64().unwrap(), 0.1_f32 as f64);
        assert_eq!(body["system"], "rules");
        assert_eq!(body["stream"], false);
    }

    #[test]
    fn the_system_prompt_is_a_field_not_a_prefix_on_the_user_prompt() {
        // It used to be folded into the prompt as "[System Instructions: ...]",
        // which put Relay's rules and the meeting's own words in one
        // undifferentiated string.
        let body = LLMClient::ollama_request_body("m", "transcript", Some("rules"), options());
        assert_eq!(body["prompt"], "transcript");
        assert!(!body["prompt"].as_str().unwrap().contains("System Instructions"));
    }

    #[test]
    fn each_cloud_provider_is_routed_to_its_own_service() {
        let openai = LLMClient::cloud_route(&ProviderType::CloudOpenAI, "gpt-4o-mini", "k").unwrap();
        assert!(openai.url.contains("api.openai.com"));
        assert_eq!(openai.auth_header.0, "Authorization");

        let anthropic =
            LLMClient::cloud_route(&ProviderType::CloudAnthropic, "claude-sonnet-4-5", "k").unwrap();
        assert!(anthropic.url.contains("api.anthropic.com"));
        assert_eq!(anthropic.auth_header.0, "x-api-key");
        assert!(anthropic
            .extra_headers
            .iter()
            .any(|(n, _)| n == "anthropic-version"));

        let gemini =
            LLMClient::cloud_route(&ProviderType::CloudGemini, "gemini-2.0-flash", "k").unwrap();
        assert!(gemini.url.contains("generativelanguage.googleapis.com"));
        assert!(gemini.url.contains("gemini-2.0-flash"));
        assert_eq!(gemini.auth_header.0, "x-goog-api-key");
    }

    #[test]
    fn no_cloud_provider_is_routed_to_another_ones_endpoint() {
        for (provider, host) in [
            (ProviderType::CloudOpenAI, "api.openai.com"),
            (ProviderType::CloudAnthropic, "api.anthropic.com"),
            (ProviderType::CloudGemini, "generativelanguage.googleapis.com"),
        ] {
            let route = LLMClient::cloud_route(&provider, "model", "key").unwrap();
            assert!(
                route.url.contains(host),
                "{:?} must not be sent to another provider's endpoint",
                provider
            );
        }
    }

    #[test]
    fn each_provider_gets_the_body_shape_it_actually_accepts() {
        let anthropic =
            LLMClient::anthropic_request_body("claude-sonnet-4-5", "user text", Some("rules"), options());
        assert_eq!(anthropic["system"], "rules");
        assert_eq!(anthropic["max_tokens"], 900);
        assert!(anthropic["messages"][0]["content"] == "user text");
        assert!(anthropic.get("messages").is_some() && anthropic["messages"].as_array().unwrap().len() == 1);

        let gemini = LLMClient::gemini_request_body("user text", Some("rules"), options());
        assert_eq!(gemini["systemInstruction"]["parts"][0]["text"], "rules");
        assert_eq!(gemini["generationConfig"]["maxOutputTokens"], 900);
        assert_eq!(gemini["contents"][0]["parts"][0]["text"], "user text");

        let openai = LLMClient::openai_request_body("gpt-4o-mini", "user text", Some("rules"), options());
        assert_eq!(openai["messages"][0]["role"], "system");
        assert_eq!(openai["messages"][1]["content"], "user text");
        assert_eq!(openai["max_completion_tokens"], 900);
    }

    #[test]
    fn text_is_read_out_of_each_providers_response_shape() {
        let anthropic = serde_json::json!({"content": [{"type": "text", "text": "hello "}, {"type": "text", "text": "world"}]});
        assert_eq!(
            LLMClient::extract_cloud_text(&ProviderType::CloudAnthropic, &anthropic),
            "hello world"
        );

        let gemini = serde_json::json!({"candidates": [{"content": {"parts": [{"text": "hi"}]}}]});
        assert_eq!(
            LLMClient::extract_cloud_text(&ProviderType::CloudGemini, &gemini),
            "hi"
        );

        let openai = serde_json::json!({"choices": [{"message": {"content": "hey"}}]});
        assert_eq!(
            LLMClient::extract_cloud_text(&ProviderType::CloudOpenAI, &openai),
            "hey"
        );
    }

    #[test]
    fn a_configured_window_below_the_floor_is_raised_rather_than_honoured() {
        let client = LLMClient::new(ProviderConfig {
            context_tokens: 512,
            ..Default::default()
        });
        assert_eq!(client.context_tokens(), 2_048);
    }

    #[tokio::test]
    async fn an_unreachable_provider_is_an_error_here_rather_than_filler() {
        let client = LLMClient::new(ProviderConfig {
            ollama_host: "http://127.0.0.1:1".to_string(),
            ..Default::default()
        });

        let masked = client.complete("prompt", Some("system")).await.unwrap();
        assert_eq!(masked.model, "heuristic-fallback");

        let honest = client
            .complete_with("prompt", Some("system"), CompletionOptions::default())
            .await;
        assert!(
            honest.is_err(),
            "a summary must never be written from canned filler presented as a model's work"
        );
    }

    #[test]
    fn ollama_ndjson_deltas_are_parsed() {
        let event = parse_stream_line(
            &ProviderType::Ollama,
            r#"{"model":"llama3.2","response":"Flat ","done":false}"#,
        );
        assert_eq!(event, StreamEvent::Delta("Flat ".to_string()));
    }

    #[test]
    fn ollama_signals_completion_with_done() {
        assert_eq!(
            parse_stream_line(&ProviderType::Ollama, r#"{"response":"","done":true}"#),
            StreamEvent::Done
        );
    }

    #[test]
    fn openai_sse_deltas_are_parsed() {
        let event = parse_stream_line(
            &ProviderType::CloudOpenAI,
            r#"data: {"choices":[{"delta":{"content":"seat "}}]}"#,
        );
        assert_eq!(event, StreamEvent::Delta("seat ".to_string()));
        assert_eq!(
            parse_stream_line(&ProviderType::CloudOpenAI, "data: [DONE]"),
            StreamEvent::Done
        );
    }

    #[test]
    fn openai_role_only_frames_are_ignored() {
        assert_eq!(
            parse_stream_line(
                &ProviderType::CloudOpenAI,
                r#"data: {"choices":[{"delta":{"role":"assistant"}}]}"#
            ),
            StreamEvent::Ignore
        );
    }

    #[test]
    fn anthropic_content_block_deltas_are_parsed() {
        let event = parse_stream_line(
            &ProviderType::CloudAnthropic,
            r#"data: {"type":"content_block_delta","delta":{"type":"text_delta","text":"licence"}}"#,
        );
        assert_eq!(event, StreamEvent::Delta("licence".to_string()));
        assert_eq!(
            parse_stream_line(
                &ProviderType::CloudAnthropic,
                r#"data: {"type":"message_stop"}"#
            ),
            StreamEvent::Done
        );
        assert_eq!(
            parse_stream_line(
                &ProviderType::CloudAnthropic,
                r#"data: {"type":"ping"}"#
            ),
            StreamEvent::Ignore
        );
    }

    #[test]
    fn gemini_sse_deltas_are_parsed() {
        let event = parse_stream_line(
            &ProviderType::CloudGemini,
            r#"data: {"candidates":[{"content":{"parts":[{"text":"We shipped"}]}}]}"#,
        );
        assert_eq!(event, StreamEvent::Delta("We shipped".to_string()));
    }

    #[test]
    fn sse_comments_and_event_lines_are_ignored() {
        for provider in [
            ProviderType::CloudOpenAI,
            ProviderType::CloudAnthropic,
            ProviderType::CloudGemini,
        ] {
            assert_eq!(parse_stream_line(&provider, ": keep-alive"), StreamEvent::Ignore);
            assert_eq!(
                parse_stream_line(&provider, "event: content_block_delta"),
                StreamEvent::Ignore
            );
            assert_eq!(parse_stream_line(&provider, ""), StreamEvent::Ignore);
        }
    }

    #[test]
    fn malformed_json_is_skipped_rather_than_fatal() {
        // A truncated frame must not abort a generation the user is
        // already listening to.
        assert_eq!(
            parse_stream_line(&ProviderType::Ollama, r#"{"response":"half"#),
            StreamEvent::Ignore
        );
        assert_eq!(
            parse_stream_line(&ProviderType::CloudOpenAI, "data: {not json"),
            StreamEvent::Ignore
        );
    }

    #[test]
    fn the_streaming_ollama_body_sets_stream_true_and_keeps_the_window() {
        let client = LLMClient::new(ProviderConfig::default());
        let (url, body, headers) = client
            .streaming_request("llama3.2", "question", Some("rules"), options())
            .unwrap();
        assert!(url.ends_with("/api/generate"));
        assert_eq!(body["stream"], true);
        assert_eq!(body["options"]["num_ctx"], 16_384);
        assert_eq!(body["system"], "rules");
        assert!(headers.is_empty());
    }

    #[test]
    fn the_streaming_gemini_request_uses_the_sse_endpoint() {
        let client = LLMClient::new(ProviderConfig {
            active_provider: ProviderType::CloudGemini,
            cloud_api_key: Some("key".to_string()),
            cloud_model: Some("gemini-2.0-flash".to_string()),
            ..Default::default()
        });
        let (url, body, headers) = client
            .streaming_request("gemini-2.0-flash", "q", None, options())
            .unwrap();
        assert!(url.contains(":streamGenerateContent?alt=sse"), "{url}");
        assert!(
            body.get("stream").is_none(),
            "Gemini signals streaming in the URL, not the body"
        );
        assert!(headers.iter().any(|(k, _)| k == "x-goog-api-key"));
    }

    #[test]
    fn the_streaming_anthropic_request_carries_the_version_header() {
        let client = LLMClient::new(ProviderConfig {
            active_provider: ProviderType::CloudAnthropic,
            cloud_api_key: Some("key".to_string()),
            ..Default::default()
        });
        let (url, body, headers) = client
            .streaming_request("claude-sonnet-4-5", "q", Some("sys"), options())
            .unwrap();
        assert_eq!(url, "https://api.anthropic.com/v1/messages");
        assert_eq!(body["stream"], true);
        assert_eq!(body["system"], "sys");
        assert!(headers.iter().any(|(k, v)| k == "anthropic-version" && v == "2023-06-01"));
    }

    #[test]
    fn a_cloud_stream_without_a_key_fails_before_the_request() {
        let client = LLMClient::new(ProviderConfig {
            active_provider: ProviderType::CloudOpenAI,
            cloud_api_key: None,
            ..Default::default()
        });
        assert!(matches!(
            client.streaming_request("gpt-4o-mini", "q", None, options()),
            Err(ProviderError::ConfigError(_))
        ));
    }

    #[test]
    fn the_model_name_falls_back_to_the_provider_default() {
        let ollama = LLMClient::new(ProviderConfig::default());
        assert_eq!(ollama.model_name(), "llama3.2:latest");

        let anthropic = LLMClient::new(ProviderConfig {
            active_provider: ProviderType::CloudAnthropic,
            cloud_model: Some("  ".to_string()),
            ..Default::default()
        });
        assert_eq!(anthropic.model_name(), "claude-sonnet-4-5");
    }

    #[tokio::test]
    async fn an_unreachable_provider_fails_the_stream_rather_than_hanging() {
        let client = LLMClient::new(ProviderConfig {
            ollama_host: "http://127.0.0.1:1".to_string(),
            ..Default::default()
        });
        let result = client
            .complete_streaming("q", None, CompletionOptions::default(), |_| true)
            .await;
        assert!(matches!(
            result,
            Err(ProviderError::OllamaUnavailable { .. })
        ));
    }
    #[test]
    fn the_openai_shaped_providers_route_to_their_own_hosts() {
        for (provider, host) in [
            (ProviderType::Groq, "api.groq.com"),
            (ProviderType::OpenRouter, "openrouter.ai"),
        ] {
            let route = LLMClient::cloud_route(&provider, "whatever", "k").unwrap();
            assert!(route.url.contains(host), "{} routed to {}", provider.slug(), route.url);
            assert_eq!(route.auth_header.0, "Authorization");
            assert_eq!(route.auth_header.1, "Bearer k");
        }
    }

    #[test]
    fn a_custom_endpoint_gets_the_completions_path_exactly_once() {
        // Both spellings are in the wild — some servers document the `/v1` and
        // some do not — and appending blindly gives `/v1/v1`.
        for given in [
            "http://localhost:1234/v1",
            "http://localhost:1234/v1/",
            "http://localhost:1234/v1/chat/completions",
        ] {
            let route = LLMClient::custom_route(given, None).unwrap();
            assert_eq!(route.url, "http://localhost:1234/v1/chat/completions");
        }
    }

    #[test]
    fn a_custom_endpoint_that_is_not_a_url_says_what_to_type() {
        let err = LLMClient::custom_route("localhost:1234", None).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("http://"), "unhelpful message: {message}");

        let err = LLMClient::custom_route("   ", None).unwrap_err();
        assert!(err.to_string().contains("endpoint URL"));
    }

    #[test]
    fn a_key_is_never_sent_unencrypted_to_a_remote_endpoint() {
        // Plain HTTP on this machine is the ordinary case — LM Studio and vLLM
        // both listen there — and plain HTTP to somewhere else with a key
        // attached puts that key in clear text on every hop.
        assert!(LLMClient::custom_route("http://localhost:1234/v1", Some("k")).is_ok());
        assert!(LLMClient::custom_route("http://127.0.0.1:8000/v1", Some("k")).is_ok());
        assert!(LLMClient::custom_route("https://models.example.com/v1", Some("k")).is_ok());
        // No key, nothing to leak.
        assert!(LLMClient::custom_route("http://models.example.com/v1", None).is_ok());

        let err = LLMClient::custom_route("http://models.example.com/v1", Some("k")).unwrap_err();
        assert!(err.to_string().contains("unencrypted"), "{err}");
    }

    #[test]
    fn a_key_saved_for_one_provider_survives_switching_away_and_back() {
        let mut config = ProviderConfig {
            active_provider: ProviderType::Groq,
            ..ProviderConfig::default()
        };
        config
            .provider_keys
            .insert("groq".into(), "groq-key".into());
        config
            .provider_keys
            .insert("cloud_openai".into(), "openai-key".into());

        assert_eq!(config.active_api_key(), Some("groq-key"));
        config.active_provider = ProviderType::CloudOpenAI;
        assert_eq!(config.active_api_key(), Some("openai-key"));
    }

    #[test]
    fn an_install_that_predates_per_provider_keys_still_has_one() {
        let config = ProviderConfig {
            active_provider: ProviderType::CloudAnthropic,
            cloud_api_key: Some("legacy".into()),
            ..ProviderConfig::default()
        };
        assert_eq!(config.active_api_key(), Some("legacy"));
    }

    #[test]
    fn a_blank_key_counts_as_no_key() {
        let mut config = ProviderConfig {
            active_provider: ProviderType::Groq,
            ..ProviderConfig::default()
        };
        config.provider_keys.insert("groq".into(), "   ".into());
        assert_eq!(config.active_api_key(), None);
    }

    #[test]
    fn only_ollama_and_a_loopback_endpoint_count_as_local() {
        assert!(ProviderType::Ollama.is_local(None));
        assert!(ProviderType::CustomOpenAI.is_local(Some("http://localhost:1234/v1")));
        assert!(ProviderType::CustomOpenAI.is_local(Some("http://127.0.0.1:8000/v1")));
        assert!(!ProviderType::CustomOpenAI.is_local(Some("https://models.example.com/v1")));
        // Unconfigured is not a promise that it stays here.
        assert!(!ProviderType::CustomOpenAI.is_local(None));
        assert!(!ProviderType::Groq.is_local(None));
        assert!(!ProviderType::CloudOpenAI.is_local(None));
    }

    #[test]
    fn only_the_providers_that_need_a_key_demand_one() {
        assert!(!ProviderType::Ollama.requires_api_key());
        // A local vLLM accepts anything; demanding a key would lock out the
        // configuration this variant exists to serve.
        assert!(!ProviderType::CustomOpenAI.requires_api_key());
        for provider in [
            ProviderType::CloudOpenAI,
            ProviderType::CloudAnthropic,
            ProviderType::CloudGemini,
            ProviderType::Groq,
            ProviderType::OpenRouter,
        ] {
            assert!(provider.requires_api_key(), "{}", provider.slug());
        }
    }

    #[test]
    fn every_provider_slug_is_its_serde_name() {
        for provider in [
            ProviderType::Ollama,
            ProviderType::CloudOpenAI,
            ProviderType::CloudGemini,
            ProviderType::CloudAnthropic,
            ProviderType::Groq,
            ProviderType::OpenRouter,
            ProviderType::CustomOpenAI,
        ] {
            let serialized = serde_json::to_string(&provider).unwrap();
            assert_eq!(serialized, format!("\"{}\"", provider.slug()));
        }
    }

    #[test]
    fn a_custom_endpoint_reports_the_model_it_was_given_rather_than_openais() {
        let client = LLMClient::new(ProviderConfig {
            active_provider: ProviderType::CustomOpenAI,
            custom_openai_endpoint: Some("http://localhost:1234/v1".into()),
            custom_openai_model: Some("qwen2.5-7b-instruct".into()),
            cloud_model: Some("gpt-4o-mini".into()),
            ..ProviderConfig::default()
        });
        assert_eq!(client.model_name(), "qwen2.5-7b-instruct");
    }
    #[tokio::test]
    async fn a_cloud_provider_with_no_key_names_the_provider_and_the_fix() {
        let config = ProviderConfig {
            active_provider: ProviderType::Groq,
            ..ProviderConfig::default()
        };
        let reason = check_ready(&config).await.unwrap_err();
        assert_eq!(reason, ProviderUnavailable::NoApiKey { provider: "Groq" });

        let message = reason.to_string();
        assert!(message.contains("Groq"), "{message}");
        assert!(message.contains("Settings"), "{message}");
    }

    #[tokio::test]
    async fn a_cloud_provider_with_a_key_is_not_checked_over_the_network() {
        // Deliberate: a key that exists and is rejected is a different failure
        // with a different message, and spending a round trip to discover that
        // before every summary is a poor trade when the request itself is
        // about to report it anyway.
        let mut config = ProviderConfig {
            active_provider: ProviderType::CloudOpenAI,
            ..ProviderConfig::default()
        };
        config
            .provider_keys
            .insert("cloud_openai".into(), "sk-whatever".into());
        assert!(check_ready(&config).await.is_ok());
    }

    #[tokio::test]
    async fn a_custom_provider_with_no_endpoint_says_what_to_type() {
        let config = ProviderConfig {
            active_provider: ProviderType::CustomOpenAI,
            ..ProviderConfig::default()
        };
        let reason = check_ready(&config).await.unwrap_err();
        assert_eq!(reason, ProviderUnavailable::NoEndpoint);
        assert!(reason.to_string().contains("http://localhost:1234/v1"));
    }

    #[tokio::test]
    async fn a_custom_provider_needs_an_endpoint_and_not_a_key() {
        let config = ProviderConfig {
            active_provider: ProviderType::CustomOpenAI,
            custom_openai_endpoint: Some("http://localhost:1234/v1".into()),
            ..ProviderConfig::default()
        };
        assert!(check_ready(&config).await.is_ok());
    }

    #[tokio::test]
    async fn a_remote_ollama_that_does_not_answer_names_the_host() {
        // Port 1 on loopback is closed everywhere, and a non-local host skips
        // the "try to start it ourselves" branch.
        let config = ProviderConfig {
            active_provider: ProviderType::Ollama,
            ollama_host: "http://198.51.100.7:1".into(),
            ..ProviderConfig::default()
        };
        let reason = check_ready(&config).await.unwrap_err();
        assert_eq!(
            reason,
            ProviderUnavailable::OllamaUnreachable {
                host: "http://198.51.100.7:1".into()
            }
        );
        assert!(reason.to_string().contains("198.51.100.7"));
    }

    #[test]
    fn every_unavailable_reason_says_where_to_go() {
        // The whole point of the type. A message that reports a failure
        // without naming the fix is the string this replaced.
        for reason in [
            ProviderUnavailable::OllamaNotInstalled,
            ProviderUnavailable::OllamaUnreachable {
                host: "http://localhost:11434".into(),
            },
            ProviderUnavailable::NoApiKey { provider: "OpenAI" },
            ProviderUnavailable::NoEndpoint,
        ] {
            let message = reason.to_string();
            assert!(
                message.contains("Settings") || message.contains("ollama.com"),
                "no route to a fix in: {message}"
            );
        }
    }

    #[test]
    fn every_provider_has_a_display_name() {
        for provider in [
            ProviderType::Ollama,
            ProviderType::CloudOpenAI,
            ProviderType::CloudGemini,
            ProviderType::CloudAnthropic,
            ProviderType::Groq,
            ProviderType::OpenRouter,
            ProviderType::CustomOpenAI,
        ] {
            assert!(!display_name(&provider).is_empty());
        }
    }
}
