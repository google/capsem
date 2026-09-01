use super::*;

#[test]
fn image_build_requires_profile_argument() {
    let error = Cli::try_parse_from(["capsem-admin", "image", "build"]).expect_err("profile is required");

    assert!(error.to_string().contains("--profile"), "{error}");
}

#[test]
fn image_workspace_is_a_supported_command_with_required_inputs() {
    let error = Cli::try_parse_from(["capsem-admin", "image", "workspace"]).expect_err("workspace inputs are required");

    assert_eq!(error.kind(), clap::error::ErrorKind::MissingRequiredArgument);
    let rendered = error.to_string();
    assert!(rendered.contains("--profile"), "{rendered}");
    assert!(rendered.contains("--output"), "{rendered}");
}

#[test]
fn image_build_workspaces_are_isolated_by_profile_and_architecture() {
    let profile = ProfileConfigFile::builtin_primary();

    let arm64 = image_build_workspace_path(&profile, Some("arm64"));
    let x86_64 = image_build_workspace_path(&profile, Some("x86_64"));

    assert_ne!(arm64, x86_64);
    assert_eq!(arm64, PathBuf::from("cache/target/image-workspace/code/arm64"));
    assert_eq!(x86_64, PathBuf::from("cache/target/image-workspace/code/x86_64"));
}

#[test]
fn image_build_rejects_dry_run_escape_hatch() {
    let error = Cli::try_parse_from([
        "capsem-admin",
        "image",
        "build",
        "--profile",
        "config/profiles/code/profile.toml",
        "--dry-run",
    ])
    .expect_err("dry-run is not a public product rail");

    assert!(error.to_string().contains("unexpected argument '--dry-run'"), "{error}");
}

#[test]
fn removed_admin_authoring_commands_are_not_parseable() {
    for argv in [
        ["capsem-admin", "profile", "init"],
        ["capsem-admin", "settings", "init"],
        ["capsem-admin", "enforcement", "compile"],
        ["capsem-admin", "detection", "compile"],
        ["capsem-admin", "manifest", "verify"],
        ["capsem-admin", "image", "plan"],
        ["capsem-admin", "image", "verify"],
    ] {
        let error = Cli::try_parse_from(argv).expect_err("removed command rejected");
        assert!(error.to_string().contains("unrecognized subcommand"), "{error}");
    }
}

#[test]
fn image_plan_is_profile_derived_and_uses_erofs_lz4hc() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().and_then(Path::parent).expect("repo root");
    let args = ImageBuildArgs {
        profile: repo_root.join("config/profiles/code/profile.toml"),
        config_root: repo_root.join("config"),
        guest_dir: repo_root.join("guest"),
        output: repo_root.join("assets"),
        arch: Some("arm64".to_string()),
        template: ImageBuildTemplate::All,
        clean: true,
        json: true,
    };

    let plan = image_build_plan(&args).expect("image plan");

    assert_eq!(plan.profile_id, "code");
    assert_eq!(plan.arches.len(), 1);
    assert_eq!(plan.arches[0].arch, "arm64");
    assert_eq!(plan.arches[0].rootfs, "rootfs.erofs");
    assert_eq!(plan.commands.len(), 3);
    assert_eq!(plan.commands[0].step, "kernel");
    assert_eq!(
        plan.commands[0].argv[0..5]
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["uv", "run", "python", "-m", "capsem_builder.image.image_build_backend",]
    );
    assert!(!plan.commands[0]
        .argv
        .windows(2)
        .any(|window| window[0] == "capsem-builder" && window[1] == "build"));
    assert_eq!(plan.commands[1].step, "rootfs");
    assert_eq!(
        plan.commands[1].argv[0..5]
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["uv", "run", "python", "-m", "capsem_builder.image.image_build_backend",]
    );
    assert!(!plan.commands[1]
        .argv
        .windows(2)
        .any(|window| window[0] == "capsem-builder" && window[1] == "build"));
    assert_eq!(
        plan.commands[1].env.get("CAPSEM_BUILD_EROFS_COMPRESSION"),
        Some(&"lz4hc".to_string())
    );
    assert_eq!(
        plan.commands[1].env.get("CAPSEM_BUILD_EROFS_COMPRESSION_LEVEL"),
        Some(&"12".to_string())
    );
    assert_eq!(plan.commands[2].step, "manifest");
}

