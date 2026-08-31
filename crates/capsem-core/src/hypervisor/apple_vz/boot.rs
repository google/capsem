use std::path::Path;

use anyhow::{Context, Result};
use objc2::AllocAnyThread;
use objc2_foundation::{NSString, NSURL};
use objc2_virtualization::VZLinuxBootLoader;
use tracing::debug_span;

use crate::vm::config::VmConfig;

/// Create a VZLinuxBootLoader from a VmConfig.
///
/// # Safety
/// Calls Objective-C APIs which are inherently unsafe.
pub fn create_boot_loader(config: &VmConfig) -> Result<objc2::rc::Retained<VZLinuxBootLoader>> {
    let _span = debug_span!("create_boot_loader").entered();
    unsafe {
        let kernel_url = nsurl_from_path(&config.kernel_path).context("failed to create kernel URL")?;

        let boot_loader = VZLinuxBootLoader::initWithKernelURL(VZLinuxBootLoader::alloc(), &kernel_url);

        let cmdline = NSString::from_str(&config.kernel_cmdline);
        boot_loader.setCommandLine(&cmdline);

        if let Some(ref initrd_path) = config.initrd_path {
            let initrd_url = nsurl_from_path(initrd_path).context("failed to create initrd URL")?;
            boot_loader.setInitialRamdiskURL(Some(&initrd_url));
        }

        Ok(boot_loader)
    }
}

fn nsurl_from_path(path: &Path) -> Result<objc2::rc::Retained<NSURL>> {
    let path_str = path.to_str().context("path is not valid UTF-8")?;
    let ns_path = NSString::from_str(path_str);
    Ok(NSURL::fileURLWithPath(&ns_path))
}

#[cfg(test)]
mod tests;
