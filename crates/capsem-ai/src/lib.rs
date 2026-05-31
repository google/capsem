use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GenerationError {
    #[error("missing provider credential for {provider}; checked {checked:?}")]
    MissingCredential {
        provider: String,
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
        Self { credentials }
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
    #[serde(default = "default_provider")]
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
    #[serde(default = "default_provider")]
    pub provider: String,
    #[serde(default)]
    pub model: Option<String>,
    pub prompt: String,
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

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextGeneration {
    pub provider: String,
    pub model: String,
    pub text: String,
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
}

#[async_trait]
pub trait ModelProvider: Send + Sync {
    async fn generate_text(&self, request: ResolvedTextRequest) -> Result<TextGeneration>;
    async fn generate_image(&self, request: ResolvedImageRequest) -> Result<ImageGeneration>;
}

#[derive(Clone, Debug)]
pub struct GenerationEngine<P = SiumaiModelProvider> {
    settings: GenerationSettings,
    provider: P,
}

impl GenerationEngine<SiumaiModelProvider> {
    pub fn from_capsem_environment() -> Result<Self> {
        Ok(Self::new(
            GenerationSettings::from_capsem_environment()?,
            SiumaiModelProvider,
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
                model: request
                    .model
                    .unwrap_or_else(|| default_text_model(&provider).to_owned()),
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
                model: request
                    .model
                    .unwrap_or_else(|| default_image_model(&provider).to_owned()),
                provider,
                prompt: request.prompt,
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
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct SiumaiModelProvider;

#[async_trait]
impl ModelProvider for SiumaiModelProvider {
    async fn generate_text(&self, request: ResolvedTextRequest) -> Result<TextGeneration> {
        use siumai::prelude::unified::*;

        let key = request.credential.expose_for_provider_client();
        let chat_request = chat_request(&request.prompt, request.system.as_deref());
        let text = match request.provider.as_str() {
            "openai" => {
                let cfg =
                    siumai::providers::openai::OpenAiConfig::new(key).with_model(&request.model);
                let client = siumai::providers::openai::OpenAiClient::from_config(cfg)
                    .map_err(|error| GenerationError::Provider(error.to_string()))?;
                let response =
                    text::generate(&client, chat_request, text::GenerateOptions::default())
                        .await
                        .map_err(|error| GenerationError::Provider(error.to_string()))?;
                response.content_text().unwrap_or_default().to_owned()
            }
            "anthropic" => {
                let cfg = siumai::providers::anthropic::AnthropicConfig::new(key)
                    .with_model(&request.model);
                let client = siumai::providers::anthropic::AnthropicClient::from_config(cfg)
                    .map_err(|error| GenerationError::Provider(error.to_string()))?;
                let response =
                    text::generate(&client, chat_request, text::GenerateOptions::default())
                        .await
                        .map_err(|error| GenerationError::Provider(error.to_string()))?;
                response.content_text().unwrap_or_default().to_owned()
            }
            "google" => {
                let cfg = siumai::providers::gemini::GeminiConfig::new(key)
                    .with_model(request.model.clone());
                let client = siumai::providers::gemini::GeminiClient::from_config(cfg)
                    .map_err(|error| GenerationError::Provider(error.to_string()))?;
                let response =
                    text::generate(&client, chat_request, text::GenerateOptions::default())
                        .await
                        .map_err(|error| GenerationError::Provider(error.to_string()))?;
                response.content_text().unwrap_or_default().to_owned()
            }
            other => {
                return Err(GenerationError::UnsupportedCapability {
                    capability: provider_capability(other, "text"),
                })
            }
        };
        Ok(TextGeneration {
            provider: request.provider,
            model: request.model,
            text,
        })
    }

    async fn generate_image(&self, request: ResolvedImageRequest) -> Result<ImageGeneration> {
        use siumai::prelude::unified::*;

        let key = request.credential.expose_for_provider_client();
        match request.provider.as_str() {
            "openai" => {
                let cfg =
                    siumai::providers::openai::OpenAiConfig::new(key).with_model(&request.model);
                let client = siumai::providers::openai::OpenAiClient::from_config(cfg)
                    .map_err(|error| GenerationError::Provider(error.to_string()))?;
                let response = image::generate(
                    &client,
                    ImageGenerationRequest {
                        prompt: request.prompt,
                        count: 1,
                        model: Some(request.model.clone()),
                        ..Default::default()
                    },
                    image::GenerateOptions::default(),
                )
                .await
                .map_err(|error| GenerationError::Provider(error.to_string()))?;
                image_generation_from_response(request.provider, request.model, response)
            }
            "google" => {
                let cfg = siumai::providers::gemini::GeminiConfig::new(key)
                    .with_model(request.model.clone());
                let client = siumai::providers::gemini::GeminiClient::from_config(cfg)
                    .map_err(|error| GenerationError::Provider(error.to_string()))?;
                let response = image::generate_image(
                    &client,
                    GenerateImageRequest::new(request.prompt),
                    image::GenerateOptions::default(),
                )
                .await
                .map_err(|error| GenerationError::Provider(error.to_string()))?;
                image_generation_from_response(request.provider, request.model, response)
            }
            other => Err(GenerationError::UnsupportedCapability {
                capability: provider_capability(other, "image"),
            }),
        }
    }
}

fn chat_request(prompt: &str, system_prompt: Option<&str>) -> siumai::prelude::ChatRequest {
    use siumai::prelude::unified::*;
    let mut builder = ChatRequest::builder();
    if let Some(system_prompt) = system_prompt.filter(|value| !value.trim().is_empty()) {
        builder = builder.message(system!(system_prompt.to_owned()));
    }
    builder.message(user!(prompt.to_owned())).build()
}

fn image_generation_from_response(
    provider: String,
    model: String,
    response: siumai::prelude::ImageGenerationResponse,
) -> Result<ImageGeneration> {
    let Some(first) = response.images.first() else {
        return Err(GenerationError::Provider(
            "provider response did not include an image".to_owned(),
        ));
    };
    let data_url = first.b64_json.as_ref().map(|b64| {
        let mime = first.format.as_deref().unwrap_or("image/png");
        format!("data:{mime};base64,{b64}")
    });
    Ok(ImageGeneration {
        provider,
        model,
        mime_type: first.format.clone(),
        data_url,
        url: first.url.clone(),
        revised_prompt: first.revised_prompt.clone(),
    })
}

#[derive(Debug, Default, Deserialize)]
struct ServiceSettingsToml {
    #[serde(default)]
    credentials: CredentialSettingsToml,
}

#[derive(Debug, Default, Deserialize)]
struct CredentialSettingsToml {
    #[serde(default)]
    items: BTreeMap<String, TomlCredential>,
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

fn default_provider() -> String {
    "google".to_owned()
}

fn default_text_model(provider: &str) -> &'static str {
    match provider {
        "openai" => "gpt-4o-mini",
        "anthropic" => "claude-3-5-haiku-20241022",
        "google" => "gemini-2.0-flash-exp",
        _ => "default",
    }
}

fn default_image_model(provider: &str) -> &'static str {
    match provider {
        "openai" => "gpt-image-1",
        "google" => "gemini-2.5-flash-image",
        _ => "default",
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

fn provider_capability(provider: &str, capability: &'static str) -> &'static str {
    match (provider, capability) {
        (_, "text") => "text generation for provider",
        (_, "image") => "image generation for provider",
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

    #[tokio::test]
    async fn fake_provider_generates_text_without_network() {
        let engine = GenerationEngine::new(
            GenerationSettings::empty().with_credential("google-api-key", "AIza-test"),
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
}