#[test]
fn image_plan_kernel_only_does_not_generate_manifest() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().and_then(Path::parent).expect("repo root");
    let args = ImageBuildArgs {
        profile: repo_root.join("config/profiles/code/profile.toml"),
        config_root: repo_root.join("config"),
        guest_dir: repo_root.join("guest"),
        output: repo_root.join("assets"),
        arch: Some("arm64".to_string()),
        template: ImageBuildTemplate::Kernel,
        clean: true,
        json: true,
    };

    let plan = image_build_plan(&args).expect("image plan");

    assert_eq!(
        plan.commands
            .iter()
            .map(|command| command.step.as_str())
            .collect::<Vec<_>>(),
        vec!["kernel"]
    );
}

#[test]
fn image_clean_rootfs_preserves_kernel_and_initrd() {
    let temp = tempfile::tempdir().expect("tempdir");
    let arch_dir = temp.path().join("arm64");
    fs::create_dir_all(&arch_dir).expect("arch dir");
    fs::write(arch_dir.join("vmlinuz"), b"kernel").expect("kernel");
    fs::write(arch_dir.join("initrd.img"), b"initrd").expect("initrd");
    fs::write(arch_dir.join("rootfs.erofs"), b"rootfs").expect("rootfs");
    fs::write(arch_dir.join("obom.cdx.json"), b"obom").expect("obom");

    clean_image_outputs(&ImageBuildPlan {
        schema: "test",
        profile_id: "code".to_string(),
        profile_revision: "test".to_string(),
        guest_dir: "guest".to_string(),
        output: temp.path().display().to_string(),
        clean: true,
        template: "rootfs",
        arches: vec![ImageBuildArchPlan {
            arch: "arm64".to_string(),
            kernel: "vmlinuz".to_string(),
            initrd: "initrd.img".to_string(),
            rootfs: "rootfs.erofs".to_string(),
        }],
        commands: Vec::new(),
    })
    .expect("rootfs clean");

    assert!(arch_dir.join("vmlinuz").is_file());
    assert!(arch_dir.join("initrd.img").is_file());
    assert!(!arch_dir.join("rootfs.erofs").exists());
    assert!(!arch_dir.join("obom.cdx.json").exists());
}

#[test]
fn image_clean_kernel_preserves_rootfs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let arch_dir = temp.path().join("arm64");
    fs::create_dir_all(&arch_dir).expect("arch dir");
    fs::write(arch_dir.join("vmlinuz"), b"kernel").expect("kernel");
    fs::write(arch_dir.join("initrd.img"), b"initrd").expect("initrd");
    fs::write(arch_dir.join("rootfs.erofs"), b"rootfs").expect("rootfs");

    clean_image_outputs(&ImageBuildPlan {
        schema: "test",
        profile_id: "code".to_string(),
        profile_revision: "test".to_string(),
        guest_dir: "guest".to_string(),
        output: temp.path().display().to_string(),
        clean: true,
        template: "kernel",
        arches: vec![ImageBuildArchPlan {
            arch: "arm64".to_string(),
            kernel: "vmlinuz".to_string(),
            initrd: "initrd.img".to_string(),
            rootfs: "rootfs.erofs".to_string(),
        }],
        commands: Vec::new(),
    })
    .expect("kernel clean");

    assert!(!arch_dir.join("vmlinuz").exists());
    assert!(!arch_dir.join("initrd.img").exists());
    assert!(arch_dir.join("rootfs.erofs").is_file());
}

#[test]
fn image_plan_rejects_arch_missing_from_profile() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().and_then(Path::parent).expect("repo root");
    let args = ImageBuildArgs {
        profile: repo_root.join("config/profiles/code/profile.toml"),
        config_root: repo_root.join("config"),
        guest_dir: repo_root.join("guest"),
        output: repo_root.join("assets"),
        arch: Some("riscv64".to_string()),
        template: ImageBuildTemplate::All,
        clean: false,
        json: false,
    };

    let error = image_build_plan(&args).expect_err("unknown arch rejected");

    assert!(
        error.to_string().contains("does not define assets for arch riscv64"),
        "{error:#}"
    );
}

