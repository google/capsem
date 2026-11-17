use super::*;
use capsem_proto::*;
use std::collections::BTreeSet;

fn require(valid: bool, message: &'static str) -> Result<(), String> {
    valid.then_some(()).ok_or_else(|| message.into())
}

fn canonical_uuid(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|id| {
        (1..=8).contains(&id.get_version_num()) && id.get_variant() == uuid::Variant::RFC4122 && id.to_string() == value
    })
}

fn text(value: &str) -> bool {
    !value
        .chars()
        .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
}

fn variable_name(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes
        .first()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
        && bytes.iter().all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
        && bytes.len() <= ENVIRONMENT_MAX_VARIABLE_NAME_BYTES
        && validate_env_key(value).is_ok()
}

impl EnvironmentDocument {
    pub(super) fn validate_fields(&self) -> Result<(), String> {
        let identity = &self.identity;
        require(identity.schema_version == 1, "unsupported environment schema version")?;
        require(
            canonical_uuid(&identity.id) && canonical_uuid(&identity.revision_id),
            "invalid environment or revision identity",
        )?;
        match (&identity.parent_environment_id, &identity.parent_revision_id) {
            (None, None) => {}
            (Some(parent), Some(revision)) => require(
                canonical_uuid(parent) && canonical_uuid(revision) && parent != &identity.id,
                "invalid environment fork lineage",
            )?,
            _ => return Err("environment fork lineage requires both parent identities".into()),
        }
        self.specification.validate()
    }
}

impl EnvironmentSpecification {
    /// Validate editable values without accepting service-owned revision fields.
    /// No failure message includes ordinary variable values or script content.
    pub fn validate(&self) -> Result<(), String> {
        let name = self.name.as_bytes();
        require(
            !name.is_empty()
                && name.len() <= ENVIRONMENT_MAX_NAME_BYTES
                && name[0].is_ascii_alphanumeric()
                && name
                    .iter()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'_' | b'-')),
            "environment name requires a safe ASCII name within its byte limit",
        )?;
        require(
            self.description.len() <= ENVIRONMENT_MAX_DESCRIPTION_BYTES && text(&self.description),
            "invalid environment description",
        )?;
        require(
            self.variables.len() <= ENVIRONMENT_MAX_VARIABLES && self.secrets.len() <= ENVIRONMENT_MAX_VARIABLES,
            "environment variable or binding count exceeds its limit",
        )?;
        let mut total = 0usize;
        let mut names = BTreeSet::new();
        for (name, value) in &self.variables {
            require(variable_name(name), "invalid environment variable name")?;
            require(
                value.len() <= ENVIRONMENT_MAX_VARIABLE_VALUE_BYTES && validate_env_value(value).is_ok(),
                "invalid environment variable value",
            )?;
            total = total
                .checked_add(name.len())
                .and_then(|total| total.checked_add(value.len()))
                .ok_or("environment variable aggregate overflow")?;
            require(
                total <= ENVIRONMENT_MAX_VARIABLE_TOTAL_BYTES,
                "environment variables exceed their aggregate byte limit",
            )?;
            names.insert(name.as_str());
        }
        for (alias, binding) in &self.secrets {
            require(
                crate::validate_identifier_shape(alias).is_ok() && canonical_uuid(&binding.binding_id),
                "invalid environment binding reference",
            )?;
            if let Some(name) = &binding.env_name {
                require(
                    variable_name(name) && names.insert(name.as_str()),
                    "invalid or colliding environment binding variable name",
                )?;
            }
        }
        require(
            names.len() <= ENVIRONMENT_MAX_VARIABLES,
            "injected environment variables exceed their count limit",
        )?;
        if let Some(image) = &self.image {
            require(
                !image.reference.is_empty()
                    && !image.reference.chars().any(char::is_whitespace)
                    && text(&image.reference),
                "invalid environment image reference",
            )?;
            require(
                image.resolved_digest.strip_prefix("sha256:").is_some_and(|hex| {
                    hex.len() == 64
                        && hex
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                }),
                "environment image requires an exact SHA-256 digest",
            )?;
        }
        if let Some(startup) = &self.startup {
            require(self.image.is_some(), "environment startup requires an OCI image")?;
            require(
                startup.script.len() <= ENVIRONMENT_MAX_STARTUP_SCRIPT_BYTES
                    && text(&startup.script)
                    && startup.timeout_seconds > 0,
                "invalid environment startup script or timeout",
            )?;
        }
        self.validate_repositories()?;
        let mut registries = BTreeSet::new();
        for registry in &self.packages.registries {
            require(
                crate::validate_identifier_shape(registry).is_ok() && registries.insert(registry),
                "invalid or duplicate package registry selection",
            )?;
        }
        Ok(())
    }

    fn validate_repositories(&self) -> Result<(), String> {
        require(
            self.repositories.len() <= ENVIRONMENT_MAX_REPOSITORIES,
            "environment repository count exceeds its limit",
        )?;
        require(
            self.repositories.is_empty() || self.image.is_some(),
            "environment repositories require an OCI image",
        )?;
        let mut destinations: Vec<&str> = Vec::new();
        for repository in &self.repositories {
            let url = url::Url::parse(&repository.url).map_err(|_| "invalid environment repository URL")?;
            require(
                matches!(url.scheme(), "http" | "https" | "ssh")
                    && url.host_str().is_some()
                    && url.password().is_none()
                    && (url.scheme() == "ssh" || url.username().is_empty())
                    && url.query().is_none()
                    && url.fragment().is_none(),
                "repository URLs must not embed credentials or tokens",
            )?;
            let reference = &repository.reference;
            require(
                !reference.is_empty()
                    && !reference.starts_with('-')
                    && !reference.ends_with('/')
                    && !reference.ends_with('.')
                    && !reference.contains("..")
                    && !reference.contains("@{")
                    && !reference.contains("//")
                    && !reference.chars().any(|ch| {
                        ch.is_whitespace() || ch.is_control() || matches!(ch, '~' | '^' | ':' | '?' | '*' | '[' | '\\')
                    }),
                "invalid environment repository ref",
            )?;
            require(
                matches!(repository.resolved_commit.len(), 40 | 64)
                    && repository
                        .resolved_commit
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
                "repository requires an exact resolved commit",
            )?;
            let destination = repository.destination.as_str();
            require(
                !destination.contains('\\')
                    && !destination.chars().any(char::is_control)
                    && destination.split('/').all(|part| !matches!(part, "" | "." | "..")),
                "repository destination must be a normalized relative path",
            )?;
            require(
                !destinations.iter().any(|other| {
                    destination == *other
                        || destination
                            .strip_prefix(other)
                            .is_some_and(|suffix| suffix.starts_with('/'))
                        || other
                            .strip_prefix(destination)
                            .is_some_and(|suffix| suffix.starts_with('/'))
                }),
                "environment repository destinations overlap",
            )?;
            destinations.push(destination);
        }
        Ok(())
    }
}
