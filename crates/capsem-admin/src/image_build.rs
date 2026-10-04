//! The one VM runtime image: kernel, initrd and rootfs, per architecture.
//!
//! There is no profile input. Applications come from OCI images; the runtime
//! is built from `config/docker/image` and the guest artifacts alone, so every
//! build of a commit produces the same asset set.

use super::*;

/// The architectures the runtime is built for, in build order.
pub(super) const RUNTIME_ARCHES: [&str; 2] = ["arm64", "x86_64"];

const KERNEL_ASSET: &str = "vmlinuz";
const INITRD_ASSET: &str = "initrd.img";
const ROOTFS_ASSET: &str = "rootfs.erofs";

pub(super) fn image_build_workspace_path(arch: Option<&str>) -> PathBuf {
    PathBuf::from("cache/target/build")
        .join("image-workspace")
        .join(arch.unwrap_or("all"))
}

pub(super) fn image_build_command(args: ImageBuildArgs) -> Result<()> {
    let workspace_report = materialize_image_workspace(&ImageWorkspaceArgs {
        config_root: args.config_root.clone(),
        guest_dir: args.guest_dir.clone(),
        output: image_build_workspace_path(args.arch.as_deref()),
        arch: args.arch.clone(),
        json: true,
    })?;
    let plan = image_build_plan(&ImageBuildArgs {
        config_root: args.config_root.clone(),
        guest_dir: PathBuf::from(&workspace_report.workspace).join("guest"),
        output: args.output.clone(),
        arch: args.arch.clone(),
        template: args.template,
        clean: args.clean,
        json: args.json,
    })?;
    if plan.clean {
        clean_image_outputs(&plan)?;
    }
    for command in &plan.commands {
        run_command(command)?;
    }
    print_image_build_plan(&plan, args.json)?;
    Ok(())
}

pub(super) fn image_workspace_command(args: ImageWorkspaceArgs) -> Result<()> {
    let json = args.json;
    let report = materialize_image_workspace(&args)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("runtime image workspace -> {}", report.workspace);
    }
    Ok(())
}

fn selected_arches(only_arch: Option<&str>) -> Result<Vec<String>> {
    match only_arch {
        Some(arch) if RUNTIME_ARCHES.contains(&arch) => Ok(vec![arch.to_string()]),
        Some(arch) => Err(anyhow!(
            "the runtime is not built for arch {arch}; expected one of {}",
            RUNTIME_ARCHES.join(", ")
        )),
        None => Ok(RUNTIME_ARCHES.iter().map(|arch| arch.to_string()).collect()),
    }
}

fn backend_command(guest_dir: &Path, arch: &str, template: &str, output: &Path) -> Vec<String> {
    builder_python_command([
        "-m".to_string(),
        "capsem_builder.image.image_build_backend".to_string(),
        guest_dir.display().to_string(),
        "--arch".to_string(),
        arch.to_string(),
        "--template".to_string(),
        template.to_string(),
        "--output".to_string(),
        format!("{}/", output.display()),
    ])
}

pub(super) fn image_build_plan(args: &ImageBuildArgs) -> Result<ImageBuildPlan> {
    let arches = selected_arches(args.arch.as_deref())?;
    let mut arch_plans = Vec::new();
    let mut commands = Vec::new();
    for arch in &arches {
        arch_plans.push(ImageBuildArchPlan {
            arch: arch.clone(),
            kernel: KERNEL_ASSET.to_string(),
            initrd: INITRD_ASSET.to_string(),
            rootfs: ROOTFS_ASSET.to_string(),
        });
        if matches!(args.template, ImageBuildTemplate::All | ImageBuildTemplate::Kernel) {
            commands.push(CommandReport {
                step: "kernel".to_string(),
                arch: Some(arch.clone()),
                env: BTreeMap::new(),
                argv: backend_command(&args.guest_dir, arch, "kernel", &args.output),
            });
        }
        if matches!(args.template, ImageBuildTemplate::All | ImageBuildTemplate::Rootfs) {
            let env = BTreeMap::from([
                ("CAPSEM_BUILD_EXPERIMENTAL_EROFS".to_string(), "1".to_string()),
                ("CAPSEM_BUILD_EROFS_COMPRESSION".to_string(), "lz4hc".to_string()),
                ("CAPSEM_BUILD_EROFS_COMPRESSION_LEVEL".to_string(), "12".to_string()),
            ]);
            commands.push(CommandReport {
                step: "rootfs".to_string(),
                arch: Some(arch.clone()),
                env,
                argv: backend_command(&args.guest_dir, arch, "rootfs", &args.output),
            });
        }
    }
    if !matches!(args.template, ImageBuildTemplate::Kernel) {
        commands.push(manifest_generate_command_report(&ManifestGenerateArgs {
            assets_dir: args.output.clone(),
            arches,
            version: None,
            json: false,
        }));
    }

    Ok(ImageBuildPlan {
        schema: "capsem.admin.image_build_plan.v2",
        guest_dir: args.guest_dir.display().to_string(),
        output: args.output.display().to_string(),
        clean: args.clean,
        template: match args.template {
            ImageBuildTemplate::All => "all",
            ImageBuildTemplate::Kernel => "kernel",
            ImageBuildTemplate::Rootfs => "rootfs",
        },
        arches: arch_plans,
        commands,
    })
}