#[test]
fn image_workspace_materializes_self_contained_profile_config() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().and_then(Path::parent).expect("repo root");
    let temp = tempfile::tempdir().expect("tempdir");
    let args = ImageWorkspaceArgs {
        profile: repo_root.join("config/profiles/code/profile.toml"),
        config_root: repo_root.join("config"),
        guest_dir: repo_root.join("guest"),
        output: temp.path().join("workspace"),
        arch: Some("arm64".to_string()),
        json: true,
    };

    let report = materialize_image_workspace(&args).expect("workspace");

    assert_eq!(report.profile_id, "code");
    assert_eq!(report.arches.len(), 1);
    assert_eq!(report.arches[0].arch, "arm64");
    assert_eq!(report.rule_files.len(), 2);
    let workspace_profile = args.output.join("config/profiles/code/profile.toml");
    assert!(workspace_profile.is_file());
    assert!(args.output.join("config/profiles/code/enforcement.toml").is_file());
    assert!(args.output.join("config/profiles/code/detection.yaml").is_file());
    assert!(args.output.join("build-plan.json").is_file());
    assert!(args.output.join("workspace.json").is_file());
    let generated_config = args.output.join("guest").join("config");
    assert!(generated_config.join("packages/apt.toml").is_file());
    let source_build = repo_root.join("config/docker/image/build.toml");
    let generated_build = generated_config.join("build.toml");
    assert_eq!(
        fs::read(&generated_build).expect("materialized build config"),
        fs::read(&source_build).expect("source build config"),
        "the image workspace must copy the one authoritative build contract byte-for-byte"
    );
    let build_config: toml::Value =
        toml::from_str(&fs::read_to_string(&generated_build).expect("read materialized build config"))
            .expect("parse materialized build config");
    let kernel = build_config["build"]["kernel"]
        .as_table()
        .expect("one common kernel source table");
    assert!(kernel.get("version").is_some());
    assert!(kernel.get("sha256").is_some());
    let apt_packages =
        fs::read_to_string(generated_config.join("packages/apt.toml")).expect("materialized apt packages");
    assert!(
        apt_packages.contains("\"zstd\""),
        "Ollama's official installer consumes .tar.zst payloads, so shipped profiles must include zstd"
    );
    assert!(generated_config.join("packages/python.toml").is_file());
    assert!(generated_config.join("packages/python-requirements.lock").is_file());
    assert!(generated_config.join("packages/npm.toml").is_file());
    assert!(generated_config.join("packages/npm-package.json").is_file());
    assert!(generated_config.join("packages/npm-package-lock.json").is_file());
    let resources = fs::read_to_string(generated_config.join("vm/resources.toml")).expect("materialized VM resources");
    assert!(resources.contains("ram_gb = 12"));
    assert!(resources.contains("scratch_disk_size_gb = 64"));
    assert!(args.output.join("guest/profile-build.sh").is_file());
    let profile_build =
        fs::read_to_string(args.output.join("guest/profile-build.sh")).expect("materialized profile build script");
    assert!(profile_build.contains("CAPSEM_OLLAMA_SHA256"));
    assert!(!profile_build.contains("https://ollama.com/install.sh"));
    assert!(args.output.join("guest/profile-root/root/.codex/config.toml").is_file());
    assert!(args.output.join("guest/artifacts/tips.txt").is_file());
    let build_plan: serde_json::Value =
        serde_json::from_slice(&fs::read(args.output.join("build-plan.json")).unwrap()).unwrap();
    assert!(build_plan["commands"]
        .as_array()
        .unwrap()
        .iter()
        .any(|command| command["argv"]
            .as_array()
            .unwrap()
            .iter()
            .any(|arg| arg == args.output.join("guest").display().to_string().as_str())));

    let copied = check_profile(&ProfileCheckArgs {
        path: workspace_profile,
        config_root: Some(args.output.join("config")),
        arch: None,
        json: true,
    })
    .expect("copied workspace profile validates and owns every pinned payload");
    assert_eq!(copied.validation.profile_id, "code");
    assert!(copied.profile_files.iter().all(|file| file.present));
}

