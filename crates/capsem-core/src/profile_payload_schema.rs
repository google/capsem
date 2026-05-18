use serde_json::Value;
use thiserror::Error;

pub const PROFILE_PAYLOAD_V2_SCHEMA_JSON: &str =
    include_str!("../../../schemas/capsem.profile.v2.schema.json");

#[derive(Debug, Error)]
pub enum ProfilePayloadSchemaError {
    #[error("failed to parse profile payload JSON: {0}")]
    ParseJson(#[from] serde_json::Error),
    #[error("failed to parse profile payload TOML: {0}")]
    ParseToml(#[from] toml::de::Error),
    #[error("failed to convert profile payload TOML to JSON-compatible data: {0}")]
    TomlBridge(serde_json::Error),
    #[error("profile payload schema artifact is invalid: {0}")]
    Compile(String),
    #[error("profile payload failed schema validation: {0}")]
    Validation(String),
}

pub type Result<T> = std::result::Result<T, ProfilePayloadSchemaError>;

pub fn validate_profile_payload_v2_json(input: &str) -> Result<Value> {
    let value = serde_json::from_str::<Value>(input)?;
    validate_profile_payload_v2_value(value)
}

pub fn validate_profile_payload_v2_toml(input: &str) -> Result<Value> {
    let value = toml::from_str::<toml::Value>(input)?;
    let value = serde_json::to_value(value).map_err(ProfilePayloadSchemaError::TomlBridge)?;
    validate_profile_payload_v2_value(value)
}

pub fn validate_profile_payload_v2_value(value: Value) -> Result<Value> {
    let schema = serde_json::from_str::<Value>(PROFILE_PAYLOAD_V2_SCHEMA_JSON)?;
    let validator = jsonschema::validator_for(&schema)
        .map_err(|error| ProfilePayloadSchemaError::Compile(error.to_string()))?;
    let errors = validator
        .iter_errors(&value)
        .map(|error| error.to_string())
        .collect::<Vec<_>>();
    if errors.is_empty() {
        Ok(value)
    } else {
        Err(ProfilePayloadSchemaError::Validation(errors.join("; ")))
    }
}
