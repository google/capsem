#[doc(hidden)]
pub fn validate_non_empty(kind: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{kind} must not be empty"))
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[doc(hidden)]
pub enum IdentifierError {
    Empty,
    TooLong,
    InvalidCharacters,
}

#[doc(hidden)]
pub fn validate_identifier_shape(value: &str) -> Result<(), IdentifierError> {
    if value.trim().is_empty() {
        return Err(IdentifierError::Empty);
    }
    if value.len() > 64 {
        return Err(IdentifierError::TooLong);
    }
    if value
        .chars()
        .all(|ch| ch == '_' || ch == '-' || ch.is_ascii_lowercase() || ch.is_ascii_digit())
    {
        Ok(())
    } else {
        Err(IdentifierError::InvalidCharacters)
    }
}

#[doc(hidden)]
pub fn validate_identifier(kind: &str, value: &str) -> Result<(), String> {
    match validate_identifier_shape(value) {
        Ok(()) => Ok(()),
        Err(IdentifierError::Empty) => Err(format!("{kind} must not be empty")),
        Err(IdentifierError::TooLong) => Err(format!("{kind} must be at most 64 characters")),
        Err(IdentifierError::InvalidCharacters) => {
            Err(format!("{kind} must use only lowercase a-z, 0-9, '_' or '-': {value}"))
        }
    }
}

#[doc(hidden)]
pub fn validate_policy_target(kind: &str, value: &str) -> Result<(), String> {
    validate_non_empty(kind, value)?;
    if value.len() > 128 {
        return Err(format!("{kind} must be at most 128 characters"));
    }
    if value.contains("..") || value.contains('\\') || value.trim() != value {
        return Err(format!("{kind} must not contain traversal or padding"));
    }
    Ok(())
}

#[cfg(test)]
mod tests;

/// Whether `value` is a value `def` can hold: its type, and its range or
/// choices when the definition declares them. Settings files are written by
/// users and by `/settings/edit`, so a wrong-typed value is refused when it
/// is written rather than discovered when something reads it.
pub fn validate_setting_value_shape(
    def: &crate::types::SettingDef,
    value: &crate::types::SettingValue,
) -> Result<(), String> {
    use crate::types::{SettingType as T, SettingValue as V};
    let id = &def.id;
    let fits = match (def.setting_type, value) {
        (T::Bool, V::Bool(_)) | (T::Number, V::Number(_)) => true,
        (T::Text | T::Url | T::Email | T::ApiKey, V::Text(_)) => true,
        (T::StringList, V::StringList(_)) | (T::IntList, V::IntList(_)) | (T::FloatList, V::FloatList(_)) => true,
        (T::KvMap, V::KvMap(_)) | (T::File, V::File { .. }) => true,
        // An McpTool setting is discovered, never written by a user.
        _ => false,
    };
    if !fits {
        return Err(format!("{id}: a {:?} setting cannot hold {value:?}", def.setting_type));
    }
    if let V::Number(n) = value {
        if let Some(min) = def.metadata.min.filter(|min| n < min) {
            return Err(format!("{id}: {n} is below the minimum {min}"));
        }
        if let Some(max) = def.metadata.max.filter(|max| n > max) {
            return Err(format!("{id}: {n} exceeds the maximum {max}"));
        }
    }
    if let V::Text(text) = value {
        if !def.metadata.choices.is_empty() && !def.metadata.choices.contains(text) {
            return Err(format!("{id}: {text:?} is not one of {:?}", def.metadata.choices));
        }
    }
    Ok(())
}