#[test]
fn image_workspace_removes_stale_profile_root_payloads_before_materializing() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().and_then(Path::parent).expect("repo root");
    let temp = tempfile::tempdir().expect("tempdir");
    let output = temp.path().join("workspace");
    let stale_profile_root = output.join("guest/profile-root/root/.gemini/config/config.json");
    fs::create_dir_all(stale_profile_root.parent().unwrap()).expect("stale parent");
    fs::write(
        &stale_profile_root,
        r#"{"ai":{"provider":"ollama","baseUrl":"http://127.0.0.1:11434"}}"#,
    )
    .expect("stale provider override");
    let stale_deleted_file = output.join("guest/profile-root/root/.stale-local-provider.json");
    fs::write(&stale_deleted_file, r#"{"provider":"ollama"}"#).expect("stale file");

    let args = ImageWorkspaceArgs {
        profile: repo_root.join("config/profiles/code/profile.toml"),
        config_root: repo_root.join("config"),
        guest_dir: repo_root.join("guest"),
        output,
        arch: Some("arm64".to_string()),
        json: true,
    };

    materialize_image_workspace(&args).expect("workspace");

    let materialized_config = fs::read_to_string(&stale_profile_root).expect("materialized AGY provider config");
    assert_eq!(materialized_config.trim(), "{}");
    assert!(
        !stale_deleted_file.exists(),
        "removed profile-root payloads must not survive into rebuilt image workspaces"
    );
}

#[test]
fn profile_materialize_writes_generated_config_from_manifest() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().and_then(Path::parent).expect("repo root");
    let temp = tempfile::tempdir().expect("tempdir");
    let assets_dir = temp.path().join("assets");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let output_root = temp.path().join("cache/target/config");
    let source_profile = repo_root.join("config/profiles/code/profile.toml");
    let original_source = fs::read_to_string(&source_profile).expect("read source profile");

    let report = materialize_profile_config(&ProfileMaterializeArgs {
        profile: source_profile.clone(),
        config_root: repo_root.join("config"),
        manifest: file_url(&manifest_path),
        assets_dir,
        output_root: output_root.clone(),
        arch: Some("arm64".to_string()),
        clean: true,
        json: true,
    })
    .expect("materialize profile config");

    assert_eq!(report.profile_id, "code");
    assert_eq!(report.materialized_assets.len(), 3);
    assert_eq!(report.materialized_obom.len(), 1);
    assert!(output_root.join("settings/settings.toml").is_file());
    assert!(output_root.join("corp/corp.toml").is_file());
    assert!(output_root.join("assets/manifest.json").is_file());
    assert!(output_root.join("profiles/code/enforcement.toml").is_file());
    assert!(output_root.join("profiles/code/detection.yaml").is_file());

    let generated_profile_path = output_root.join("profiles/code/profile.toml");
    let generated: ProfileConfigFile =
        toml::from_str(&fs::read_to_string(&generated_profile_path).expect("read generated"))
            .expect("parse generated profile");
    let arm64 = generated.assets.arch.get("arm64").expect("arm64 assets");
    assert!(arm64.kernel.url.starts_with("file://"));
    assert!(arm64.initrd.url.starts_with("file://"));
    assert!(arm64.rootfs.url.starts_with("file://"));
    assert_eq!(
        arm64.kernel.hash,
        Some(format!("blake3:{}", blake3::hash(b"kernel-arm64").to_hex()))
    );
    assert_eq!(arm64.initrd.size, Some(b"initrd-arm64".len() as u64));
    assert_eq!(arm64.rootfs.name, "rootfs.erofs");
    assert!(generated
        .files
        .iter()
        .all(|(_, descriptor)| descriptor.hash.is_some() && descriptor.size.is_some()));
    let obom = generated
        .obom
        .as_ref()
        .expect("materialized profile has base-image OBOM")
        .arch
        .get("arm64")
        .expect("arm64 OBOM");
    assert!(obom.url.starts_with("file://"));
    assert_eq!(
        obom.hash,
        format!("blake3:{}", blake3::hash(test_obom_json().as_bytes()).to_hex())
    );
    assert_eq!(obom.generator, "cdxgen");
    assert_eq!(obom.generator_version, "11.0.0");

    let validation =
        validate_materialized_profile(&generated_profile_path, Some(&output_root)).expect("valid materialized output");
    assert_eq!(validation.profile_id, "code");
    assert_eq!(
        fs::read_to_string(source_profile).expect("read source profile after"),
        original_source,
        "materialization must not mutate checked-in source profile"
    );
}

