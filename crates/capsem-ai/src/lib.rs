use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GenerationError {
    #[error("missing provider credential for {provider}; checked {checked:?}")]
    MissingCredential {
        provider: String,
        checked: Vec<String>,
    },
    #[error("missing model configuration for {provider} {capability}; checked {checked:?}")]
    MissingModel {
        provider: String,
        capability: &'static str,
        checked: Vec<String>,
    },
    #[error("unsupported generation capability: {capability}")]
    UnsupportedCapability { capability: &'static str },
    #[error("provider error: {0}")]
    Provider(String),
    #[error("failed to read settings {path:?}: {details}")]
    SettingsRead { path: PathBuf, details: String },
    #[error("failed to parse settings {path:?}: {details}")]
    SettingsParse { path: PathBuf, details: String },
}

pub type Result<T> = std::result::Result<T, GenerationError>;

#[derive(Clone, Debug, Default)]
pub struct GenerationSettings {
    credentials: BTreeMap<String, String>,
    models: BTreeMap<String, GenerationModelDefaults>,
}

#[derive(Clone, Debug, Default)]
struct GenerationModelDefaults {
    text: Option<String>,
    image: Option<String>,
    embedding: Option<String>,
}

impl GenerationSettings {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn from_service_toml_str(input: &str) -> Result<Self> {
        let settings: ServiceSettingsToml =
            toml::from_str(input).map_err(|source| GenerationError::SettingsParse {
                path: PathBuf::from("<inline>"),
                details: source.to_string(),
            })?;
        Ok(Self::from_service_settings(settings))
    }

    pub fn from_service_toml_file(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let input = fs::read_to_string(path).map_err(|source| GenerationError::SettingsRead {
            path: path.to_path_buf(),
            details: source.to_string(),
        })?;
        let settings: ServiceSettingsToml =
            toml::from_str(&input).map_err(|source| GenerationError::SettingsParse {
                path: path.to_path_buf(),
                details: source.to_string(),
            })?;
        Ok(Self::from_service_settings(settings))
    }

    pub fn from_capsem_environment() -> Result<Self> {
        let Some(path) = service_settings_path() else {
            return Ok(Self::default());
        };
        match fs::read_to_string(&path) {
            Ok(input) => {
                let settings: ServiceSettingsToml =
                    toml::from_str(&input).map_err(|source| GenerationError::SettingsParse {
                        path,
                        details: source.to_string(),
                    })?;
                Ok(Self::from_service_settings(settings))
            }
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(GenerationError::SettingsRead {
                path,
                details: source.to_string(),
            }),
        }
    }

    pub fn with_credential(mut self, id: impl Into<String>, value: impl Into<String>) -> Self {
        self.credentials.insert(id.into(), value.into());
        self
    }

    pub fn with_model(
        mut self,
        provider: impl AsRef<str>,
        capability: &'static str,
        model: impl Into<String>,
    ) -> Self {
        self.set_model(provider.as_ref(), capability, model.into());
        self
    }

    pub fn resolve_api_key(&self, provider: &str) -> Result<ResolvedCredential> {
        let candidates = credential_candidates(provider);
        for id in &candidates {
            if let Some(value) = self.credentials.get(id).and_then(resolve_credential_value) {
                return Ok(ResolvedCredential {
                    source: format!("settings:{id}"),
                    value,
                });
            }
        }
        for env_name in provider_env_candidates(provider) {
            if let Ok(value) = env::var(env_name) {
                let value = value.trim().to_owned();
                if !value.is_empty() {
                    return Ok(ResolvedCredential {
                        source: format!("env:{env_name}"),
                        value,
                    });
                }
            }
        }
        Err(GenerationError::MissingCredential {
            provider: provider.to_owned(),
            checked: candidates
                .into_iter()
                .map(|id| format!("settings:{id}"))
                .chain(
                    provider_env_candidates(provider)
                        .into_iter()
                        .map(|id| format!("env:{id}")),
                )
                .collect(),
        })
    }

    fn from_service_settings(settings: ServiceSettingsToml) -> Self {
        let credentials = settings
            .credentials
            .items
            .into_iter()
            .map(|(id, credential)| (id, credential.value))
            .collect();
        let mut output = Self {
            credentials,
            models: BTreeMap::new(),
        };
        for (provider, models) in settings.ai.models {
            output.set_models(&provider, models);
        }
        output
    }

    fn set_models(&mut self, provider: &str, models: ModelDefaultsToml) {
        if let Some(model) = models.text.and_then(non_empty) {
            self.set_model(provider, "text", model);
        }
        if let Some(model) = models.image.and_then(non_empty) {
            self.set_model(provider, "image", model);
        }
        if let Some(model) = models.embedding.and_then(non_empty) {
            self.set_model(provider, "embedding", model);
        }
    }

    fn set_model(&mut self, provider: &str, capability: &'static str, model: String) {
        let entry = self.models.entry(canonical_provider(provider)).or_default();
        match capability {
            "text" => entry.text = Some(model),
            "image" => entry.image = Some(model),
            "embedding" => entry.embedding = Some(model),
            _ => {}
        }
    }

    fn resolve_model(
        &self,
        provider: &str,
        capability: &'static str,
        requested: Option<String>,
    ) -> Result<String> {
        if let Some(model) = requested.and_then(non_empty) {
            return Ok(model);
        }
        let model = self
            .models
            .get(provider)
            .and_then(|models| match capability {
                "text" => models.text.clone(),
                "image" => models.image.clone(),
                "embedding" => models.embedding.clone(),
                _ => None,
            })
            .and_then(non_empty);
        model.ok_or_else(|| GenerationError::MissingModel {
            provider: provider.to_owned(),
            capability,
            checked: vec![format!("[ai.models.{provider}].{capability}")],
        })
    }
}

#[derive(Clone, Debug)]
pub struct ResolvedCredential {
    pub source: String,
    value: String,
}

impl ResolvedCredential {
    pub fn expose_for_provider_client(&self) -> String {
        self.value.clone()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateTextRequest {
    pub provider: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub system: Option<String>,
    pub prompt: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateImageRequest {
    pub provider: String,
    #[serde(default)]
    pub model: Option<String>,
    pub prompt: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateEmbeddingRequest {
    pub provider: String,
    #[serde(default)]
    pub model: Option<String>,
    pub input: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct ResolvedTextRequest {
    pub provider: String,
    pub model: String,
    pub system: Option<String>,
    pub prompt: String,
    pub credential: ResolvedCredential,
}

#[derive(Clone, Debug)]
pub struct ResolvedImageRequest {
    pub provider: String,
    pub model: String,
    pub prompt: String,
    pub credential: ResolvedCredential,
}

#[derive(Clone, Debug)]
pub struct ResolvedEmbeddingRequest {
    pub provider: String,
    pub model: String,
    pub input: Vec<String>,
    pub credential: ResolvedCredential,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GenerationUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_prompt_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_prompt_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_usage: Option<Value>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GenerationCost {
    pub estimated_usd: f64,
    pub currency: String,
    pub source: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextGeneration {
    pub provider: String,
    pub model: String,
    pub text: String,
    pub usage: Option<GenerationUsage>,
    pub cost: Option<GenerationCost>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageGeneration {
    pub provider: String,
    pub model: String,
    pub mime_type: Option<String>,
    pub data_url: Option<String>,
    pub url: Option<String>,
    pub revised_prompt: Option<String>,
    pub usage: Option<GenerationUsage>,
    pub cost: Option<GenerationCost>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingGeneration {
    pub provider: String,
    pub model: String,
    pub dimensions: usize,
    pub vectors: Vec<EmbeddingVector>,
    pub usage: Option<GenerationUsage>,
    pub cost: Option<GenerationCost>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingVector {
    pub index: u32,
    pub values: Vec<f64>,
}

#[async_trait]
pub trait ModelProvider: Send + Sync {
    async fn generate_text(&self, request: ResolvedTextRequest) -> Result<TextGeneration>;
    async fn generate_image(&self, request: ResolvedImageRequest) -> Result<ImageGeneration>;
    async fn generate_embedding(
        &self,
        request: ResolvedEmbeddingRequest,
    ) -> Result<EmbeddingGeneration>;
}

#[derive(Clone, Debug)]
pub struct GenerationEngine<P = CapsemHttpModelProvider> {
    settings: GenerationSettings,
    provider: P,
}

impl GenerationEngine<CapsemHttpModelProvider> {
    pub fn from_capsem_environment() -> Result<Self> {
        Ok(Self::new(
            GenerationSettings::from_capsem_environment()?,
            CapsemHttpModelProvider::default(),
        ))
    }
}

impl<P> GenerationEngine<P>
where
    P: ModelProvider,
{
    pub fn new(settings: GenerationSettings, provider: P) -> Self {
        Self { settings, provider }
    }

    pub async fn generate_text(&self, request: GenerateTextRequest) -> Result<TextGeneration> {
        require_non_empty("prompt", &request.prompt)?;
        let provider = canonical_provider(&request.provider);
        let credential = self.settings.resolve_api_key(&provider)?;
        self.provider
            .generate_text(ResolvedTextRequest {
                model: self
                    .settings
                    .resolve_model(&provider, "text", request.model)?,
                provider,
                system: request.system,
                prompt: request.prompt,
                credential,
            })
            .await
    }

    pub async fn generate_image(&self, request: GenerateImageRequest) -> Result<ImageGeneration> {
        require_non_empty("prompt", &request.prompt)?;
        let provider = canonical_provider(&request.provider);
        let credential = self.settings.resolve_api_key(&provider)?;
        self.provider
            .generate_image(ResolvedImageRequest {
                model: self
                    .settings
                    .resolve_model(&provider, "image", request.model)?,
                provider,
                prompt: request.prompt,
                credential,
            })
            .await
    }

    pub async fn generate_embedding(
        &self,
        request: GenerateEmbeddingRequest,
    ) -> Result<EmbeddingGeneration> {
        if request.input.is_empty() {
            return Err(GenerationError::Provider(
                "input must contain at least one item".to_owned(),
            ));
        }
        for value in &request.input {
            require_non_empty("input", value)?;
        }
        let provider = canonical_provider(&request.provider);
        let credential = self.settings.resolve_api_key(&provider)?;
        self.provider
            .generate_embedding(ResolvedEmbeddingRequest {
                model: self
                    .settings
                    .resolve_model(&provider, "embedding", request.model)?,
                provider,
                input: request.input,
                credential,
            })
            .await
    }
}

#[derive(Clone, Debug, Default)]
pub struct FakeModelProvider;

#[async_trait]
impl ModelProvider for FakeModelProvider {
    async fn generate_text(&self, request: ResolvedTextRequest) -> Result<TextGeneration> {
        Ok(TextGeneration {
            provider: request.provider,
            model: request.model,
            text: format!("fake: {}", request.prompt),
            usage: Some(fake_usage()),
            cost: Some(fake_cost()),
        })
    }

    async fn generate_image(&self, request: ResolvedImageRequest) -> Result<ImageGeneration> {
        Ok(ImageGeneration {
            provider: request.provider,
            model: request.model,
            mime_type: Some("image/png".to_owned()),
            data_url: Some("data:image/png;base64,ZmFrZQ==".to_owned()),
            url: None,
            revised_prompt: Some(request.prompt),
            usage: None,
            cost: None,
        })
    }

    async fn generate_embedding(
        &self,
        request: ResolvedEmbeddingRequest,
    ) -> Result<EmbeddingGeneration> {
        let vectors = request
            .input
            .iter()
            .enumerate()
            .map(|(index, value)| EmbeddingVector {
                index: index as u32,
                values: vec![value.len() as f64, index as f64, 1.0],
            })
            .collect();
        Ok(EmbeddingGeneration {
            provider: request.provider,
            model: request.model,
            dimensions: 3,
            vectors,
            usage: Some(fake_usage()),
            cost: Some(fake_cost()),
        })
    }
}

#[derive(Clone, Debug)]
pub struct CapsemHttpModelProvider {
    client: reqwest::Client,
}

impl Default for CapsemHttpModelProvider {
    fn default() -> Self {
        Self {
            client: reqwest::Client::builder()
                .user_agent("capsem-ai/0.1")
                .build()
                .expect("reqwest client configuration is static"),
        }
    }
}

#[async_trait]
impl ModelProvider for CapsemHttpModelProvider {
    async fn generate_text(&self, request: ResolvedTextRequest) -> Result<TextGeneration> {
        ensure_provider_supports(&request.provider, "text")?;
        match request.provider.as_str() {
            "openai" => self.generate_openai_text(request).await,
            "google" => self.generate_google_text(request).await,
            "anthropic" => self.generate_anthropic_text(request).await,
            other => Err(unsupported(other, "text")),
        }
    }

    async fn generate_image(&self, request: ResolvedImageRequest) -> Result<ImageGeneration> {
        ensure_provider_supports(&request.provider, "image")?;
        match request.provider.as_str() {
            "openai" => self.generate_openai_image(request).await,
            "google" => self.generate_google_image(request).await,
            other => Err(unsupported(other, "image")),
        }
    }

    async fn generate_embedding(
        &self,
        request: ResolvedEmbeddingRequest,
    ) -> Result<EmbeddingGeneration> {
        ensure_provider_supports(&request.provider, "embedding")?;
        match request.provider.as_str() {
            "openai" => self.generate_openai_embedding(request).await,
            "google" => self.generate_google_embedding(request).await,
            other => Err(unsupported(other, "embedding")),
        }
    }
}

impl CapsemHttpModelProvider {
    async fn generate_openai_text(&self, request: ResolvedTextRequest) -> Result<TextGeneration> {
        let body = json!({
            "model": request.model,
            "messages": openai_messages(&request.prompt, request.system.as_deref())
        });
        let value = self
            .post_json(
                self.client
                    .post("https://api.openai.com/v1/chat/completions")
                    .bearer_auth(request.credential.expose_for_provider_client()),
                &body,
            )
            .await?;
        let response: OpenAiChatResponse = parse_provider_value(value)?;
        let usage = response.usage.map(openai_usage);
        let cost = usage
            .as_ref()
            .and_then(|usage| estimate_cost("openai", &response.model, usage));
        Ok(TextGeneration {
            provider: request.provider,
            model: response.model,
            text: response
                .choices
                .first()
                .map(|choice| choice.message.content.clone())
                .unwrap_or_default(),
            usage,
            cost,
        })
    }

    async fn generate_openai_image(
        &self,
        request: ResolvedImageRequest,
    ) -> Result<ImageGeneration> {
        let body = json!({
            "model": request.model,
            "prompt": request.prompt,
            "n": 1
        });
        let value = self
            .post_json(
                self.client
                    .post("https://api.openai.com/v1/images/generations")
                    .bearer_auth(request.credential.expose_for_provider_client()),
                &body,
            )
            .await?;
        let response: OpenAiImageResponse = parse_provider_value(value)?;
        let Some(first) = response.data.first() else {
            return Err(GenerationError::Provider(
                "provider response did not include an image".to_owned(),
            ));
        };
        let usage = response.usage.map(openai_usage);
        let cost = usage
            .as_ref()
            .and_then(|usage| estimate_cost("openai", &request.model, usage));
        Ok(ImageGeneration {
            provider: request.provider,
            model: request.model,
            mime_type: first.b64_json.as_ref().map(|_| "image/png".to_owned()),
            data_url: first
                .b64_json
                .as_ref()
                .map(|b64| format!("data:image/png;base64,{b64}")),
            url: first.url.clone(),
            revised_prompt: first.revised_prompt.clone(),
            usage,
            cost,
        })
    }

    async fn generate_openai_embedding(
        &self,
        request: ResolvedEmbeddingRequest,
    ) -> Result<EmbeddingGeneration> {
        let body = json!({
            "model": request.model,
            "input": request.input
        });
        let value = self
            .post_json(
                self.client
                    .post("https://api.openai.com/v1/embeddings")
                    .bearer_auth(request.credential.expose_for_provider_client()),
                &body,
            )
            .await?;
        let response: OpenAiEmbeddingResponse = parse_provider_value(value)?;
        let dimensions = response
            .data
            .first()
            .map(|vector| vector.embedding.len())
            .unwrap_or_default();
        let usage = response.usage.map(openai_usage);
        let cost = usage
            .as_ref()
            .and_then(|usage| estimate_cost("openai", &response.model, usage));
        Ok(EmbeddingGeneration {
            provider: request.provider,
            model: response.model,
            dimensions,
            vectors: response
                .data
                .into_iter()
                .map(|vector| EmbeddingVector {
                    index: vector.index,
                    values: vector.embedding,
                })
                .collect(),
            usage,
            cost,
        })
    }

    async fn generate_google_text(&self, request: ResolvedTextRequest) -> Result<TextGeneration> {
        let body = google_generate_content_body(&request.prompt, request.system.as_deref(), None);
        let value = self
            .post_json(
                self.client
                    .post(google_generate_content_url(&request.model))
                    .header(
                        "x-goog-api-key",
                        request.credential.expose_for_provider_client(),
                    ),
                &body,
            )
            .await?;
        let response: GeminiGenerateContentResponse = parse_provider_value(value)?;
        let text = response.first_text().unwrap_or_default();
        let usage = response.usage_metadata.map(gemini_usage);
        let cost = usage
            .as_ref()
            .and_then(|usage| estimate_cost("google", &request.model, usage));
        Ok(TextGeneration {
            provider: request.provider,
            model: response.model_version.unwrap_or(request.model),
            text,
            usage,
            cost,
        })
    }

    async fn generate_google_image(
        &self,
        request: ResolvedImageRequest,
    ) -> Result<ImageGeneration> {
        let body = google_generate_content_body(
            &request.prompt,
            None,
            Some(json!({"responseModalities": ["TEXT", "IMAGE"]})),
        );
        let value = self
            .post_json(
                self.client
                    .post(google_generate_content_url(&request.model))
                    .header(
                        "x-goog-api-key",
                        request.credential.expose_for_provider_client(),
                    ),
                &body,
            )
            .await?;
        let response: GeminiGenerateContentResponse = parse_provider_value(value)?;
        let image = response.first_inline_image().ok_or_else(|| {
            GenerationError::Provider("provider response did not include an image".to_owned())
        })?;
        let revised_prompt = response.first_text();
        let usage = response.usage_metadata.map(gemini_usage);
        let cost = usage
            .as_ref()
            .and_then(|usage| estimate_cost("google", &request.model, usage));
        Ok(ImageGeneration {
            provider: request.provider,
            model: response.model_version.unwrap_or(request.model),
            mime_type: Some(image.mime_type.clone()),
            data_url: Some(format!("data:{};base64,{}", image.mime_type, image.data)),
            url: None,
            revised_prompt,
            usage,
            cost,
        })
    }

    async fn generate_google_embedding(
        &self,
        request: ResolvedEmbeddingRequest,
    ) -> Result<EmbeddingGeneration> {
        let model_path = google_model_path(&request.model);
        let mut vectors = Vec::with_capacity(request.input.len());
        for (index, input) in request.input.iter().enumerate() {
            let body = json!({
                "model": model_path,
                "content": {"parts": [{"text": input}]}
            });
            let value = self
                .post_json(
                    self.client
                        .post(format!(
                            "https://generativelanguage.googleapis.com/v1beta/{}:embedContent",
                            model_path
                        ))
                        .header(
                            "x-goog-api-key",
                            request.credential.expose_for_provider_client(),
                        ),
                    &body,
                )
                .await?;
            let response: GeminiEmbeddingResponse = parse_provider_value(value)?;
            vectors.push(EmbeddingVector {
                index: index as u32,
                values: response.embedding.values,
            });
        }
        let dimensions = vectors
            .first()
            .map(|vector| vector.values.len())
            .unwrap_or_default();
        Ok(EmbeddingGeneration {
            provider: request.provider,
            model: request.model,
            dimensions,
            vectors,
            usage: None,
            cost: None,
        })
    }

    async fn generate_anthropic_text(
        &self,
        request: ResolvedTextRequest,
    ) -> Result<TextGeneration> {
        let mut body = json!({
            "model": request.model,
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": request.prompt}]
        });
        if let Some(system) = request.system.filter(|system| !system.trim().is_empty()) {
            body["system"] = json!(system);
        }
        let value = self
            .post_json(
                self.client
                    .post("https://api.anthropic.com/v1/messages")
                    .header("anthropic-version", "2023-06-01")
                    .header("x-api-key", request.credential.expose_for_provider_client()),
                &body,
            )
            .await?;
        let response: AnthropicMessageResponse = parse_provider_value(value)?;
        let usage = response.usage.map(anthropic_usage);
        let cost = usage
            .as_ref()
            .and_then(|usage| estimate_cost("anthropic", &response.model, usage));
        Ok(TextGeneration {
            provider: request.provider,
            model: response.model,
            text: response
                .content
                .into_iter()
                .filter_map(|part| part.text)
                .collect::<Vec<_>>()
                .join("\n"),
            usage,
            cost,
        })
    }

    async fn post_json(&self, request: reqwest::RequestBuilder, body: &Value) -> Result<Value> {
        let response = request
            .json(body)
            .send()
            .await
            .map_err(|error| GenerationError::Provider(error.to_string()))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|error| GenerationError::Provider(error.to_string()))?;
        let value = serde_json::from_str::<Value>(&text).unwrap_or_else(|_| json!({"raw": text}));
        if !status.is_success() {
            return Err(GenerationError::Provider(format!(
                "{}: {}",
                status.as_u16(),
                provider_error_message(&value)
            )));
        }
        Ok(value)
    }
}

#[derive(Debug, Deserialize)]
struct OpenAiChatResponse {
    model: String,
    choices: Vec<OpenAiChoice>,
    usage: Option<OpenAiUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    message: OpenAiMessage,
}

#[derive(Debug, Deserialize)]
struct OpenAiMessage {
    content: String,
}

#[derive(Debug, Deserialize)]
struct OpenAiImageResponse {
    data: Vec<OpenAiImage>,
    usage: Option<OpenAiUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenAiImage {
    #[serde(default)]
    b64_json: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    revised_prompt: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiEmbeddingResponse {
    model: String,
    data: Vec<OpenAiEmbedding>,
    usage: Option<OpenAiUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenAiEmbedding {
    embedding: Vec<f64>,
    index: u32,
}

#[derive(Debug, Deserialize, Serialize)]
struct OpenAiUsage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
    #[serde(default)]
    total_tokens: u64,
    #[serde(default)]
    prompt_tokens_details: Option<OpenAiPromptTokensDetails>,
}

#[derive(Debug, Deserialize, Serialize)]
struct OpenAiPromptTokensDetails {
    #[serde(default)]
    cached_tokens: u64,
    #[serde(default)]
    audio_tokens: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiGenerateContentResponse {
    #[serde(default)]
    candidates: Vec<GeminiCandidate>,
    #[serde(default)]
    usage_metadata: Option<GeminiUsageMetadata>,
    #[serde(default)]
    model_version: Option<String>,
}

impl GeminiGenerateContentResponse {
    fn first_text(&self) -> Option<String> {
        self.candidates
            .iter()
            .flat_map(|candidate| candidate.content.parts.iter())
            .find_map(|part| part.text.clone())
    }

    fn first_inline_image(&self) -> Option<GeminiInlineData> {
        self.candidates
            .iter()
            .flat_map(|candidate| candidate.content.parts.iter())
            .find_map(|part| part.inline_data.clone())
    }
}

#[derive(Debug, Deserialize)]
struct GeminiCandidate {
    content: GeminiContent,
}

#[derive(Debug, Deserialize)]
struct GeminiContent {
    #[serde(default)]
    parts: Vec<GeminiPart>,
}

#[derive(Clone, Debug, Deserialize)]
struct GeminiPart {
    #[serde(default)]
    text: Option<String>,
    #[serde(default, rename = "inlineData")]
    inline_data: Option<GeminiInlineData>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiInlineData {
    mime_type: String,
    data: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiUsageMetadata {
    #[serde(default)]
    prompt_token_count: u64,
    #[serde(default)]
    candidates_token_count: u64,
    #[serde(default)]
    total_token_count: u64,
}

#[derive(Debug, Deserialize)]
struct GeminiEmbeddingResponse {
    embedding: GeminiEmbedding,
}

#[derive(Debug, Deserialize)]
struct GeminiEmbedding {
    values: Vec<f64>,
}

#[derive(Debug, Deserialize)]
struct AnthropicMessageResponse {
    model: String,
    content: Vec<AnthropicContentPart>,
    usage: Option<AnthropicUsage>,
}

#[derive(Debug, Deserialize)]
struct AnthropicContentPart {
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct AnthropicUsage {
    input_tokens: u64,
    output_tokens: u64,
}

fn openai_messages(prompt: &str, system_prompt: Option<&str>) -> Vec<Value> {
    let mut messages = Vec::new();
    if let Some(system_prompt) = system_prompt.filter(|value| !value.trim().is_empty()) {
        messages.push(json!({"role": "system", "content": system_prompt}));
    }
    messages.push(json!({"role": "user", "content": prompt}));
    messages
}

fn google_generate_content_body(
    prompt: &str,
    system_prompt: Option<&str>,
    generation_config: Option<Value>,
) -> Value {
    let mut body = json!({
        "contents": [{"role": "user", "parts": [{"text": prompt}]}]
    });
    if let Some(system_prompt) = system_prompt.filter(|value| !value.trim().is_empty()) {
        body["systemInstruction"] = json!({"parts": [{"text": system_prompt}]});
    }
    if let Some(generation_config) = generation_config {
        body["generationConfig"] = generation_config;
    }
    body
}

fn google_generate_content_url(model: &str) -> String {
    format!(
        "https://generativelanguage.googleapis.com/v1beta/{}:generateContent",
        google_model_path(model)
    )
}

fn google_model_path(model: &str) -> String {
    let model = model.trim();
    if model.starts_with("models/") {
        model.to_owned()
    } else {
        format!("models/{model}")
    }
}

fn openai_usage(usage: OpenAiUsage) -> GenerationUsage {
    GenerationUsage {
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
        total_tokens: usage.total_tokens,
        cached_prompt_tokens: usage
            .prompt_tokens_details
            .as_ref()
            .map(|details| details.cached_tokens)
            .filter(|value| *value > 0),
        audio_prompt_tokens: usage
            .prompt_tokens_details
            .as_ref()
            .map(|details| details.audio_tokens)
            .filter(|value| *value > 0),
        provider_usage: serde_json::to_value(usage).ok(),
    }
}

fn gemini_usage(usage: GeminiUsageMetadata) -> GenerationUsage {
    GenerationUsage {
        prompt_tokens: usage.prompt_token_count,
        completion_tokens: usage.candidates_token_count,
        total_tokens: usage.total_token_count,
        cached_prompt_tokens: None,
        audio_prompt_tokens: None,
        provider_usage: serde_json::to_value(usage).ok(),
    }
}

fn anthropic_usage(usage: AnthropicUsage) -> GenerationUsage {
    let total_tokens = usage.input_tokens + usage.output_tokens;
    GenerationUsage {
        prompt_tokens: usage.input_tokens,
        completion_tokens: usage.output_tokens,
        total_tokens,
        cached_prompt_tokens: None,
        audio_prompt_tokens: None,
        provider_usage: serde_json::to_value(usage).ok(),
    }
}

fn estimate_cost(provider: &str, model: &str, usage: &GenerationUsage) -> Option<GenerationCost> {
    let model = model
        .trim()
        .strip_prefix("models/")
        .unwrap_or(model.trim())
        .to_ascii_lowercase();
    let pricing = match (provider, model.as_str()) {
        ("openai", "gpt-4o-mini") => Some((0.15, 0.60)),
        ("openai", "text-embedding-3-small") => Some((0.02, 0.0)),
        ("anthropic", "claude-3-5-haiku-20241022") => Some((0.80, 4.0)),
        _ => None,
    }?;
    let estimated_usd = (usage.prompt_tokens as f64 * pricing.0
        + usage.completion_tokens as f64 * pricing.1)
        / 1_000_000.0;
    Some(GenerationCost {
        estimated_usd,
        currency: "USD".to_owned(),
        source: "capsem-static-pricing-table".to_owned(),
    })
}

fn provider_error_message(value: &Value) -> String {
    value
        .pointer("/error/message")
        .or_else(|| value.pointer("/error"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| value.get("raw").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or_else(|| value.to_string())
}

fn parse_provider_value<T>(value: Value) -> Result<T>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_value(value).map_err(|error| GenerationError::Provider(error.to_string()))
}

fn unsupported(provider: &str, capability: &'static str) -> GenerationError {
    GenerationError::UnsupportedCapability {
        capability: provider_capability(provider, capability),
    }
}

fn fake_usage() -> GenerationUsage {
    GenerationUsage {
        prompt_tokens: 4,
        completion_tokens: 2,
        total_tokens: 6,
        cached_prompt_tokens: None,
        audio_prompt_tokens: None,
        provider_usage: Some(serde_json::json!({
            "prompt_tokens": 4,
            "completion_tokens": 2,
            "total_tokens": 6
        })),
    }
}

fn fake_cost() -> GenerationCost {
    GenerationCost {
        estimated_usd: 0.000001,
        currency: "USD".to_owned(),
        source: "fake-provider".to_owned(),
    }
}

#[derive(Debug, Default, Deserialize)]
struct ServiceSettingsToml {
    #[serde(default)]
    credentials: CredentialSettingsToml,
    #[serde(default)]
    ai: AiSettingsToml,
}

#[derive(Debug, Default, Deserialize)]
struct CredentialSettingsToml {
    #[serde(default)]
    items: BTreeMap<String, TomlCredential>,
}

#[derive(Debug, Default, Deserialize)]
struct AiSettingsToml {
    #[serde(default)]
    models: BTreeMap<String, ModelDefaultsToml>,
}

#[derive(Debug, Default, Deserialize)]
struct ModelDefaultsToml {
    text: Option<String>,
    image: Option<String>,
    embedding: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TomlCredential {
    value: String,
}

fn service_settings_path() -> Option<PathBuf> {
    if let Some(path) = env::var_os("CAPSEM_SERVICE_SETTINGS_PATH")
        .or_else(|| env::var_os("CAPSEM_SETTINGS_PATH"))
        .map(PathBuf::from)
    {
        return Some(path);
    }
    if let Some(home) = env::var_os("CAPSEM_HOME").map(PathBuf::from) {
        return Some(home.join("service.toml"));
    }
    env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".capsem").join("service.toml"))
}

fn resolve_credential_value(value: &String) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Some(env_name) = value.strip_prefix("env:") {
        return env::var(env_name.trim())
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
    }
    Some(value.to_owned())
}

fn credential_candidates(provider: &str) -> Vec<String> {
    match canonical_provider(provider).as_str() {
        "google" => [
            "google-api-key",
            "google.api_key",
            "google",
            "gemini-api-key",
            "gemini.api_key",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        "openai" => ["openai-api-key", "openai.api_key", "openai"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        "anthropic" => ["anthropic-api-key", "anthropic.api_key", "anthropic"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        other => [
            format!("{other}-api-key"),
            format!("{other}.api_key"),
            other.to_owned(),
        ]
        .into_iter()
        .collect(),
    }
}

fn provider_env_candidates(provider: &str) -> Vec<&'static str> {
    match canonical_provider(provider).as_str() {
        "google" => vec!["GEMINI_API_KEY", "GOOGLE_API_KEY", "CAPSEM_GEMINI_API_KEY"],
        "openai" => vec!["OPENAI_API_KEY"],
        "anthropic" => vec!["ANTHROPIC_API_KEY"],
        _ => Vec::new(),
    }
}

fn canonical_provider(provider: &str) -> String {
    match provider.trim().to_ascii_lowercase().as_str() {
        "gemini" | "google-ai" | "google_ai" => "google".to_owned(),
        other => other.to_owned(),
    }
}

fn require_non_empty(field: &'static str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        Err(GenerationError::Provider(format!(
            "{field} cannot be empty"
        )))
    } else {
        Ok(())
    }
}

fn non_empty(value: impl Into<String>) -> Option<String> {
    let value = value.into();
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn ensure_provider_supports(provider: &str, capability: &'static str) -> Result<()> {
    let supported = match capability {
        "text" => matches!(provider, "openai" | "anthropic" | "google"),
        "image" => matches!(provider, "openai" | "google"),
        "embedding" => matches!(provider, "openai" | "google"),
        _ => false,
    };
    if supported {
        Ok(())
    } else {
        Err(GenerationError::UnsupportedCapability {
            capability: provider_capability(provider, capability),
        })
    }
}

fn provider_capability(provider: &str, capability: &'static str) -> &'static str {
    match (provider, capability) {
        (_, "text") => "text generation for provider",
        (_, "image") => "image generation for provider",
        (_, "embedding") => "embedding generation for provider",
        _ => "generation provider capability",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_settings_resolves_capsem_google_key() {
        let settings = GenerationSettings::from_service_toml_str(
            r#"
            [credentials.items.google-api-key]
            value = "AIza-test"
            "#,
        )
        .unwrap();
        let credential = settings.resolve_api_key("gemini").unwrap();
        assert_eq!(credential.source, "settings:google-api-key");
        assert_eq!(credential.expose_for_provider_client(), "AIza-test");
    }

    #[test]
    fn service_settings_resolves_env_indirection() {
        env::set_var("CAPSEM_TEST_OPENAI_KEY", "sk-test");
        let settings = GenerationSettings::from_service_toml_str(
            r#"
            [credentials.items."openai.api_key"]
            value = "env:CAPSEM_TEST_OPENAI_KEY"
            "#,
        )
        .unwrap();
        let credential = settings.resolve_api_key("openai").unwrap();
        assert_eq!(credential.expose_for_provider_client(), "sk-test");
    }

    #[test]
    fn service_settings_resolves_model_policy() {
        let settings = GenerationSettings::from_service_toml_str(
            r#"
            [ai.models.google]
            text = "gemini-3.5-flash"
            image = "gemini-3.5-flash-image"
            embedding = "gemini-embedding-2"
            "#,
        )
        .unwrap();

        assert_eq!(
            settings.resolve_model("google", "text", None).unwrap(),
            "gemini-3.5-flash"
        );
        assert_eq!(
            settings
                .resolve_model("google", "image", Some("override-image".to_owned()))
                .unwrap(),
            "override-image"
        );
    }

    #[tokio::test]
    async fn fake_provider_generates_text_without_network() {
        let engine = GenerationEngine::new(
            GenerationSettings::empty()
                .with_credential("google-api-key", "AIza-test")
                .with_model("google", "text", "gemini-3.5-flash"),
            FakeModelProvider,
        );
        let response = engine
            .generate_text(GenerateTextRequest {
                provider: "gemini".to_owned(),
                model: None,
                system: Some("be terse".to_owned()),
                prompt: "hello".to_owned(),
            })
            .await
            .unwrap();
        assert_eq!(response.provider, "google");
        assert_eq!(response.text, "fake: hello");
        assert_eq!(response.usage.unwrap().total_tokens, 6);
        assert_eq!(response.cost.unwrap().source, "fake-provider");
    }

    #[tokio::test]
    async fn fake_provider_generates_embedding_without_network() {
        let engine = GenerationEngine::new(
            GenerationSettings::empty()
                .with_credential("google-api-key", "AIza-test")
                .with_model("google", "embedding", "gemini-embedding-2"),
            FakeModelProvider,
        );
        let response = engine
            .generate_embedding(GenerateEmbeddingRequest {
                provider: "gemini".to_owned(),
                model: None,
                input: vec!["hello".to_owned(), "world".to_owned()],
            })
            .await
            .unwrap();

        assert_eq!(response.provider, "google");
        assert_eq!(response.dimensions, 3);
        assert_eq!(response.vectors.len(), 2);
        assert_eq!(response.usage.unwrap().prompt_tokens, 4);
        assert_eq!(response.cost.unwrap().currency, "USD");
    }

    #[test]
    fn google_model_path_is_native_and_stable() {
        assert_eq!(
            google_model_path("gemini-3.5-flash-image"),
            "models/gemini-3.5-flash-image"
        );
        assert_eq!(
            google_model_path("models/gemini-embedding-2"),
            "models/gemini-embedding-2"
        );
    }

    #[test]
    fn parses_gemini_image_part_without_provider_crate() {
        let response: GeminiGenerateContentResponse = serde_json::from_value(json!({
            "modelVersion": "gemini-3.5-flash-image",
            "candidates": [{
                "content": {
                    "parts": [
                        {"text": "revised"},
                        {"inlineData": {"mimeType": "image/png", "data": "ZmFrZQ=="}}
                    ]
                }
            }],
            "usageMetadata": {
                "promptTokenCount": 7,
                "candidatesTokenCount": 3,
                "totalTokenCount": 10
            }
        }))
        .unwrap();

        assert_eq!(response.first_text().unwrap(), "revised");
        assert_eq!(response.first_inline_image().unwrap().data, "ZmFrZQ==");
        assert_eq!(
            gemini_usage(response.usage_metadata.unwrap()).total_tokens,
            10
        );
    }

    #[tokio::test]
    async fn missing_key_is_typed() {
        let engine = GenerationEngine::new(GenerationSettings::empty(), FakeModelProvider);
        let error = engine
            .generate_text(GenerateTextRequest {
                provider: "gemini".to_owned(),
                model: None,
                system: None,
                prompt: "hello".to_owned(),
            })
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            GenerationError::MissingCredential { provider, .. } if provider == "google"
        ));
    }

    #[tokio::test]
    async fn missing_model_is_typed() {
        let engine = GenerationEngine::new(
            GenerationSettings::empty().with_credential("google-api-key", "AIza-test"),
            FakeModelProvider,
        );
        let error = engine
            .generate_text(GenerateTextRequest {
                provider: "gemini".to_owned(),
                model: None,
                system: None,
                prompt: "hello".to_owned(),
            })
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            GenerationError::MissingModel {
                provider,
                capability: "text",
                ..
            } if provider == "google"
        ));
    }

    #[tokio::test]
    #[ignore = "live OpenAI image generation smoke; skips if OPENAI_API_KEY is absent"]
    async fn live_openai_image_generation_smoke() {
        let Some(settings) = live_settings_with_provider("openai") else {
            eprintln!("skipping live OpenAI image smoke: OPENAI_API_KEY not configured");
            return;
        };
        let engine = GenerationEngine::new(settings, CapsemHttpModelProvider::default());
        let output = engine
            .generate_image(GenerateImageRequest {
                provider: "openai".to_owned(),
                model: Some("gpt-image-1".to_owned()),
                prompt: "small clean icon of a golden security gate, no text".to_owned(),
            })
            .await
            .unwrap();

        assert!(output.data_url.is_some() || output.url.is_some());
    }

    #[tokio::test]
    #[ignore = "live Gemini Nano Banana image generation smoke; skips if Gemini credentials are absent"]
    async fn live_gemini_image_generation_smoke() {
        let Some(settings) = live_settings_with_provider("google") else {
            eprintln!("skipping live Gemini image smoke: Gemini credential not configured");
            return;
        };
        let engine = GenerationEngine::new(settings, CapsemHttpModelProvider::default());
        let output = engine
            .generate_image(GenerateImageRequest {
                provider: "gemini".to_owned(),
                model: Some("gemini-3.5-flash-image".to_owned()),
                prompt: "small clean icon of a golden security gate, no text".to_owned(),
            })
            .await
            .unwrap();

        assert!(output.data_url.is_some() || output.url.is_some());
    }

    #[tokio::test]
    #[ignore = "live embedding smoke; skips if OPENAI_API_KEY is absent"]
    async fn live_openai_embedding_smoke() {
        let Some(settings) = live_settings_with_provider("openai") else {
            eprintln!("skipping live OpenAI embedding smoke: OPENAI_API_KEY not configured");
            return;
        };
        let engine = GenerationEngine::new(settings, CapsemHttpModelProvider::default());
        let output = engine
            .generate_embedding(GenerateEmbeddingRequest {
                provider: "openai".to_owned(),
                model: Some("text-embedding-3-small".to_owned()),
                input: vec!["capsem accounting smoke".to_owned()],
            })
            .await
            .unwrap();

        assert_eq!(output.vectors.len(), 1);
        assert!(output.dimensions > 0);
        assert!(output.usage.is_some());
    }

    #[tokio::test]
    #[ignore = "live Gemini embedding smoke; skips if Gemini credentials are absent"]
    async fn live_gemini_embedding_smoke() {
        let Some(settings) = live_settings_with_provider("google") else {
            eprintln!("skipping live Gemini embedding smoke: Gemini credential not configured");
            return;
        };
        let engine = GenerationEngine::new(settings, CapsemHttpModelProvider::default());
        let output = engine
            .generate_embedding(GenerateEmbeddingRequest {
                provider: "gemini".to_owned(),
                model: Some("gemini-embedding-2".to_owned()),
                input: vec!["capsem accounting smoke".to_owned()],
            })
            .await
            .unwrap();

        assert_eq!(output.vectors.len(), 1);
        assert!(output.dimensions > 0);
    }

    fn live_settings_with_provider(provider: &str) -> Option<GenerationSettings> {
        let settings = GenerationSettings::from_capsem_environment().ok()?;
        settings.resolve_api_key(provider).ok()?;
        Some(settings)
    }
}
