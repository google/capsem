use super::*;
use capsem_proto::*;
use std::collections::BTreeSet;

fn require(valid: bool, message: &'static str) -> Result<(), String> {
    valid.then_some(()).ok_or_else(|| message.into())
}

pub(super) fn canonical_uuid(value: &str) -> bool {
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

impl<I, R, C> EnvironmentSpecification<I, R, C> {
    fn validate_common(&self) -> Result<(), String> {
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
        validate_values(&self.variables, &self.secrets)?;
        self.plugins.validate().map_err(str::to_owned)?;
        if let Some(startup) = &self.startup {
            require(self.image.is_some(), "environment startup requires an OCI image")?;
            require(
                startup.script.len() <= ENVIRONMENT_MAX_STARTUP_SCRIPT_BYTES
                    && text(&startup.script)
                    && startup.timeout_seconds > 0,
                "invalid environment startup script or timeout",
            )?;
        }
        require(
            self.repositories.len() <= ENVIRONMENT_MAX_REPOSITORIES,
            "environment repository count exceeds its limit",
        )?;
        require(
            self.repositories.is_empty() || self.image.is_some(),
            "environment repositories require an OCI image",
        )?;
        require(
            self.packages.registries.len() <= ENVIRONMENT_MAX_PACKAGE_REGISTRIES,
            "environment package registries exceed their count limit",
        )?;
        let mut registries = BTreeSet::new();
        for registry in &self.packages.registries {
            require(
                crate::validate_identifier_shape(registry).is_ok() && registries.insert(registry),
                "invalid or duplicate package registry selection",
            )?;
        }
        Ok(())
    }
}

fn validate_image_reference(reference: &str) -> Result<(), String> {
    require(
        !reference.is_empty() && !reference.chars().any(char::is_whitespace) && text(reference),
        "invalid environment image reference",
    )?;
    Ok(())
}

fn validate_repositories<'a>(
    repositories: impl IntoIterator<Item = (&'a str, &'a str, &'a str)>,
) -> Result<(), String> {
    let mut destinations: Vec<&str> = Vec::new();
    for (repository_url, reference, destination) in repositories {
        let url = url::Url::parse(repository_url).map_err(|_| "invalid environment repository URL")?;
        require(
            matches!(url.scheme(), "http" | "https" | "ssh")
                && url.host_str().is_some()
                && url.password().is_none()
                && (url.scheme() == "ssh" || url.username().is_empty())
                && url.query().is_none()
                && url.fragment().is_none(),
            "repository URLs must not embed credentials or tokens",
        )?;
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

impl EnvironmentSpecification {
    /// Validate host-resolved revision content. A selector alone is not a pin.
    pub fn validate(&self) -> Result<(), String> {
        self.validate_common()?;
        self.skills.validate()?;
        self.mcps.catalog.validate()?;
        mcps::validate(
            &self.mcps,
            self.mcps.catalog.selected.iter().map(|entry| entry.id.as_str()),
            self.image.is_some(),
        )?;
        self.access.catalog.validate()?;
        self.detection.catalog.validate()?;
        rules::access(
            &self.access,
            self.access.catalog.selected.iter().map(|entry| entry.id.as_str()),
        )?;
        rules::detection(
            &self.detection,
            self.detection.catalog.selected.iter().map(|entry| entry.id.as_str()),
        )?;
        if let Some(image) = &self.image {
            validate_image_reference(&image.reference)?;
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
        validate_repositories(self.repositories.iter().map(|repository| {
            (
                repository.url.as_str(),
                repository.reference.as_str(),
                repository.destination.as_str(),
            )
        }))?;
        for repository in &self.repositories {
            require(
                matches!(repository.resolved_commit.len(), 40 | 64)
                    && repository
                        .resolved_commit
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
                "repository requires an exact resolved commit",
            )?;
        }
        Ok(())
    }
}

impl EnvironmentDraft {
    /// Validate intent before resolution; the ledger admission bounds the final
    /// encoded proposal. Errors and Debug never expose variable/script values.
    pub fn validate(&self) -> Result<(), String> {
        self.validate_common()?;
        self.skills.validate()?;
        self.mcps.catalog.validate()?;
        mcps::validate(
            &self.mcps,
            self.mcps.catalog.selected.iter().map(|entry| entry.id.as_str()),
            self.image.is_some(),
        )?;
        self.access.catalog.validate()?;
        self.detection.catalog.validate()?;
        rules::access(
            &self.access,
            self.access.catalog.selected.iter().map(|entry| entry.id.as_str()),
        )?;
        rules::detection(
            &self.detection,
            self.detection.catalog.selected.iter().map(|entry| entry.id.as_str()),
        )?;
        if let Some(image) = &self.image {
            validate_image_reference(&image.reference)?;
        }
        validate_repositories(self.repositories.iter().map(|repository| {
            (
                repository.url.as_str(),
                repository.reference.as_str(),
                repository.destination.as_str(),
            )
        }))
    }
}

pub(super) fn validate_values(
    variables: &BTreeMap<String, String>,
    secrets: &BTreeMap<String, EnvironmentSecretBinding>,
) -> Result<(), String> {
    require(
        variables.len() <= ENVIRONMENT_MAX_VARIABLES && secrets.len() <= ENVIRONMENT_MAX_VARIABLES,
        "environment variable or binding count exceeds its limit",
    )?;
    let mut total = 0usize;
    let mut names = BTreeSet::new();
    for (name, value) in variables {
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
    for (alias, binding) in secrets {
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
    Ok(())
}
