//! Compatibility exports for configuration validation used by profile I/O.

pub(crate) use capsem_config::{
    validate_identifier_shape, validate_non_empty, validate_profile_target, IdentifierError,
};
