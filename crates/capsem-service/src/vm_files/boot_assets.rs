use super::*;
use capsem_assets::asset_manager::{host_manifest_arch, load_manifest_for_assets, ResolvedAssets};

impl ServiceState {
    pub(crate) fn validate_pinned_asset_files(&self, resolved: &ResolvedAssets, pins: &BootAssetPins) -> Result<()> {
        let files = [
            ("kernel", &resolved.kernel, &pins.kernel),
            ("initrd", &resolved.initrd, &pins.initrd),
            ("rootfs", &resolved.rootfs, &pins.rootfs),
        ];
        // Only development's mutable plain names need the selected manifest's
        // binding. Retained digest names remain independent of runtime advances.
        // The existing resume cache fingerprints the manifest and file metadata;
        // no polled read hashes boot images. Process boot still verifies bytes.
        let expected = if files
            .iter()
            .any(|(_, path, pin)| path.file_name() == Some(pin.name.as_ref()))
        {
            load_manifest_for_assets(&self.assets_dir).and_then(|manifest| {
                let arch = host_manifest_arch();
                let selected = manifest.resolve(&self.current_version, arch, &self.assets_dir).ok()?;
                manifest.expected_hashes(&selected.asset_version, arch)
            })
        } else {
            None
        };
        for (kind, path, pin) in files {
            let metadata = std::fs::metadata(path)
                .with_context(|| format!("{kind} asset '{}' is missing at {}", pin.name, path.display()))?;
            if !metadata.is_file() {
                return Err(anyhow!("{kind} asset '{}' is not a file", pin.name));
            }
            if path.file_name() == Some(pin.name.as_ref()) {
                let binding = expected.as_ref().map(|hashes| match kind {
                    "kernel" => hashes.kernel.as_str(),
                    "initrd" => hashes.initrd.as_str(),
                    _ => hashes.rootfs.as_str(),
                });
                if binding.is_none_or(|hash| {
                    hash.strip_prefix("blake3:").unwrap_or(hash)
                        != pin.hash.strip_prefix("blake3:").unwrap_or(&pin.hash)
                }) {
                    return Err(anyhow!(
                        "{kind} asset '{}' does not match its recorded pin in the selected manifest",
                        pin.name
                    ));
                }
            }
        }
        Ok(())
    }
}
