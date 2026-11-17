//! Recover editable selectors from a resolved specification by moving content.
use super::*;

fn selections(
    input: EnvironmentCatalogSelections<EnvironmentCatalogPin>,
) -> EnvironmentCatalogSelections<EnvironmentCatalogSelection> {
    EnvironmentCatalogSelections {
        selected: input
            .selected
            .into_iter()
            .map(|pin| EnvironmentCatalogSelection {
                id: pin.id,
                version: pin.version,
            })
            .collect(),
    }
}
fn rules<R>(input: EnvironmentRules<EnvironmentCatalogPin, R>) -> EnvironmentRules<EnvironmentCatalogSelection, R> {
    EnvironmentRules {
        catalog: selections(input.catalog),
        parameters: input.parameters,
        custom: input.custom,
    }
}

impl EnvironmentSpecification {
    /// Consume resolved content to recover the exact editable intent. Only host
    /// resolution fields are discarded; large user buffers retain ownership.
    /// The resulting draft follows the ordinary validation and approval flow.
    pub fn into_draft(self) -> EnvironmentDraft {
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
            skills: selections(skills),
            access: rules(access),
            detection: rules(detection),
            mcps: EnvironmentMcps {
                catalog: selections(mcps.catalog),
                servers: mcps.servers,
                permissions: mcps.permissions,
            },
            plugins,
        }
    }
}
