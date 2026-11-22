//! Operator-only catalog observations, separate from runtime admission.
use capsem_api::environments::{CatalogAvailability, CatalogKind, CatalogRefreshError, CatalogStatus};
use capsem_assets::catalogs::ValidatedCatalogSource;
use std::sync::Mutex;

pub(crate) struct Cache(Mutex<State>);

struct State {
    sequence: u64,
    last_policy_check: u64,
    entries: [Observation; CatalogKind::ALL.len()],
}

struct Observation {
    status: CatalogStatus,
    last_refresh: u64,
}

impl Default for Cache {
    fn default() -> Self {
        Self(Mutex::new(State {
            sequence: 0,
            last_policy_check: 0,
            entries: CatalogKind::ALL.map(|kind| Observation {
                status: unavailable(kind, CatalogRefreshError::PolicyUnavailable),
                last_refresh: 0,
            }),
        }))
    }
}

pub(super) fn unavailable(kind: CatalogKind, error: CatalogRefreshError) -> CatalogStatus {
    CatalogStatus {
        kind,
        availability: CatalogAvailability::Unavailable,
        reference: None,
        origin: None,
        digest: None,
        last_successful_refresh: None,
        error: Some(error),
    }
}

impl Cache {
    pub(crate) fn read(&self) -> Vec<CatalogStatus> {
        self.0
            .lock()
            .unwrap()
            .entries
            .iter()
            .map(|entry| entry.status.clone())
            .collect()
    }

    pub(super) fn begin(&self) -> u64 {
        let mut state = self.0.lock().unwrap();
        state.sequence += 1;
        state.sequence
    }

    pub(super) fn observe_policy(&self, observed: u64, statuses: [CatalogStatus; CatalogKind::ALL.len()]) {
        let mut state = self.0.lock().unwrap();
        if observed <= state.last_policy_check {
            return;
        }
        state.last_policy_check = observed;
        for (entry, status) in state.entries.iter_mut().zip(statuses) {
            let current = &entry.status;
            // A successful or failed probe updates availability independently.
            // Keep that observation while policy still selects the same source.
            if status.availability == CatalogAvailability::Unverified
                && status.reference == current.reference
                && status.origin == current.origin
            {
                continue;
            }
            *entry = Observation {
                status,
                last_refresh: observed,
            };
        }
    }

    pub(super) fn observe_refresh(
        &self,
        observed: u64,
        source: &ValidatedCatalogSource,
        outcome: Result<(&str, i64), CatalogRefreshError>,
    ) {
        let reference = source.reference().to_string();
        let mut state = self.0.lock().unwrap();
        let entry = state
            .entries
            .iter_mut()
            .find(|entry| entry.status.kind == source.kind())
            .unwrap();
        if observed <= entry.last_refresh
            || entry.status.reference.as_deref() != Some(reference.as_str())
            || entry.status.origin != Some(source.origin())
        {
            return;
        }
        entry.last_refresh = observed;
        match outcome {
            Ok((digest, refreshed)) => {
                entry.status.availability = CatalogAvailability::Available;
                entry.status.digest = Some(digest.to_owned());
                entry.status.last_successful_refresh = Some(refreshed);
                entry.status.error = None;
            }
            Err(error) => {
                entry.status.availability = if entry.status.digest.is_some() {
                    CatalogAvailability::Stale
                } else {
                    CatalogAvailability::Unavailable
                };
                entry.status.error = Some(error);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use capsem_api::environments::CatalogSourceOrigin;
    use capsem_config::catalogs::{CatalogSourceDescriptor, SelectedCatalogSource};

    fn source(name: &str) -> ValidatedCatalogSource {
        ValidatedCatalogSource::new(
            CatalogKind::Skills,
            SelectedCatalogSource {
                source: &CatalogSourceDescriptor {
                    reference: format!("registry.example/team/{name}:stable"),
                    schema_version: 1,
                },
                origin: CatalogSourceOrigin::Manifest,
            },
        )
        .unwrap()
    }

    fn policy(source: &ValidatedCatalogSource) -> [CatalogStatus; 3] {
        CatalogKind::ALL.map(|kind| {
            let mut status = unavailable(kind, CatalogRefreshError::MissingSource);
            if kind == source.kind() {
                status.reference = Some(source.reference().to_string());
                status.origin = Some(source.origin());
                status.availability = CatalogAvailability::Unverified;
                status.error = None;
            }
            status
        })
    }

    #[test]
    fn late_policy_or_refresh_results_cannot_overwrite_a_newer_catalog_observation() {
        let cache = Cache::default();
        let first = source("first");
        let second = source("second");
        cache.observe_policy(cache.begin(), policy(&first));
        let old_policy = cache.begin();
        let old_refresh = cache.begin();
        cache.observe_policy(cache.begin(), policy(&second));
        cache.observe_policy(old_policy, policy(&first));
        cache.observe_refresh(old_refresh, &first, Ok(("old", 10)));
        assert_eq!(cache.read(), policy(&second));

        // If policy leaves a source and later picks it again, work started
        // before that replacement still cannot speak for the new observation.
        cache.observe_policy(cache.begin(), policy(&first));
        cache.observe_refresh(old_refresh, &first, Ok(("old", 10)));
        assert_eq!(cache.read(), policy(&first));
        let slow = cache.begin();
        let fast = cache.begin();
        cache.observe_refresh(fast, &first, Ok(("fresh", 30)));
        cache.observe_refresh(slow, &first, Err(CatalogRefreshError::RegistryUnavailable));
        let latest = &cache.read()[0];
        assert_eq!(latest.digest.as_deref(), Some("fresh"));
        assert_eq!(latest.last_successful_refresh, Some(30));
        assert_eq!(latest.availability, CatalogAvailability::Available);
    }
}
