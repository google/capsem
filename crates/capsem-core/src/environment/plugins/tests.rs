use super::*;
use capsem_config::SecurityPluginMode::{Allow, Ask, Block, Disable, Rewrite};

#[test]
fn environment_plugin_compilation_defaults_are_typed_and_keep_required_builtins() {
    let compiled =
        CompiledEnvironmentPlugins::resolve(&EnvironmentPlugins::default(), &EnvironmentPlugins::default()).unwrap();
    for id in PluginId::ALL {
        for direction in [PluginDirection::Inbound, PluginDirection::Outbound] {
            let config = compiled.configuration(id, direction);
            assert_eq!(config.source, EnvironmentPluginSource::Builtin);
            assert!(!config.locked());
            let expected = if matches!(id, PluginId::CredentialBroker | PluginId::LogSanitizer) {
                Rewrite
            } else {
                Disable
            };
            assert_eq!(config.mode, expected);
        }
    }
    assert!(matches!(
        compiled
            .configuration(PluginId::CredentialBroker, PluginDirection::Outbound)
            .options,
        CompiledPluginOptions::CredentialBroker { capture_enabled: true }
    ));
    assert!(matches!(
        compiled
            .configuration(PluginId::Utf8Guard, PluginDirection::Inbound)
            .options,
        CompiledPluginOptions::Utf8Guard
    ));
    assert!(matches!(
        compiled
            .configuration(PluginId::LogSanitizer, PluginDirection::Inbound)
            .options,
        CompiledPluginOptions::LogSanitizer
    ));
}

#[test]
fn environment_plugin_compilation_keeps_independent_pii_modes_levels_and_categories() {
    let mut environment = EnvironmentPlugins {
        pii_filter: Some(PiiFilterConfiguration {
            inbound: PluginConfiguration {
                mode: Rewrite,
                detection_level: DetectionLevel::High,
                options: PiiFilterOptions {
                    email: false,
                    ..Default::default()
                },
            },
            outbound: PluginConfiguration {
                mode: Block,
                detection_level: DetectionLevel::Low,
                options: PiiFilterOptions {
                    phone: false,
                    ..Default::default()
                },
            },
        }),
        ..Default::default()
    };
    let compiled = CompiledEnvironmentPlugins::resolve(&environment, &EnvironmentPlugins::default()).unwrap();
    let inbound = compiled.configuration(PluginId::PiiFilter, PluginDirection::Inbound);
    let outbound = compiled.configuration(PluginId::PiiFilter, PluginDirection::Outbound);
    assert_eq!(inbound.mode, Rewrite);
    assert_eq!(outbound.mode, Block);
    assert_eq!(inbound.active_detection_level(), Some(DetectionLevel::High));
    assert_eq!(outbound.active_detection_level(), Some(DetectionLevel::Low));
    let CompiledPluginOptions::PiiFilter(inbound_types) = &inbound.options else {
        panic!("PII-specific options")
    };
    let CompiledPluginOptions::PiiFilter(outbound_types) = &outbound.options else {
        panic!("PII-specific options")
    };
    assert!(!inbound_types.email && inbound_types.phone);
    assert!(outbound_types.email && !outbound_types.phone);
    assert_eq!(inbound.source, EnvironmentPluginSource::Environment);
    environment.pii_filter.as_mut().unwrap().inbound.mode = Disable;
    let disabled = CompiledEnvironmentPlugins::resolve(&environment, &EnvironmentPlugins::default()).unwrap();
    assert_eq!(
        disabled
            .configuration(PluginId::PiiFilter, PluginDirection::Inbound)
            .active_detection_level(),
        None
    );
    assert_eq!(
        disabled
            .configuration(PluginId::PiiFilter, PluginDirection::Outbound)
            .mode,
        Block
    );
}