#[test]
fn profile_materialize_remote_manifest_derives_release_site_asset_urls() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().and_then(Path::parent).expect("repo root");
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let manifest_json = fs::read_to_string(&manifest_path).expect("manifest");
    let manifest_url = serve_manifest_once(manifest_json);
    let output_root = temp.path().join("cache/target/config");

    materialize_profile_config(&ProfileMaterializeArgs {
        profile: repo_root.join("config/profiles/code/profile.toml"),
        config_root: repo_root.join("config"),
        manifest: manifest_url.clone(),
        assets_dir: temp.path().join("no-local-assets"),
        output_root: output_root.clone(),
        arch: Some("arm64".to_string()),
        clean: true,
        json: true,
    })
    .expect("remote manifest materializes without local asset blobs");

    let generated_profile_path = output_root.join("profiles/code/profile.toml");
    let generated: ProfileConfigFile =
        toml::from_str(&fs::read_to_string(&generated_profile_path).expect("read generated"))
            .expect("parse generated profile");
    let arm64 = generated.assets.arch.get("arm64").expect("arm64 assets");
    let expected_base = manifest_url.replace("/assets/stable/manifest.json", "/assets/releases/2030.0101.1");
    assert_eq!(arm64.kernel.url, format!("{expected_base}/arm64-vmlinuz"));
    assert_eq!(arm64.initrd.url, format!("{expected_base}/arm64-initrd.img"));
    assert_eq!(arm64.rootfs.url, format!("{expected_base}/arm64-rootfs.erofs"));
    assert_eq!(
        arm64.kernel.hash,
        Some(format!("blake3:{}", blake3::hash(b"kernel-arm64").to_hex()))
    );
    let obom = generated
        .obom
        .as_ref()
        .expect("remote OBOM descriptor")
        .arch
        .get("arm64")
        .expect("arm64 OBOM");
    assert_eq!(obom.url, format!("{expected_base}/arm64-obom.cdx.json"));
    assert_eq!(obom.generator, "remote");
    assert_eq!(obom.generator_version, "unknown");
}

