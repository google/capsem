//! Resolve editable intent once before review; unchanged selectors retain their pins.
use capsem_config::{catalogs::CatalogKind, environment::*};
use std::future::Future;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvironmentResolutionError {
    InvalidIntent,
    InvalidPin,
    Unavailable,
    Refused,
    UnusableCatalogEntry,
}
impl std::fmt::Display for EnvironmentResolutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidIntent => "invalid environment intent",
            Self::InvalidPin => "environment source returned an invalid pin",
            Self::Unavailable => "environment resolution source unavailable",
            Self::Refused => "environment source refused by current policy",
            Self::UnusableCatalogEntry => "environment catalog entry is no longer usable",
        })
    }
}
impl std::error::Error for EnvironmentResolutionError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvironmentResolutionMode {
    Edit,
    RefreshImage,
}

/// Host-owned source capabilities, never supplied by an API caller. Implementors
/// verify fetched content and enforce their transport deadlines. Catalog admission
/// must use current corporate source restrictions and known revocations, including
/// for retained pins; resolving a moving tag again is not admission of an old pin.
pub trait EnvironmentPinResolver: Sync {
    fn image(&self, reference: &str) -> impl Future<Output = Result<String, EnvironmentResolutionError>> + Send;
    fn repository(
        &self,
        url: &str,
        reference: &str,
    ) -> impl Future<Output = Result<String, EnvironmentResolutionError>> + Send;
    fn catalog(
        &self,
        kind: CatalogKind,
        selection: &EnvironmentCatalogSelection,
    ) -> impl Future<Output = Result<EnvironmentCatalogPin, EnvironmentResolutionError>> + Send;
    fn admit_catalog(
        &self,
        kind: CatalogKind,
        pin: &EnvironmentCatalogPin,
    ) -> impl Future<Output = Result<(), EnvironmentResolutionError>> + Send;
}

/// This produces content for review, not caller authority or permission to apply.
/// Large variable/script/rule buffers move into the result without re-encoding.
pub async fn resolve_environment(
    draft: EnvironmentDraft,
    base: Option<&EnvironmentSpecification>,
    mode: EnvironmentResolutionMode,
    sources: &impl EnvironmentPinResolver,
) -> Result<EnvironmentSpecification, EnvironmentResolutionError> {
    draft
        .validate()
        .map_err(|_| EnvironmentResolutionError::InvalidIntent)?;
    if let Some(base) = base {
        base.validate().map_err(|_| EnvironmentResolutionError::InvalidPin)?;
    }
    let EnvironmentSpecification {
        name,
        description,
        image,
        startup,
        variables,
        secrets,
        repositories,
        packages,
        skills,
        access,
        detection,
        mcps,
        plugins,
    } = draft;
    let image = if let Some(selection) = image {
        let retained = base
            .and_then(|base| base.image.as_ref())
            .filter(|pin| mode == EnvironmentResolutionMode::Edit && pin.reference == selection.reference);
        let resolved_digest = if let Some(pin) = retained {
            pin.resolved_digest.clone()
        } else {
            sources.image(&selection.reference).await?
        };
        Some(EnvironmentImage {
            reference: selection.reference,
            resolved_digest,
            update_policy: selection.update_policy,
        })
    } else {
        None
    };
    let mut resolved_repositories = Vec::with_capacity(repositories.len());
    for selection in repositories {
        let retained = base.and_then(|base| {
            base.repositories
                .iter()
                .find(|pin| pin.url == selection.url && pin.reference == selection.reference)
        });
        let resolved_commit = if let Some(pin) = retained {
            pin.resolved_commit.clone()
        } else {
            sources.repository(&selection.url, &selection.reference).await?
        };
        resolved_repositories.push(EnvironmentRepository {
            url: selection.url,
            reference: selection.reference,
            destination: selection.destination,
            resolved_commit,
        });
    }
    let skills = selections(skills, base.map(|base| &base.skills), CatalogKind::Skills, sources).await?;
    let access_catalog = selections(
        access.catalog,
        base.map(|base| &base.access.catalog),
        CatalogKind::Rules,
        sources,
    )
    .await?;
    let detection_catalog = selections(
        detection.catalog,
        base.map(|base| &base.detection.catalog),
        CatalogKind::Rules,
        sources,
    )
    .await?;
    let mcp_catalog = selections(
        mcps.catalog,
        base.map(|base| &base.mcps.catalog),
        CatalogKind::Mcp,
        sources,
    )
    .await?;
    let result = EnvironmentSpecification {
        name,
        description,
        image,
        startup,
        variables,
        secrets,
        repositories: resolved_repositories,
        packages,
        skills,
        access: EnvironmentRules {
            catalog: access_catalog,
            parameters: access.parameters,
            custom: access.custom,
        },
        detection: EnvironmentRules {
            catalog: detection_catalog,
            parameters: detection.parameters,
            custom: detection.custom,
        },
        mcps: EnvironmentMcps {
            catalog: mcp_catalog,
            servers: mcps.servers,
            permissions: mcps.permissions,
        },
        plugins,
    };
    result.validate().map_err(|_| EnvironmentResolutionError::InvalidPin)?;
    Ok(result)
}

async fn selections(
    selections: EnvironmentCatalogSelections<EnvironmentCatalogSelection>,
    base: Option<&EnvironmentCatalogSelections<EnvironmentCatalogPin>>,
    kind: CatalogKind,
    sources: &impl EnvironmentPinResolver,
) -> Result<EnvironmentCatalogSelections<EnvironmentCatalogPin>, EnvironmentResolutionError> {
    let mut selected = Vec::with_capacity(selections.selected.len());
    for selection in selections.selected {
        let retained = base.and_then(|base| {
            base.selected
                .iter()
                .find(|pin| pin.id == selection.id && pin.version == selection.version)
        });
        let pin = if let Some(pin) = retained {
            pin.clone()
        } else {
            let pin = sources.catalog(kind, &selection).await?;
            if pin.id != selection.id || pin.version != selection.version {
                return Err(EnvironmentResolutionError::InvalidPin);
            }
            pin
        };
        sources.admit_catalog(kind, &pin).await?;
        selected.push(pin);
    }
    Ok(EnvironmentCatalogSelections { selected })
}

#[cfg(test)]
mod tests;
