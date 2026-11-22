//! Refresh admission once, then compile the exact retained rule definitions.
//! This is launch/mutation work, never a discovery or status polling path.
use super::*;
use capsem_core::{
    environment::{EnvironmentPolicyInputs, RuleCatalogContents, RuleCatalogInputs},
    net::policy_config::ActivePolicyFile,
};

pub(crate) async fn launch_policy(
    state: &ServiceState,
    inputs: &EnvironmentPolicyInputs<'_>,
) -> Result<ActivePolicyFile, Error> {
    let pins = inputs
        .access_rules()
        .catalog
        .selected
        .iter()
        .chain(&inputs.detection_rules().catalog.selected)
        .collect::<Vec<_>>();
    if pins.is_empty() {
        return ActivePolicyFile::from_environment(inputs, None).map_err(|error| {
            tracing::warn!(%error, "could not compile saved environment policy");
            Error::InvalidIntent
        });
    }
    let kind = CatalogKind::Rules;
    let selected = source(state, kind).await?;
    // Reject a replaced source or a known revocation before reaching any
    // registry. Each admitted pin is checked again after content retrieval.
    for pin in &pins {
        require_current(state, kind, pin).await?;
        if pin.catalog_reference != selected.reference().to_string() {
            return Err(Error::Refused);
        }
    }
    let workspace = workspace_for_refresh(state, &selected).await?;
    let current = refresh::<RuleCatalogEntry>(state, &selected, &workspace)
        .await?
        .ok_or(Error::Unavailable)?;
    let mut retained: Vec<RuleCatalogContents> = Vec::new();
    for pin in pins {
        require_current(state, kind, pin).await?;
        if current.digest().as_str() == pin.resolved_digest {
            current.entry_for_pin(pin).map_err(content_error)?;
        } else if let Some(contents) = retained
            .iter()
            .find(|contents| contents.digest().as_str() == pin.resolved_digest)
        {
            contents.entry_for_pin(pin).map_err(content_error)?;
        } else {
            // The moving current tag establishes admission, never the chosen
            // rule body. Missing retained bytes fail without a network fallback.
            retained.push(
                RuleCatalogContents::fetch_cached(&workspace.puller, &selected, pin, &workspace.parent)
                    .await
                    .map_err(content_error)?,
            );
        }
        require_current(state, kind, pin).await?;
    }
    let pinned = retained.iter().collect::<Vec<_>>();
    let catalogs = RuleCatalogInputs::new(&[&current], &pinned).map_err(|_| Error::InvalidPin)?;
    ActivePolicyFile::from_environment(inputs, Some(&catalogs)).map_err(|error| {
        tracing::warn!(%error, "could not compile saved environment policy with admitted catalogs");
        Error::InvalidPin
    })
}
