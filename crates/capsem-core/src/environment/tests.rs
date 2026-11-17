use super::*;
use capsem_config::environment::*;

type Change = fn(&mut EnvironmentSpecification);

fn specification() -> EnvironmentSpecification {
    toml::from_str("name='development'\n[image]\nreference='development:stable'\nresolved_digest='sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'").unwrap()
}

#[test]
fn environment_change_impact_assigns_every_field_to_its_delivery_owner() {
    use EnvironmentChangeKind::*;
    use EnvironmentChangeTiming::*;
    let cases: [(EnvironmentChangeKind, EnvironmentChangeTiming, Change); 14] = [
        (Name, Metadata, |spec| spec.name = "renamed".into()),
        (Description, Metadata, |spec| {
            spec.description = "reviewed description".into()
        }),
        (Image, NextStart, |spec| {
            spec.image.as_mut().unwrap().resolved_digest = format!("sha256:{}", "b".repeat(64))
        }),
        (Startup, NextStart, |spec| {
            spec.startup = Some(EnvironmentStartup {
                script: "printf ready".into(),
                timeout_seconds: 120,
            })
        }),
        (Variables, NextStart, |spec| {
            spec.variables.insert("PROJECT".into(), "value".into());
        }),
        (SecretAccess, Live, |spec| {
            spec.secrets.insert(
                "api".into(),
                EnvironmentSecretBinding {
                    binding_id: "00000000-0000-4000-8000-000000000001".into(),
                    env_name: None,
                },
            );
        }),
        (Repositories, NextStart, |spec| {
            spec.repositories.push(EnvironmentRepository {
                url: "https://example.org/project.git".into(),
                reference: "main".into(),
                resolved_commit: "a".repeat(40),
                destination: "project".into(),
            })
        }),
        (PackageAccess, Live, |spec| {
            spec.packages.allows_package_manager = true;
            spec.packages.registries = vec!["npm".into()];
        }),
        (AccessRules, Live, |spec| {
            spec.access.custom.insert(
                "network".into(),
                toml::from_str("name='Network'\naction='block'\nmatch='http.host == \"blocked.example\"'").unwrap(),
            );
        }),
        (DetectionRules, NextStart, |spec| {
            spec.detection.custom.insert(
                "observe".into(),
                toml::from_str("name='Observe'\nlevel='high'\nmatch='http.host == \"observed.example\"'").unwrap(),
            );
        }),
        (Plugins, NextStart, |spec| {
            spec.plugins.utf8_guard = Some(Default::default())
        }),
        (McpDefinitions, NextStart, |spec| {
            spec.mcps.servers.insert(
                "remote".into(),
                EnvironmentMcpDefinition::Remote {
                    url: "https://mcp.example.org/rpc".into(),
                    headers: Default::default(),
                    binding_id: None,
                },
            );
        }),
        (McpPermissions, Live, |spec| {
            spec.mcps.permissions.insert(
                "local".into(),
                EnvironmentMcpPermissions {
                    default: EnvironmentCapabilityDecision::Ask,
                    ..Default::default()
                },
            );
        }),
        (Skills, NextStart, |spec| {
            spec.skills.selected.push(EnvironmentCatalogPin {
                id: "sandbox".into(),
                version: "1.0.0".into(),
                catalog_reference: "ghcr.io/google/capsem/skills:stable".into(),
                resolved_digest: format!("sha256:{}", "a".repeat(64)),
            })
        }),
    ];
    for (kind, timing, change) in cases {
        let before = specification();
        let mut after = before.clone();
        change(&mut after);
        after.validate().unwrap();
        let impact = EnvironmentChangeImpact::classify(&before, &after);
        assert_eq!(impact.changes(), [EnvironmentChange { kind, timing }], "{kind:?}");
        assert_eq!(impact.has_deferred_changes(), timing == NextStart);
        assert_eq!(impact.affects_live_policy(), timing == Live);
    }
}

#[test]
fn environment_change_impact_preserves_mixed_timing_and_ignores_revision_identity() {
    use EnvironmentChangeKind::*;
    use EnvironmentChangeTiming::*;
    let before = specification();
    assert!(EnvironmentChangeImpact::classify(&before, &before).changes().is_empty());
    let mut after = before.clone();
    after.variables.insert("PROJECT".into(), "next-start".into());
    after.mcps.permissions.insert(
        "local".into(),
        EnvironmentMcpPermissions {
            default: EnvironmentCapabilityDecision::Allow,
            ..Default::default()
        },
    );
    let impact = EnvironmentChangeImpact::classify(&before, &after);
    assert_eq!(
        impact.changes(),
        [
            EnvironmentChange {
                kind: Variables,
                timing: NextStart
            },
            EnvironmentChange {
                kind: McpPermissions,
                timing: Live
            }
        ]
    );
    assert!(impact.has_deferred_changes() && impact.affects_live_policy());
    after.variables.clear();
    let impact = EnvironmentChangeImpact::classify(&before, &after);
    assert!(!impact.has_deferred_changes() && impact.affects_live_policy());
    // Revision IDs are intentionally absent from the inputs: a new revision
    // with only live content must not manufacture a workload restart.
}

mod policy_inputs;