#[test]
fn profile_materialize_release_channel_manifest_uses_profile_image_urls() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().and_then(Path::parent).expect("repo root");
    let temp = tempfile::tempdir().expect("tempdir");
    let output_root = temp.path().join("cache/target/config");
    let local_obom = temp.path().join("resolved-obom.cdx.json");
    fs::write(&local_obom, test_obom_json()).expect("write resolved OBOM");
    let local_obom_url = file_url(&local_obom);
    let software_inventory = test_software_inventory_json("arm64");
    let local_software_inventory = temp.path().join("resolved-software-inventory.json");
    fs::write(&local_software_inventory, &software_inventory).expect("write resolved software inventory");
    let local_software_inventory_url = file_url(&local_software_inventory);
    let digest = |bytes: &[u8]| {
        serde_json::json!({
            "sha256": format!("{:x}", Sha256::digest(bytes)),
            "blake3": blake3::hash(bytes).to_hex().to_string(),
        })
    };
    let manifest_json = serde_json::to_string_pretty(&serde_json::json!({
        "version": "1.5.0+assets.2030.0101.1",
        "status": "current",
        "packages": [],
        "profiles": {
            "code": {
                "version": "2030.0101.1",
                "id": "code",
                "name": "Code",
                "revision": "2030.0101.1",
                "status": "current",
                "min_capsem_version": "1.5.0",
                "architectures": [
                    {
                        "architecture": "arm64",
                        "images": [
                            {
                                "kind": "kernel",
                                "name": "vmlinuz",
                                "url": "/profiles/releases/2030.0101.1/code/arm64/vmlinuz",
                                "bytes": b"kernel-arm64".len(),
                                "digest": digest(b"kernel-arm64"),
                                "status": "current"
                            },
                            {
                                "kind": "initrd",
                                "name": "initrd.img",
                                "url": "https://cdn.example.test/initrd.img",
                                "bytes": b"initrd-arm64".len(),
                                "digest": digest(b"initrd-arm64"),
                                "status": "current"
                            },
                            {
                                "kind": "rootfs",
                                "name": "rootfs.erofs",
                                "url": "/profiles/releases/2030.0101.1/code/arm64/rootfs.erofs",
                                "bytes": b"rootfs-arm64".len(),
                                "digest": digest(b"rootfs-arm64"),
                                "status": "current"
                            }
                        ],
                        "evidence": [
                            {
                                "kind": "obom",
                                "name": "obom.cdx.json",
                                "url": local_obom_url,
                                "bytes": test_obom_json().len(),
                                "digest": digest(test_obom_json().as_bytes()),
                                "status": "current"
                            },
                            {
                                "kind": "software_inventory",
                                "url": local_software_inventory_url,
                                "bytes": software_inventory.len(),
                                "digest": digest(software_inventory.as_bytes()),
                                "status": "current"
                            }
                        ]
                    }
                ]
            }
        }
    }))
    .expect("release channel manifest");
    let manifest_url = serve_manifest_once(manifest_json);

    materialize_profile_config(&ProfileMaterializeArgs {
        profile: repo_root.join("config/profiles/code/profile.toml"),
        config_root: repo_root.join("config"),
        manifest: manifest_url.clone(),
        assets_dir: temp.path().join("no-local-assets"),
        output_root: output_root.clone(),
        arch: Some("arm64".to_string()),
        clean: true,
        json: true,
    })
    .expect("release channel manifest materializes without local asset blobs");

    let generated_profile_path = output_root.join("profiles/code/profile.toml");
    let generated: ProfileConfigFile =
        toml::from_str(&fs::read_to_string(&generated_profile_path).expect("read generated"))
            .expect("parse generated profile");
    let arm64 = generated.assets.arch.get("arm64").expect("arm64 assets");
    let expected_origin = manifest_url.replace("/assets/stable/manifest.json", "");
    assert_eq!(
        arm64.kernel.url,
        format!("{expected_origin}/profiles/releases/2030.0101.1/code/arm64/vmlinuz")
    );
    assert_eq!(arm64.initrd.url, "https://cdn.example.test/initrd.img");
    assert_eq!(
        arm64.rootfs.url,
        format!("{expected_origin}/profiles/releases/2030.0101.1/code/arm64/rootfs.erofs")
    );
    assert_eq!(
        arm64.rootfs.hash,
        Some(format!("blake3:{}", blake3::hash(b"rootfs-arm64").to_hex()))
    );

    let obom = generated
        .obom
        .as_ref()
        .expect("release channel OBOM descriptor")
        .arch
        .get("arm64")
        .expect("arm64 OBOM");
    assert_eq!(obom.url, local_obom_url);
    assert_eq!(obom.generator, "cdxgen");
    assert_eq!(obom.generator_version, "11.0.0");

    let converted_manifest_path = output_root.join("assets/manifest.json");
    let converted =
        ManifestV2::from_json(&fs::read_to_string(&converted_manifest_path).expect("read converted manifest"))
            .expect("converted release channel manifest is raw v2");
    assert_eq!(converted.format, 2);
    assert_eq!(converted.assets.current, "2030.0101.1");
    // The graph profile's `min_capsem_version` reaches the runtime manifest as
    // `min_binary`. Dropped, it broke the release lane's whole glow-up: that
    // step hands its paired runtime manifest back to `assets channel build`,
    // which copies `min_binary` onto every graph profile as
    // `min_capsem_version`, and `record-binary` refuses an empty semver.
    assert_eq!(
        converted.assets.releases["2030.0101.1"].min_binary, "1.5.0",
        "the graph profile declares a binary floor and the runtime manifest \
         has to carry it, or re-authoring a channel names no floor at all"
    );
    let converted_assets = converted.assets.releases["2030.0101.1"]
        .arches
        .get("arm64")
        .expect("converted arm64 assets");
    assert!(converted_assets.contains_key("obom.cdx.json"));
    assert!(converted_assets.contains_key("software-inventory.json"));
    assert_eq!(
        converted_assets["software-inventory.json"].sha256,
        format!("{:x}", Sha256::digest(software_inventory.as_bytes()))
    );
    assert_eq!(
        converted_assets["initrd.img"].sha256,
        format!("{:x}", Sha256::digest(b"initrd-arm64"))
    );
}