/// Copy the image build contract and guest artifacts into a private tree.
///
/// The backend reads only this tree, so a build cannot pick up anything the
/// workspace did not copy: `config/docker/image` and `guest/artifacts`.
pub(super) fn materialize_image_workspace(args: &ImageWorkspaceArgs) -> Result<ImageWorkspaceReport> {
    let arches = selected_arches(args.arch.as_deref())?;
    let workspace = &args.output;
    if workspace.exists() {
        fs::remove_dir_all(workspace)
            .with_context(|| format!("remove stale image workspace {}", workspace.display()))?;
    }
    let workspace_guest_dir = workspace.join("guest");
    let source_config = args.config_root.join("docker").join("image");
    let workspace_config = workspace_guest_dir.join("config");
    fs::create_dir_all(&workspace_config).with_context(|| format!("create {}", workspace_config.display()))?;
    for relative in ["build.toml", "manifest.toml"] {
        let source = source_config.join(relative);
        let destination = workspace_config.join(relative);
        fs::copy(&source, &destination)
            .with_context(|| format!("copy {} to {}", source.display(), destination.display()))?;
    }
    for directory in ["kernel", "security", "vm"] {
        copy_dir_recursive(&source_config.join(directory), &workspace_config.join(directory))?;
    }
    copy_dir_recursive(
        &args.guest_dir.join("artifacts"),
        &workspace_guest_dir.join("artifacts"),
    )?;

    let plan = image_build_plan(&ImageBuildArgs {
        config_root: args.config_root.clone(),
        guest_dir: workspace_guest_dir,
        output: workspace.join("assets"),
        arch: args.arch.clone(),
        template: ImageBuildTemplate::All,
        clean: false,
        json: true,
    })?;
    let build_plan_path = workspace.join("build-plan.json");
    fs::write(&build_plan_path, serde_json::to_vec_pretty(&plan)?)
        .with_context(|| format!("write {}", build_plan_path.display()))?;

    let report = ImageWorkspaceReport {
        schema: "capsem.admin.image_workspace.v2",
        ok: true,
        workspace: workspace.display().to_string(),
        build_plan_path: build_plan_path.display().to_string(),
        arches: plan
            .arches
            .into_iter()
            .filter(|arch| arches.iter().any(|selected| selected == &arch.arch))
            .collect(),
    };
    fs::write(workspace.join("workspace.json"), serde_json::to_vec_pretty(&report)?)
        .with_context(|| format!("write {}", workspace.join("workspace.json").display()))?;
    Ok(report)
}

pub(super) fn copy_dir_recursive(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination).with_context(|| format!("create {}", destination.display()))?;
    for entry in fs::read_dir(source).with_context(|| format!("read {}", source.display()))? {
        let entry = entry.with_context(|| format!("read entry in {}", source.display()))?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let file_type = entry
            .file_type()
            .with_context(|| format!("stat {}", source_path.display()))?;
        if file_type.is_dir() {
            copy_dir_recursive(&source_path, &destination_path)?;
        } else if file_type.is_file() {
            if let Some(parent) = destination_path.parent() {
                fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
            }
            fs::copy(&source_path, &destination_path)
                .with_context(|| format!("copy {} to {}", source_path.display(), destination_path.display()))?;
        }
    }
    Ok(())
}

pub(super) fn builder_python_command(arguments: impl IntoIterator<Item = String>) -> Vec<String> {
    ["uv", "run", "--project", "build_system", "--frozen", "python"]
        .into_iter()
        .map(str::to_owned)
        .chain(arguments)
        .collect()
}

pub(super) fn print_image_build_plan(plan: &ImageBuildPlan, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(plan)?);
        return Ok(());
    }
    println!("runtime image -> {}", plan.output);
    for arch in &plan.arches {
        println!("  {}: {}, {}, {}", arch.arch, arch.kernel, arch.initrd, arch.rootfs);
    }
    for command in &plan.commands {
        let env = command
            .env
            .iter()
            .flat_map(|(key, value)| [key.as_str(), "=", value.as_str(), " "])
            .collect::<String>();
        println!("  {}{}", env, command.argv.join(" "));
    }
    Ok(())
}

pub(super) fn clean_image_outputs(plan: &ImageBuildPlan) -> Result<()> {
    let output = PathBuf::from(&plan.output);
    for arch in &plan.arches {
        let path = output.join(&arch.arch);
        if !path.exists() {
            continue;
        }
        let files: &[&str] = match plan.template {
            "all" => {
                fs::remove_dir_all(&path).with_context(|| format!("remove {}", path.display()))?;
                continue;
            }
            "kernel" => &[KERNEL_ASSET, INITRD_ASSET],
            "rootfs" => &[
                ROOTFS_ASSET,
                "rootfs.squashfs",
                "obom.cdx.json",
                "software-inventory.json",
                "build-ledger.log",
                "tool-versions.txt",
            ],
            other => return Err(anyhow!("unsupported image build template {other}")),
        };
        for name in files {
            let file = path.join(name);
            if file.exists() {
                fs::remove_file(&file).with_context(|| format!("remove {}", file.display()))?;
            }
        }
    }
    if plan.arches.len() > 1 {
        for name in ["manifest.json", "B3SUMS"] {
            let path = output.join(name);
            if path.exists() {
                fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
            }
        }
    }
    Ok(())
}

pub(super) fn run_command(command: &CommandReport) -> Result<()> {
    let (program, args) = command
        .argv
        .split_first()
        .ok_or_else(|| anyhow!("empty command for step {}", command.step))?;
    let status = Command::new(program)
        .args(args)
        .envs(&command.env)
        .stdin(Stdio::null())
        .status()
        .with_context(|| format!("run image build step {}", command.step))?;
    if !status.success() {
        return Err(anyhow!("image build step {} failed with status {status}", command.step));
    }
    Ok(())
}
