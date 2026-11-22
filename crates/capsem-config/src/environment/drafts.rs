//! Recover editable selectors from a resolved specification by moving content.
use super::*;

fn selections(
    input: EnvironmentCatalogSelections<EnvironmentCatalogPin>,
    requested: Option<&EnvironmentCatalogSelections<EnvironmentCatalogSelection>>,
) -> EnvironmentCatalogSelections<EnvironmentCatalogSelection> {
    EnvironmentCatalogSelections {
        selected: input
            .selected
            .into_iter()
            .enumerate()
            .map(|(index, pin)| {
                let expected_catalog = requested
                    .and_then(|requested| requested.selected.get(index))
                    .and_then(|selection| selection.expected_catalog.as_ref())
                    .filter(|expected| expected.matches(&pin))
                    .cloned();
                EnvironmentCatalogSelection {
                    id: pin.id,
                    version: pin.version,
                    expected_catalog,
                }
            })
            .collect(),
    }
}
fn rules<R>(
    input: EnvironmentRules<EnvironmentCatalogPin, R>,
    requested: Option<&EnvironmentRules<EnvironmentCatalogSelection, R>>,
) -> EnvironmentRules<EnvironmentCatalogSelection, R> {
    EnvironmentRules {
        catalog: selections(input.catalog, requested.map(|rules| &rules.catalog)),
        parameters: input.parameters,
        custom: input.custom,
    }
}

impl EnvironmentSpecification {
    /// Consume resolved content to recover the exact editable intent. Only host
    /// resolution fields are discarded; large user buffers retain ownership.
    /// The resulting draft follows the ordinary validation and approval flow.
    pub fn into_draft(self) -> EnvironmentDraft {
        self.draft_for(None)
    }

    /// A retry can reuse the receipt when its expectations match the recorded
    /// host-verified pins and the rest of the intent matches exactly.
    pub fn matches_draft(self, requested: &EnvironmentDraft) -> bool {
        self.draft_for(Some(requested)) == *requested
    }

    fn draft_for(self, requested: Option<&EnvironmentDraft>) -> EnvironmentDraft {
        let Self {
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
        } = self;
        EnvironmentSpecification {
            name,
            description,
            image: image.map(|image| EnvironmentImageSelection {
                reference: image.reference,
                update_policy: image.update_policy,
            }),
            startup,
            variables,
            secrets,
            repositories: repositories
                .into_iter()
                .map(|repository| EnvironmentRepositorySelection {
                    url: repository.url,
                    reference: repository.reference,
                    destination: repository.destination,
                })
                .collect(),
            packages,
            skills: selections(skills, requested.map(|draft| &draft.skills)),
            access: rules(access, requested.map(|draft| &draft.access)),
            detection: rules(detection, requested.map(|draft| &draft.detection)),
            mcps: EnvironmentMcps {
                catalog: selections(mcps.catalog, requested.map(|draft| &draft.mcps.catalog)),
                servers: mcps.servers,
                permissions: mcps.permissions,
            },
            plugins,
        }
    }
}