#[test]
fn profile_materialize_preserves_previous_profiles_in_same_output_catalog() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().and_then(Path::parent).expect("repo root");
    let temp = tempfile::tempdir().expect("tempdir");
    let assets_dir = temp.path().join("assets");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");
    let output_root = temp.path().join("cache/target/config");
    let config_root = repo_root.join("config");

    materialize_profile_config(&ProfileMaterializeArgs {
        profile: config_root.join("profiles/co-work/profile.toml"),
        config_root: config_root.clone(),
        manifest: file_url(&manifest_path),
        assets_dir: assets_dir.clone(),
        output_root: output_root.clone(),
        arch: Some("arm64".to_string()),
        clean: true,
        json: true,
    })
    .expect("materialize co-work");

    materialize_profile_config(&ProfileMaterializeArgs {
        profile: config_root.join("profiles/code/profile.toml"),
        config_root,
        manifest: file_url(&manifest_path),
        assets_dir,
        output_root: output_root.clone(),
        arch: Some("arm64".to_string()),
        clean: false,
        json: true,
    })
    .expect("materialize code");

    for profile_id in ["co-work", "code"] {
        let generated_profile_path = output_root.join("profiles").join(profile_id).join("profile.toml");
        let generated: ProfileConfigFile =
            toml::from_str(&fs::read_to_string(&generated_profile_path).expect("read generated profile"))
                .expect("generated profile parses");
        let arm64 = generated.assets.arch.get("arm64").expect("arm64 assets");
        assert_eq!(
            arm64.kernel.hash,
            Some(format!("blake3:{}", blake3::hash(b"kernel-arm64").to_hex())),
            "{profile_id} kernel pin must remain generated"
        );
        assert_eq!(
            arm64.initrd.hash,
            Some(format!("blake3:{}", blake3::hash(b"initrd-arm64").to_hex())),
            "{profile_id} initrd pin must remain generated"
        );
        assert_eq!(
            arm64.rootfs.hash,
            Some(format!("blake3:{}", blake3::hash(b"rootfs-arm64").to_hex())),
            "{profile_id} rootfs pin must remain generated"
        );
        assert!(arm64.kernel.url.starts_with("file://"));
        assert!(arm64.initrd.url.starts_with("file://"));
        assert!(arm64.rootfs.url.starts_with("file://"));
    }
}

#[test]
fn profile_materialize_rejects_arch_missing_from_manifest() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().and_then(Path::parent).expect("repo root");
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");

    let error = materialize_profile_config(&ProfileMaterializeArgs {
        profile: repo_root.join("config/profiles/code/profile.toml"),
        config_root: repo_root.join("config"),
        manifest: file_url(&manifest_path),
        assets_dir: temp.path().join("assets"),
        output_root: temp.path().join("cache/target/config"),
        arch: Some("x86_64".to_string()),
        clean: true,
        json: false,
    })
    .expect_err("missing manifest arch rejected");

    assert!(
        format!("{error:#}").contains("does not contain profile arch x86_64"),
        "{error:#}"
    );
}

#[test]
fn profile_materialize_manifest_source_must_be_url() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().and_then(Path::parent).expect("repo root");
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_test_assets_manifest(temp.path(), "arm64");

    let error = materialize_profile_config(&ProfileMaterializeArgs {
        profile: repo_root.join("config/profiles/code/profile.toml"),
        config_root: repo_root.join("config"),
        manifest: manifest_path.display().to_string(),
        assets_dir: temp.path().join("assets"),
        output_root: temp.path().join("cache/target/config"),
        arch: Some("arm64".to_string()),
        clean: true,
        json: false,
    })
    .expect_err("bare manifest path rejected");

    assert!(format!("{error:#}").contains("manifest must be a URL"), "{error:#}");
}
