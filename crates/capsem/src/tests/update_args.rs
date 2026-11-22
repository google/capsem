//! Update command arguments and validation.

use super::*;

#[test]
fn parse_update() {
    let cli = Cli::parse_from(["capsem", "update"]);
    match cli.command.unwrap() {
        Commands::Misc(MiscCommands::Update {
            yes,
            check,
            assets,
            channel,
            manifest,
            install_manifest_stdin,
            corp,
        }) => {
            assert!(!yes);
            assert!(!check);
            assert!(!assets);
            assert_eq!(channel, None);
            assert_eq!(manifest, None);
            assert!(!install_manifest_stdin);
            assert_eq!(corp, None);
        }
        _ => panic!("expected Update"),
    }
}

#[test]
fn parse_update_yes() {
    let cli = Cli::parse_from(["capsem", "update", "--yes"]);
    match cli.command.unwrap() {
        Commands::Misc(MiscCommands::Update {
            yes,
            check,
            assets,
            channel,
            manifest,
            install_manifest_stdin,
            corp,
        }) => {
            assert!(yes);
            assert!(!check);
            assert!(!assets);
            assert_eq!(channel, None);
            assert_eq!(manifest, None);
            assert!(!install_manifest_stdin);
            assert_eq!(corp, None);
        }
        _ => panic!("expected Update"),
    }
}

#[test]
fn parse_update_check() {
    let cli = Cli::parse_from(["capsem", "update", "--check"]);
    match cli.command.unwrap() {
        Commands::Misc(MiscCommands::Update {
            yes,
            check,
            assets,
            channel,
            manifest,
            install_manifest_stdin,
            corp,
        }) => {
            assert!(!yes);
            assert!(check);
            assert!(!assets);
            assert_eq!(channel, None);
            assert_eq!(manifest, None);
            assert!(!install_manifest_stdin);
            assert_eq!(corp, None);
        }
        _ => panic!("expected Update"),
    }
}

#[test]
fn parse_update_check_rejects_mutating_options() {
    for args in [
        vec!["capsem", "update", "--check", "--yes"],
        vec!["capsem", "update", "--check", "--assets"],
        vec![
            "capsem",
            "update",
            "--check",
            "--manifest",
            "https://release.capsem.org/assets/stable/manifest.json",
        ],
        vec![
            "capsem",
            "update",
            "--check",
            "--corp",
            "https://corp.example/capsem/corp.json",
        ],
    ] {
        assert!(
            Cli::try_parse_from(args.clone()).is_err(),
            "expected {args:?} to be rejected"
        );
    }
}

#[test]
fn parse_update_corp_requires_authenticated_transport() {
    assert!(Cli::try_parse_from(["capsem", "update", "--corp", "http://corp.example/capsem/corp.toml",]).is_err());
    for source in ["https://corp.example/capsem/corp.toml", "file:///etc/capsem/corp.toml"] {
        assert!(
            Cli::try_parse_from(["capsem", "update", "--corp", source]).is_ok(),
            "{source}"
        );
    }
}

#[test]
fn parse_hidden_install_manifest_stdin_requires_the_exact_asset_handoff_shape() {
    let cli = Cli::parse_from([
        "capsem",
        "update",
        "--assets",
        "--manifest",
        "https://release.capsem.org/assets/nightly/manifest.json",
        "--install-manifest-stdin",
    ]);
    match cli.command.unwrap() {
        Commands::Misc(MiscCommands::Update {
            assets,
            manifest,
            install_manifest_stdin,
            ..
        }) => {
            assert!(assets);
            assert!(manifest.is_some());
            assert!(install_manifest_stdin);
        }
        _ => panic!("expected Update"),
    }

    for args in [
        vec!["capsem", "update", "--install-manifest-stdin"],
        vec![
            "capsem",
            "update",
            "--manifest",
            "file:///tmp/manifest.json",
            "--install-manifest-stdin",
        ],
        vec![
            "capsem",
            "update",
            "--assets",
            "--manifest",
            "file:///tmp/manifest.json",
            "--install-manifest-stdin",
            "--corp",
            "file:///tmp/corp.toml",
        ],
    ] {
        assert!(Cli::try_parse_from(args).is_err());
    }
    let help = match Cli::try_parse_from(["capsem", "update", "--help"]) {
        Err(error) => error.to_string(),
        Ok(_) => panic!("--help must stop parsing"),
    };
    assert!(!help.contains("install-manifest-stdin"));
}

