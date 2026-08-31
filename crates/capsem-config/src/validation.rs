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
        Err(IdentifierError::InvalidCharacters) => Err(format!(
            "{kind} must use only lowercase a-z, 0-9, '_' or '-': {value}"
        )),
    }
}

#[doc(hidden)]
pub fn validate_profile_target(kind: &str, value: &str) -> Result<(), String> {
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
