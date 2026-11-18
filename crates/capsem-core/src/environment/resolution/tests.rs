use super::*;
use std::sync::Mutex;

#[derive(Default)]
struct Sources {
    calls: Mutex<Vec<String>>,
    invalid: Option<&'static str>,
    revoked: bool,
    refused_image: bool,
    wrong_selection: bool,
}
impl EnvironmentPinResolver for Sources {
    async fn image(&self, reference: &str) -> Result<String, EnvironmentResolutionError> {
        self.calls.lock().unwrap().push(format!("image:{reference}"));
        if self.refused_image {
            return Err(EnvironmentResolutionError::Refused);
        }
        Ok(if self.invalid == Some("image") {
            "bad".into()
        } else {
            format!("sha256:{}", "a".repeat(64))
        })
    }
    async fn repository(&self, url: &str, reference: &str) -> Result<String, EnvironmentResolutionError> {
        self.calls.lock().unwrap().push(format!("repository:{url}@{reference}"));
        Ok(if self.invalid == Some("repository") {
            "bad".into()
        } else {
            "b".repeat(40)
        })
    }
    async fn catalog(
        &self,
        kind: CatalogKind,
        selection: &EnvironmentCatalogSelection,
    ) -> Result<EnvironmentCatalogPin, EnvironmentResolutionError> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("catalog:{kind:?}:{}", selection.id));
        Ok(EnvironmentCatalogPin {
            id: if self.wrong_selection {
                "different".into()
            } else {
                selection.id.clone()
            },
            version: selection.version.clone(),
            catalog_reference: "registry.example/catalog:stable".into(),
            resolved_digest: if self.invalid == Some("catalog") {
                "bad".into()
            } else {
                format!("sha256:{}", "c".repeat(64))
            },
        })
    }
    async fn admit_catalog(
        &self,
        kind: CatalogKind,
        pin: &EnvironmentCatalogPin,
    ) -> Result<(), EnvironmentResolutionError> {
        self.calls.lock().unwrap().push(format!("admit:{kind:?}:{}", pin.id));
        if self.revoked {
            Err(EnvironmentResolutionError::UnusableCatalogEntry)
        } else {
            Ok(())
        }
    }
}
fn draft() -> EnvironmentDraft {
    toml::from_str(
        r#"
name='development'
[image]
reference='development:stable'
[[repositories]]
url='https://example.org/project.git'
ref='main'
destination='project'
[[skills.selected]]
id='sandbox'
version='1.0.0'
[[access.catalog.selected]]
id='access'
version='1.0.0'
[[detection.catalog.selected]]
id='observe'
version='1.0.0'
[[mcps.catalog.selected]]
id='server'
version='1.0.0'
"#,
    )
    .unwrap()
}

#[tokio::test]
async fn new_resolution_moves_user_buffers_and_resolves_each_selection_once() {
    let mut draft = draft();
    draft.variables.insert("LARGE".into(), "retained buffer ".repeat(512));
    draft.startup = Some(EnvironmentStartup {
        script: "printf 'retained script'\n".repeat(512),
        timeout_seconds: 30,
    });
    let variable = draft.variables["LARGE"].as_ptr();
    let script = draft.startup.as_ref().unwrap().script.as_ptr();
    let sources = Sources::default();
    let result = resolve_environment(draft, None, EnvironmentResolutionMode::Edit, &sources)
        .await
        .unwrap();
    assert_eq!(result.variables["LARGE"].as_ptr(), variable);
    assert_eq!(result.startup.as_ref().unwrap().script.as_ptr(), script);
    let calls = sources.calls.lock().unwrap();
    assert_eq!(calls.iter().filter(|call| call.starts_with("image:")).count(), 1);
    assert_eq!(calls.iter().filter(|call| call.starts_with("repository:")).count(), 1);
    assert_eq!(calls.iter().filter(|call| call.starts_with("catalog:")).count(), 4);
    assert_eq!(calls.iter().filter(|call| call.starts_with("admit:")).count(), 4);
    for expected in [
        "catalog:Skills:sandbox",
        "catalog:Rules:access",
        "catalog:Rules:observe",
        "catalog:Mcp:server",
    ] {
        assert!(calls.iter().any(|call| call == expected), "missing {expected}");
    }
    drop(calls);
}