#[test]
fn parse_update_assets() {
    let cli = Cli::parse_from(["capsem", "update", "--assets"]);
    match cli.command.unwrap() {
        Commands::Misc(MiscCommands::Update {
            yes,
            check,
            assets,
            channel,
            manifest,
            install_manifest_stdin,
            corp,
        }) => {
            assert!(!yes);
            assert!(!check);
            assert!(assets);
            assert_eq!(channel, None);
            assert_eq!(manifest, None);
            assert!(!install_manifest_stdin);
            assert_eq!(corp, None);
        }
        _ => panic!("expected Update"),
    }
}

#[test]
fn parse_update_named_channel_for_check_and_asset_switch() {
    for args in [
        ["capsem", "update", "--check", "--channel", "nightly"],
        ["capsem", "update", "--assets", "--channel", "stable"],
    ] {
        let cli = Cli::parse_from(args);
        match cli.command.unwrap() {
            Commands::Misc(MiscCommands::Update { channel, .. }) => {
                assert!(matches!(channel.as_deref(), Some("nightly" | "stable")));
            }
            _ => panic!("expected Update"),
        }
    }
}

#[test]
fn parse_update_rejects_invalid_or_ambiguous_channel_selection() {
    for args in [
        vec!["capsem", "update", "--channel", "../nightly"],
        vec![
            "capsem",
            "update",
            "--channel",
            "nightly",
            "--manifest",
            "https://release.capsem.org/assets/stable/manifest.json",
        ],
    ] {
        assert!(Cli::try_parse_from(args).is_err());
    }
}

#[test]
fn parse_update_rejects_assets_with_corp_policy() {
    let err = match Cli::try_parse_from([
        "capsem",
        "update",
        "--assets",
        "--corp",
        "https://corp.example/capsem/corp.toml",
    ]) {
        Ok(_) => panic!("--corp provisions policy config and must not combine with --assets"),
        Err(err) => err,
    };
    let message = err.to_string();
    assert!(message.contains("cannot be used with"), "{message}");
    assert!(message.contains("--assets"), "{message}");
}

#[test]
fn parse_update_url_overrides_reject_bare_paths() {
    for flag in ["--manifest", "--corp"] {
        for source in ["/tmp/capsem/manifest.json", "assets/stable/manifest.json"] {
            let err = match Cli::try_parse_from(["capsem", "update", "--assets", flag, source]) {
                Ok(_) => panic!("update source overrides must reject bare filesystem paths"),
                Err(err) => err,
            };
            let message = err.to_string();
            assert!(message.contains(&format!("{flag} must be a URL")), "{message}");
            assert!(message.contains("https://..."), "{message}");
            assert!(message.contains("http://..."), "{message}");
            assert!(message.contains("file:///absolute/path"), "{message}");
        }
    }
}

#[test]
fn parse_update_url_overrides_reject_url_shorthand_paths() {
    for flag in ["--manifest", "--corp"] {
        for (source, expected) in [
            ("file:assets/stable/manifest.json", "file URL must start with file://"),
            (
                "https:release.capsem.org/assets/stable/manifest.json",
                "must use https://, http://, or file:// URLs",
            ),
        ] {
            let err = match Cli::try_parse_from(["capsem", "update", "--assets", flag, source]) {
                Ok(_) => panic!("update source overrides must reject URL shorthand paths"),
                Err(err) => err,
            };
            let message = err.to_string();
            assert!(message.contains(expected), "{message}");
        }
    }
}

// CAPSEM_RUN_DIR resolution