#[test]
fn environment_plugin_compilation_preserves_whole_corporate_entry_replacement() {
    let environment = EnvironmentPlugins {
        pii_filter: Some(PiiFilterConfiguration {
            inbound: PluginConfiguration {
                mode: Rewrite,
                options: PiiFilterOptions {
                    email: false,
                    ..Default::default()
                },
                ..Default::default()
            },
            outbound: PluginConfiguration {
                mode: Block,
                ..Default::default()
            },
        }),
        ..Default::default()
    };
    let corporate = EnvironmentPlugins {
        pii_filter: Some(PiiFilterConfiguration {
            inbound: PluginConfiguration {
                mode: Ask,
                detection_level: DetectionLevel::Critical,
                ..Default::default()
            },
            ..Default::default()
        }),
        ..Default::default()
    };
    let compiled = CompiledEnvironmentPlugins::resolve(&environment, &corporate).unwrap();
    for direction in [PluginDirection::Inbound, PluginDirection::Outbound] {
        assert!(compiled.configuration(PluginId::PiiFilter, direction).locked());
        assert_eq!(
            compiled.configuration(PluginId::PiiFilter, direction).source,
            EnvironmentPluginSource::Corporate
        );
    }
    assert_eq!(
        compiled
            .configuration(PluginId::PiiFilter, PluginDirection::Outbound)
            .mode,
        Disable,
        "whole entry replacement does not inherit the environment's outbound setting"
    );
    let inbound = compiled.configuration(PluginId::PiiFilter, PluginDirection::Inbound);
    assert_eq!(inbound.mode, Ask);
    assert_eq!(inbound.detection_level, DetectionLevel::Critical);
    assert!(matches!(&inbound.options, CompiledPluginOptions::PiiFilter(options) if options.email));
}

#[test]
fn environment_plugin_compilation_separates_capture_and_utf8_direction_without_weakening_required_modes() {
    let mut environment = EnvironmentPlugins {
        credential_broker: Some(PluginConfiguration {
            options: CredentialBrokerOptions { capture_enabled: false },
            ..Default::default()
        }),
        utf8_guard: Some(PluginConfiguration {
            mode: Block,
            options: Utf8GuardOptions {
                inbound: false,
                outbound: true,
            },
            ..Default::default()
        }),
        dummy_post_allow: Some(PluginConfiguration {
            mode: Allow,
            ..Default::default()
        }),
        ..Default::default()
    };
    let compiled = CompiledEnvironmentPlugins::resolve(&environment, &EnvironmentPlugins::default()).unwrap();
    assert_eq!(
        compiled
            .configuration(PluginId::Utf8Guard, PluginDirection::Inbound)
            .mode,
        Disable
    );
    assert_eq!(
        compiled
            .configuration(PluginId::Utf8Guard, PluginDirection::Outbound)
            .mode,
        Block
    );
    for direction in [PluginDirection::Inbound, PluginDirection::Outbound] {
        let broker = compiled.configuration(PluginId::CredentialBroker, direction);
        assert_eq!(broker.mode, Rewrite);
        assert!(matches!(
            broker.options,
            CompiledPluginOptions::CredentialBroker { capture_enabled: false }
        ));
        assert_eq!(compiled.configuration(PluginId::LogSanitizer, direction).mode, Rewrite);
    }
    environment.credential_broker.as_mut().unwrap().mode = Disable;
    assert!(CompiledEnvironmentPlugins::resolve(&environment, &EnvironmentPlugins::default()).is_err());
    environment.credential_broker.as_mut().unwrap().mode = Rewrite;
    environment.utf8_guard.as_mut().unwrap().mode = Rewrite;
    assert!(CompiledEnvironmentPlugins::resolve(&environment, &EnvironmentPlugins::default()).is_err());
    let corporate = EnvironmentPlugins {
        log_sanitizer: Some(PluginConfiguration {
            mode: Disable,
            ..Default::default()
        }),
        ..Default::default()
    };
    assert!(CompiledEnvironmentPlugins::resolve(&EnvironmentPlugins::default(), &corporate).is_err());
}
