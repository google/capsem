//! Current admission observations and immutable content are separate inputs.
use super::RuleCatalogContents;
use capsem_config::{
    catalogs::{CatalogEntry, ResolvedCatalogRule},
    environment::{EnvironmentCatalogPin, EnvironmentRuleParameter},
};
use std::collections::BTreeMap;

/// Borrow verified snapshots without copying catalog payloads or storing a
/// ledger projection. The registry owner supplies current observations only
/// after freshness, persistent known-revocation and source-policy admission.
/// These compilation inputs cannot establish that authority themselves.
pub struct RuleCatalogInputs<'a> {
    current: BTreeMap<String, &'a RuleCatalogContents>,
    pinned: BTreeMap<(String, String), &'a RuleCatalogContents>,
}
impl<'a> RuleCatalogInputs<'a> {
    pub fn new(current: &[&'a RuleCatalogContents], pinned: &[&'a RuleCatalogContents]) -> Result<Self, String> {
        // Access and detection can each select this many independent catalogs.
        let maximum = capsem_proto::ENVIRONMENT_MAX_CATALOG_SELECTIONS * 2;
        if current.len() > maximum || pinned.len() > maximum {
            return Err("too many rule catalog snapshots".into());
        }
        let mut inputs = Self {
            current: BTreeMap::new(),
            pinned: BTreeMap::new(),
        };
        for catalog in current {
            let reference = catalog.source().reference().to_string();
            if inputs.current.insert(reference.clone(), catalog).is_some() {
                return Err("ambiguous current rule catalog source".into());
            }
            inputs.pinned.insert((reference, catalog.digest().to_string()), catalog);
        }
        for catalog in pinned {
            inputs.pinned.insert(
                (catalog.source().reference().to_string(), catalog.digest().to_string()),
                catalog,
            );
        }
        Ok(inputs)
    }

    pub(super) fn resolve(
        &self,
        pin: &EnvironmentCatalogPin,
        values: &BTreeMap<String, EnvironmentRuleParameter>,
    ) -> Result<ResolvedCatalogRule, String> {
        let current = self
            .current
            .get(&pin.catalog_reference)
            .ok_or("selected rule catalog source is unavailable")?;
        let current_entry = current
            .entries()
            .iter()
            .find(|entry| entry.metadata().id == pin.id && entry.metadata().version == pin.version)
            .ok_or("selected rule catalog entry is not currently admitted")?;
        if !current_entry.metadata().is_selectable() {
            return Err("selected rule catalog entry is revoked or incompatible".into());
        }
        let catalog = self
            .pinned
            .get(&(pin.catalog_reference.clone(), pin.resolved_digest.clone()))
            .ok_or("pinned rule catalog content is unavailable")?;
        catalog
            .entry_for_pin(pin)
            .map_err(|error| error.to_string())?
            .resolve(values)
            .map_err(String::from)
    }
}
