//! Scoped asset manifest generation through the image builder.

use super::*;

#[derive(Debug, Parser)]
pub(super) struct ManifestGenerateArgs {
    /// Asset directory containing built per-arch assets.
    #[arg(default_value = "assets")]
    pub(super) assets_dir: PathBuf,
    /// Binary version to record. Defaults to capsem-builder's project version.
    #[arg(long)]
    pub(super) version: Option<String>,
    /// Restrict the manifest to these architecture directories (repeatable).
    #[arg(long = "arch", value_parser = ["arm64", "x86_64"])]
    pub(super) arches: Vec<String>,
    /// Emit the generated manifest after writing it.
    #[arg(long)]
    pub(super) json: bool,
}

pub(super) fn manifest_generate_command_report(args: &ManifestGenerateArgs) -> CommandReport {
    let version_expr = match &args.version {
        Some(version) => format!("{version:?}"),
        None => "get_project_version(Path('.'))".to_string(),
    };
    let selection = if args.arches.is_empty() {
        "None".to_owned()
    } else {
        format!("{:?}", args.arches)
    };
    CommandReport {
        step: "manifest".to_string(),
        arch: None,
        env: BTreeMap::new(),
        argv: builder_python_command([
            "-c".to_string(),
            format!(
                "from pathlib import Path; from capsem_builder.image.docker import generate_checksums, get_project_version; v = {version_expr}; generate_checksums(Path({:?}), v, arches={selection}); print(f'manifest.json generated (v{{v}})')",
                args.assets_dir.display().to_string()
            ),
        ]),
    }
}
