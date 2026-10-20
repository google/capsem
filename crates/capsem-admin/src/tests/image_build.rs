use super::*;

fn repo_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("repo root")
        .to_path_buf()
}

fn runtime_build_args(arch: Option<&str>, template: ImageBuildTemplate) -> ImageBuildArgs {
    let repo_root = repo_root();
    ImageBuildArgs {
        config_root: repo_root.join("config"),
        guest_dir: repo_root.join("guest"),
        output: repo_root.join("assets"),
        arch: arch.map(ToOwned::to_owned),
        template,
        clean: true,
        json: true,
    }
}

#[test]
fn image_build_runs_from_defaults_alone() {
    Cli::try_parse_from(["capsem-admin", "image", "build"]).expect("the runtime builds from defaults alone");
}

#[test]
fn image_workspace_requires_only_an_output() {
    let error = Cli::try_parse_from(["capsem-admin", "image", "workspace"]).expect_err("workspace output is required");

    assert_eq!(error.kind(), clap::error::ErrorKind::MissingRequiredArgument);
    let rendered = error.to_string();
    assert!(rendered.contains("--output"), "{rendered}");
}

#[test]
fn image_build_rejects_an_architecture_the_runtime_is_not_built_for() {
    let error = Cli::try_parse_from(["capsem-admin", "image", "build", "--arch", "riscv64"])
        .expect_err("unknown arch rejected at the argv boundary");
    assert!(error.to_string().contains("riscv64"), "{error}");

    let error = image_build_plan(&runtime_build_args(Some("riscv64"), ImageBuildTemplate::All))
        .expect_err("unknown arch rejected by the plan");
    assert!(
        error.to_string().contains("the runtime is not built for arch riscv64"),
        "{error:#}"
    );
}

#[test]
fn image_build_workspaces_are_isolated_by_architecture() {
    assert_eq!(
        image_build_workspace_path(Some("arm64")),
        PathBuf::from("cache/target/build/image-workspace/arm64")
    );
    assert_eq!(
        image_build_workspace_path(Some("x86_64")),
        PathBuf::from("cache/target/build/image-workspace/x86_64")
    );
    assert_eq!(
        image_build_workspace_path(None),
        PathBuf::from("cache/target/build/image-workspace/all")
    );
}

#[test]
fn image_build_rejects_dry_run_escape_hatch() {
    let error = Cli::try_parse_from(["capsem-admin", "image", "build", "--dry-run"])
        .expect_err("dry-run is not a public product rail");

    assert!(error.to_string().contains("unexpected argument '--dry-run'"), "{error}");
}

