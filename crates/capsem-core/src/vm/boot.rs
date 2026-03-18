use std::path::Path;

use anyhow::{Context, Result};
use objc2::AllocAnyThread;
use objc2_foundation::{NSString, NSURL};
use objc2_virtualization::VZLinuxBootLoader;
use tracing::debug_span;

use super::config::VmConfig;

/// Create a VZLinuxBootLoader from a VmConfig.
///
/// # Safety
/// Calls Objective-C APIs which are inherently unsafe.
pub fn create_boot_loader(config: &VmConfig) -> Result<objc2::rc::Retained<VZLinuxBootLoader>> {
    let _span = debug_span!("create_boot_loader").entered();
    unsafe {
        let kernel_url = nsurl_from_path(&config.kernel_path)
            .context("failed to create kernel URL")?;

        let boot_loader = VZLinuxBootLoader::initWithKernelURL(
            VZLinuxBootLoader::alloc(),
            &kernel_url,
        );

        let cmdline = NSString::from_str(&config.kernel_cmdline);
        boot_loader.setCommandLine(&cmdline);

        if let Some(ref initrd_path) = config.initrd_path {
            let initrd_url = nsurl_from_path(initrd_path)
                .context("failed to create initrd URL")?;
            boot_loader.setInitialRamdiskURL(Some(&initrd_url));
        }

        Ok(boot_loader)
    }
}

fn nsurl_from_path(path: &Path) -> Result<objc2::rc::Retained<NSURL>> {
    let path_str = path
        .to_str()
        .context("path is not valid UTF-8")?;
    let ns_path = NSString::from_str(path_str);
    Ok(NSURL::fileURLWithPath(&ns_path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_file(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("capsem-test-boot");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(b"fake kernel").unwrap();
        path
    }

    #[test]
    fn creates_boot_loader_with_kernel_only() {
        let kernel = temp_file("vmlinuz-boot-test");
        let config = VmConfig::builder()
            .kernel_path(&kernel)
            .build()
            .unwrap();
        let loader = create_boot_loader(&config).unwrap();

        let cmdline = unsafe { loader.commandLine() };
        assert_eq!(cmdline.to_string(), "console=hvc0 root=/dev/vda ro init_on_alloc=1 slab_nomerge page_alloc.shuffle=1");
        let initrd = unsafe { loader.initialRamdiskURL() };
        assert!(initrd.is_none());
    }

    #[test]
    fn creates_boot_loader_with_custom_cmdline() {
        let kernel = temp_file("vmlinuz-boot-cmd");
        let config = VmConfig::builder()
            .kernel_path(&kernel)
            .kernel_cmdline("console=ttyS0 debug")
            .build()
            .unwrap();
        let loader = create_boot_loader(&config).unwrap();

        let cmdline = unsafe { loader.commandLine() };
        assert_eq!(cmdline.to_string(), "console=ttyS0 debug");
    }

    #[test]
    fn creates_boot_loader_with_initrd() {
        let kernel = temp_file("vmlinuz-boot-initrd");
        let initrd = temp_file("initrd-boot-test.img");
        let config = VmConfig::builder()
            .kernel_path(&kernel)
            .initrd_path(&initrd)
            .build()
            .unwrap();
        let loader = create_boot_loader(&config).unwrap();

        let initrd_url = unsafe { loader.initialRamdiskURL() };
        assert!(initrd_url.is_some());
    }

    #[test]
    fn nsurl_from_valid_path() {
        let url = nsurl_from_path(Path::new("/tmp/test.txt")).unwrap();
        let path = url.path();
        assert!(path.is_some());
        assert_eq!(path.unwrap().to_string(), "/tmp/test.txt");
    }
}