#[tokio::test]
async fn edits_keep_exact_base_pins_but_recheck_current_catalog_authority() {
    let mut base = resolve_environment(draft(), None, EnvironmentResolutionMode::Edit, &Sources::default())
        .await
        .unwrap();
    base.image.as_mut().unwrap().resolved_digest = format!("sha256:{}", "d".repeat(64));
    base.repositories[0].resolved_commit = "e".repeat(40);
    base.skills.selected[0].resolved_digest = format!("sha256:{}", "f".repeat(64));
    let mut edited = base.clone().into_draft();
    edited.description = "unrelated edit".into();
    edited.image.as_mut().unwrap().update_policy = EnvironmentImageUpdatePolicy::Ask;
    edited.repositories[0].destination = "relocated".into();
    let sources = Sources::default();
    let result = resolve_environment(edited, Some(&base), EnvironmentResolutionMode::Edit, &sources)
        .await
        .unwrap();
    assert_eq!(
        result.image.as_ref().unwrap().resolved_digest,
        base.image.as_ref().unwrap().resolved_digest
    );
    assert_eq!(
        result.repositories[0].resolved_commit,
        base.repositories[0].resolved_commit
    );
    assert!(
        result.skills == base.skills
            && result.access == base.access
            && result.detection == base.detection
            && result.mcps == base.mcps
    );
    assert_eq!(result.repositories[0].destination, "relocated");
    assert_eq!(
        result.image.as_ref().unwrap().update_policy,
        EnvironmentImageUpdatePolicy::Ask
    );
    assert!(sources
        .calls
        .lock()
        .unwrap()
        .iter()
        .all(|call| call.starts_with("admit:")));
    let denied = Sources {
        revoked: true,
        ..Sources::default()
    };
    assert_eq!(
        resolve_environment(
            base.clone().into_draft(),
            Some(&base),
            EnvironmentResolutionMode::Edit,
            &denied
        )
        .await
        .err(),
        Some(EnvironmentResolutionError::UnusableCatalogEntry)
    );
}

#[tokio::test]
async fn explicit_image_refresh_leaves_other_pins_frozen_and_changed_selectors_resolve() {
    let mut base = resolve_environment(draft(), None, EnvironmentResolutionMode::Edit, &Sources::default())
        .await
        .unwrap();
    base.image.as_mut().unwrap().resolved_digest = format!("sha256:{}", "d".repeat(64));
    let sources = Sources::default();
    let refreshed = resolve_environment(
        base.clone().into_draft(),
        Some(&base),
        EnvironmentResolutionMode::RefreshImage,
        &sources,
    )
    .await
    .unwrap();
    assert_ne!(
        refreshed.image.as_ref().unwrap().resolved_digest,
        base.image.as_ref().unwrap().resolved_digest
    );
    assert!(refreshed.repositories == base.repositories && refreshed.skills == base.skills);
    assert_eq!(
        sources
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| !call.starts_with("admit:"))
            .cloned()
            .collect::<Vec<_>>(),
        ["image:development:stable"]
    );
    let mut changed = base.clone().into_draft();
    changed.image.as_mut().unwrap().reference = "development:new".into();
    changed.repositories[0].reference = "release".into();
    changed.skills.selected[0].version = "2.0.0".into();
    let sources = Sources::default();
    let resolved = resolve_environment(changed, Some(&base), EnvironmentResolutionMode::Edit, &sources)
        .await
        .unwrap();
    assert_eq!(resolved.skills.selected[0].version, "2.0.0");
    assert_eq!(
        sources
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| !call.starts_with("admit:"))
            .count(),
        3
    );
}

#[tokio::test]
async fn source_output_cannot_change_selection_or_publish_invalid_pins() {
    let sources = Sources {
        wrong_selection: true,
        ..Sources::default()
    };
    assert_eq!(
        resolve_environment(draft(), None, EnvironmentResolutionMode::Edit, &sources)
            .await
            .err(),
        Some(EnvironmentResolutionError::InvalidPin)
    );
    for field in ["image", "repository", "catalog"] {
        let mut draft = draft();
        if field == "catalog" {
            draft.image = None;
        }
        if field != "repository" {
            draft.repositories.clear();
        }
        if field != "catalog" {
            draft.skills.selected.clear();
            draft.access.catalog.selected.clear();
            draft.detection.catalog.selected.clear();
            draft.mcps.catalog.selected.clear();
        }
        let sources = Sources {
            invalid: Some(field),
            ..Sources::default()
        };
        assert_eq!(
            resolve_environment(draft, None, EnvironmentResolutionMode::Edit, &sources)
                .await
                .err(),
            Some(EnvironmentResolutionError::InvalidPin)
        );
    }
    let mut invalid = draft();
    invalid.name = String::new();
    let sources = Sources::default();
    assert_eq!(
        resolve_environment(invalid, None, EnvironmentResolutionMode::Edit, &sources)
            .await
            .err(),
        Some(EnvironmentResolutionError::InvalidIntent)
    );
    assert!(sources.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn policy_refusal_stops_resolution_without_consulting_later_sources() {
    let sources = Sources {
        refused_image: true,
        ..Sources::default()
    };
    assert_eq!(
        resolve_environment(draft(), None, EnvironmentResolutionMode::Edit, &sources)
            .await
            .err(),
        Some(EnvironmentResolutionError::Refused)
    );
    assert_eq!(*sources.calls.lock().unwrap(), ["image:development:stable"]);
}