#[test]
fn removed_admin_authoring_commands_are_not_parseable() {
    for argv in [
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
fn image_plan_builds_every_runtime_architecture_by_default() {
    let plan = image_build_plan(&runtime_build_args(None, ImageBuildTemplate::All)).expect("image plan");

    assert_eq!(
        plan.arches.iter().map(|arch| arch.arch.as_str()).collect::<Vec<_>>(),
        RUNTIME_ARCHES
    );
    assert!(plan
        .commands
        .last()
        .unwrap()
        .argv
        .last()
        .unwrap()
        .contains("arches=[\"arm64\", \"x86_64\"]"));
}

#[test]
fn image_plan_uses_the_runtime_assets_and_erofs_lz4hc() {
    let plan = image_build_plan(&runtime_build_args(Some("arm64"), ImageBuildTemplate::All)).expect("image plan");

    assert_eq!(plan.arches.len(), 1);
    assert_eq!(plan.arches[0].arch, "arm64");
    assert_eq!(plan.arches[0].kernel, "vmlinuz");
    assert_eq!(plan.arches[0].initrd, "initrd.img");
    assert_eq!(plan.arches[0].rootfs, "rootfs.erofs");
    assert_eq!(plan.commands.len(), 3);
    for command in &plan.commands {
        assert_eq!(
            &command.argv[..5],
            ["uv", "run", "--project", "build_system", "--frozen"],
            "{} must use the locked builder project even without an activated environment",
            command.step
        );
    }
    for (index, step) in [(0, "kernel"), (1, "rootfs")] {
        assert_eq!(plan.commands[index].step, step);
        assert_eq!(
            plan.commands[index].argv[5..8]
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["python", "-m", "capsem_builder.image.image_build_backend"]
        );
    }
    assert_eq!(
        plan.commands[1].env.get("CAPSEM_BUILD_EROFS_COMPRESSION"),
        Some(&"lz4hc".to_string())
    );
    assert_eq!(
        plan.commands[1].env.get("CAPSEM_BUILD_EROFS_COMPRESSION_LEVEL"),
        Some(&"12".to_string())
    );
    assert_eq!(plan.commands[2].step, "manifest");
    assert!(plan.commands[2].argv.last().unwrap().contains("arches=[\"arm64\"]"));
}

#[test]
fn image_plan_kernel_only_does_not_generate_manifest() {
    let plan = image_build_plan(&runtime_build_args(Some("arm64"), ImageBuildTemplate::Kernel)).expect("image plan");

    assert_eq!(
        plan.commands
            .iter()
            .map(|command| command.step.as_str())
            .collect::<Vec<_>>(),
        vec!["kernel"]
    );
}

fn runtime_plan(output: &Path, template: &'static str) -> ImageBuildPlan {
    ImageBuildPlan {
        schema: "test",
        guest_dir: "guest".to_string(),
        output: output.display().to_string(),
        clean: true,
        template,
        arches: vec![ImageBuildArchPlan {
            arch: "arm64".to_string(),
            kernel: "vmlinuz".to_string(),
            initrd: "initrd.img".to_string(),
            rootfs: "rootfs.erofs".to_string(),
        }],
        commands: Vec::new(),
    }
}

fn write_arch_outputs(arch_dir: &Path) {
    fs::create_dir_all(arch_dir).expect("arch dir");
    fs::write(arch_dir.join("vmlinuz"), b"kernel").expect("kernel");
    fs::write(arch_dir.join("initrd.img"), b"initrd").expect("initrd");
    fs::write(arch_dir.join("rootfs.erofs"), b"rootfs").expect("rootfs");
    fs::write(arch_dir.join("obom.cdx.json"), b"obom").expect("obom");
}

#[test]
fn image_clean_rootfs_preserves_kernel_and_initrd() {
    let temp = tempfile::tempdir().expect("tempdir");
    let arch_dir = temp.path().join("arm64");
    write_arch_outputs(&arch_dir);

    clean_image_outputs(&runtime_plan(temp.path(), "rootfs")).expect("rootfs clean");

    assert!(arch_dir.join("vmlinuz").is_file());
    assert!(arch_dir.join("initrd.img").is_file());
    assert!(!arch_dir.join("rootfs.erofs").exists());
    assert!(!arch_dir.join("obom.cdx.json").exists());
}

#[test]
fn image_clean_kernel_preserves_rootfs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let arch_dir = temp.path().join("arm64");
    write_arch_outputs(&arch_dir);

    clean_image_outputs(&runtime_plan(temp.path(), "kernel")).expect("kernel clean");

    assert!(!arch_dir.join("vmlinuz").exists());
    assert!(!arch_dir.join("initrd.img").exists());
    assert!(arch_dir.join("rootfs.erofs").is_file());
}

#[test]
fn image_workspace_copies_only_the_build_contract_and_guest_artifacts() {
    let repo_root = repo_root();
    let temp = tempfile::tempdir().expect("tempdir");
    let output = temp.path().join("workspace");
    let stale = output.join("guest/stale/seed.json");
    fs::create_dir_all(stale.parent().unwrap()).expect("stale parent");
    fs::write(&stale, "{}").expect("stale seed");
    let args = ImageWorkspaceArgs {
        config_root: repo_root.join("config"),
        guest_dir: repo_root.join("guest"),
        output: output.clone(),
        arch: Some("arm64".to_string()),
        json: true,
    };

    let report = materialize_image_workspace(&args).expect("workspace");

    assert_eq!(report.arches.len(), 1);
    assert_eq!(report.arches[0].arch, "arm64");
    assert!(output.join("build-plan.json").is_file());
    assert!(output.join("workspace.json").is_file());
    let generated_config = output.join("guest").join("config");
    let source_build = repo_root.join("config/docker/image/build.toml");
    assert_eq!(
        fs::read(generated_config.join("build.toml")).expect("materialized build config"),
        fs::read(&source_build).expect("source build config"),
        "the image workspace must copy the one authoritative build contract byte-for-byte"
    );
    assert_eq!(
        fs::read(output.join("guest/artifacts/tips.txt")).expect("materialized tips"),
        fs::read(repo_root.join("guest/artifacts/tips.txt")).expect("source tips"),
        "the guest tips file is the only tips file"
    );
    for gone in [
        "guest/stale",
        "guest/config/packages",
        "guest/config/vm/resources.toml",
        "config",
    ] {
        assert!(
            !output.join(gone).exists(),
            "{gone} is not a runtime build input and must not exist"
        );
    }
}
