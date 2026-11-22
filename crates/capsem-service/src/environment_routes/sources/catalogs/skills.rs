//! Selected skills use the catalog's current admission and retained exact bytes.
use super::*;
use capsem_core::environment::SkillCatalogContents;
use std::path::PathBuf;

pub(in crate::environment_routes) struct PinnedSkills {
    source: ValidatedCatalogSource,
    current: Option<SkillCatalogContents>,
    retained: Option<SkillCatalogContents>,
    puller: capsem_assets::oci::Puller,
    parent: PathBuf,
}

impl PinnedSkills {
    pub(in crate::environment_routes) async fn open(
        state: &ServiceState,
        pins: &[&EnvironmentCatalogPin],
    ) -> Result<Self, Error> {
        let kind = CatalogKind::Skills;
        let selected = source(state, kind).await?;
        let workspace = workspace_for_refresh(state, &selected).await?;
        let mut current = None;
        for pin in pins {
            if selected.reference().to_string() != pin.catalog_reference {
                continue;
            }
            match require_current(state, kind, pin).await {
                Ok(_) => {}
                Err(Error::UnusableCatalogEntry) => continue,
                Err(error) => return Err(error),
            }
            // Refresh is explicit. A failed refresh can still serve an exact
            // verified retained digest under current source and known revocations.
            current = refresh(state, &selected, &workspace).await?;
            break;
        }
        Ok(Self {
            source: selected,
            current,
            retained: None,
            puller: workspace.puller,
            parent: workspace.parent,
        })
    }

    pub(in crate::environment_routes) fn discovery_fresh(&self) -> bool {
        self.current.is_some()
    }

    pub(in crate::environment_routes) async fn entry(
        &mut self,
        state: &ServiceState,
        pin: &EnvironmentCatalogPin,
    ) -> Result<&SkillCatalogEntry, Error> {
        let kind = CatalogKind::Skills;
        require_current(state, kind, pin).await?;
        let contents = if let Some(current) = self
            .current
            .as_ref()
            .filter(|contents| contents.digest().as_str() == pin.resolved_digest)
        {
            current
        } else {
            if self
                .retained
                .as_ref()
                .is_none_or(|contents| contents.digest().as_str() != pin.resolved_digest)
            {
                self.retained = Some(
                    SkillCatalogContents::fetch_cached(&self.puller, &self.source, pin, &self.parent)
                        .await
                        .map_err(content_error)?,
                );
            }
            self.retained.as_ref().ok_or(Error::Unavailable)?
        };
        let entry = contents.entry_for_pin(pin).map_err(content_error)?;
        require_current(state, kind, pin).await?;
        Ok(entry)
    }
}
