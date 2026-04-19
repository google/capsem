use anyhow::{Context, Result};
use block2::RcBlock;
use objc2::AllocAnyThread;
use objc2::rc::Retained;
use objc2_foundation::{NSArray, NSData, NSObjectProtocol, NSString, NSURL};
use objc2_virtualization::{
    VZDiskImageCachingMode, VZDiskImageStorageDeviceAttachment, VZDiskImageSynchronizationMode,
    VZDirectorySharingDeviceConfiguration,
    VZEntropyDeviceConfiguration, VZGenericMachineIdentifier, VZGenericPlatformConfiguration,
    VZSerialPortConfiguration, VZSharedDirectory, VZSingleDirectoryShare,
    VZSocketDevice, VZSocketDeviceConfiguration,
    VZStorageDeviceConfiguration,
    VZVirtioBlockDeviceConfiguration,
    VZVirtioEntropyDeviceConfiguration,
    VZVirtioFileSystemDeviceConfiguration,
    VZVirtioSocketDeviceConfiguration,
    VZVirtualMachine as ObjcVZVirtualMachine,
    VZVirtualMachineConfiguration, VZVirtualMachineState,
};
use tracing::{debug_span, info};

use super::boot::create_boot_loader;
use super::serial::{self, AppleVzSerialConsole};
use crate::vm::VmState;
use crate::vm::config::VmConfig;

/// Returns true if the current thread is the main thread.
/// VZVirtualMachine operations must be called from the main dispatch queue.
pub fn is_main_thread() -> bool {
    // pthread_main_np() returns 1 on the main thread, 0 otherwise.
    // Available on macOS since 10.0.
    extern "C" {
        fn pthread_main_np() -> libc::c_int;
    }
    unsafe { pthread_main_np() == 1 }
}

/// Run a closure on the main thread and wait for its result.
///
/// VZ API calls (pause, save_state, stop, resume) must run on the thread
/// that owns the main CFRunLoop. The caller is typically a tokio worker,
/// so we dispatch the work via GCD's main queue and block until done.
///
/// If already on the main thread, invokes the closure directly — avoids
/// deadlock when boot-time code (on main) eventually calls into this.
pub fn run_on_main_thread<F, R>(f: F) -> Result<R>
where
    F: FnOnce() -> Result<R> + Send + 'static,
    R: Send + 'static,
{
    if is_main_thread() {
        return f();
    }

    // Why not GCD's main queue?
    //
    // VZ completion handlers (e.g. from pauseWithCompletionHandler) are
    // driven by the main dispatch queue. If we're sitting inside a
    // dispatch_async(main_queue, ...) block, the main queue is busy with
    // our block and VZ's completion can't fire -- deadlock. Instead we
    // schedule directly on the main CFRunLoop via CFRunLoopPerformBlock,
    // which executes on the main thread *without* going through the main
    // dispatch queue. That leaves the main queue free to service VZ's
    // internal completions while our block calls spin_runloop_until.
    extern "C" {
        fn CFRunLoopGetMain() -> *mut std::ffi::c_void;
        fn CFRunLoopPerformBlock(
            rl: *mut std::ffi::c_void,
            mode: *const std::ffi::c_void,
            block: *mut block2::Block<dyn Fn()>,
        );
        fn CFRunLoopWakeUp(rl: *mut std::ffi::c_void);
        static kCFRunLoopCommonModes: *const std::ffi::c_void;
    }

    // RcBlock requires `Fn` (callable repeatedly), but `f` is FnOnce. Wrap
    // it in an Option behind a mutex so the first invocation takes it and
    // subsequent invocations (which shouldn't happen, but guard anyway)
    // become no-ops.
    let f_slot: std::sync::Arc<std::sync::Mutex<Option<F>>> =
        std::sync::Arc::new(std::sync::Mutex::new(Some(f)));
    let (tx, rx) = std::sync::mpsc::channel();
    let f_slot_cb = std::sync::Arc::clone(&f_slot);
    let block = block2::RcBlock::new(move || {
        if let Some(f) = f_slot_cb.lock().unwrap().take() {
            let _ = tx.send(f());
        }
    });

    unsafe {
        let rl = CFRunLoopGetMain();
        CFRunLoopPerformBlock(rl, kCFRunLoopCommonModes, &*block as *const _ as *mut _);
        CFRunLoopWakeUp(rl);
    }

    rx.recv()
        .map_err(|_| anyhow::anyhow!("main-runloop perform channel closed"))?
}

/// Load a persisted VZGenericMachineIdentifier from `path`, or generate a new
/// one and write it to `path`. Returns the generated identifier when `path`
/// is `None`.
///
/// Apple VZ requires the identifier to match between save and restore. The
/// default constructor generates a fresh identifier each time, so callers
/// that care about save/restore parity must persist it across boots.
fn load_or_create_machine_identifier(
    path: Option<&std::path::Path>,
) -> Result<Retained<VZGenericMachineIdentifier>> {
    let Some(path) = path else {
        return Ok(unsafe { VZGenericMachineIdentifier::new() });
    };

    if path.exists() {
        let bytes = std::fs::read(path)
            .with_context(|| format!("failed to read machine identifier at {}", path.display()))?;
        let nsdata = NSData::with_bytes(&bytes);
        let id = unsafe {
            VZGenericMachineIdentifier::initWithDataRepresentation(
                VZGenericMachineIdentifier::alloc(),
                &nsdata,
            )
        }
        .ok_or_else(|| {
            anyhow::anyhow!("invalid machine identifier data at {}", path.display())
        })?;
        return Ok(id);
    }

    let id = unsafe { VZGenericMachineIdentifier::new() };
    let data = unsafe { id.dataRepresentation() }.to_vec();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(path, &data)
        .with_context(|| format!("failed to write machine identifier to {}", path.display()))?;
    Ok(id)
}

/// Internal wrapper around VZVirtualMachine.
pub(crate) struct AppleVzMachine {
    inner: Retained<ObjcVZVirtualMachine>,
}

// VZVirtualMachine is main-thread-only, but we manage that with dispatch.
// We wrap access behind a Mutex and ensure VZ calls happen on the main queue.
unsafe impl Send for AppleVzMachine {}

impl AppleVzMachine {
    /// Create and configure a new virtual machine from the given config.
    /// Must be called from the main thread (or dispatched to it).
    ///
    /// Returns the machine, and the serial console that owns both read and input fds.
    pub fn create(
        config: &VmConfig,
    ) -> Result<(Self, AppleVzSerialConsole)> {
        let boot_loader = {
            let _span = debug_span!("create_boot_loader").entered();
            create_boot_loader(config)?
        };

        let (serial_port_config, serial_console) = {
            let _span = debug_span!("create_serial_port").entered();
            serial::create_serial_port()?
        };

        let vz_config = {
            let _span = debug_span!("vz_configure").entered();
            unsafe {
                let vz_config = VZVirtualMachineConfiguration::new();

                vz_config.setCPUCount(config.cpu_count as usize);
                vz_config.setMemorySize(config.ram_bytes);
                vz_config.setBootLoader(Some(&boot_loader));

                // Platform. VZGenericPlatformConfiguration auto-generates a
                // fresh machineIdentifier on every construction, which would
                // make restoreMachineStateFromURL fail with VZErrorRestore
                // because the saved state references the original identifier.
                // Persist the identifier alongside the session and reuse it
                // across boots so save/restore parity holds.
                let platform = VZGenericPlatformConfiguration::new();
                let identifier =
                    load_or_create_machine_identifier(config.machine_identifier_path.as_deref())?;
                platform.setMachineIdentifier(&identifier);
                vz_config.setPlatform(&platform);

                // Entropy device (prevents hangs waiting for random)
                let entropy_config = VZVirtioEntropyDeviceConfiguration::new();
                let entropy_super: Retained<VZEntropyDeviceConfiguration> =
                    Retained::into_super(entropy_config);
                let entropy_array = NSArray::from_retained_slice(&[entropy_super]);
                vz_config.setEntropyDevices(&entropy_array);

                // Serial ports - cast to superclass array
                let serial_port_super: Retained<VZSerialPortConfiguration> =
                    Retained::into_super(serial_port_config);
                let serial_array = NSArray::from_retained_slice(&[serial_port_super]);
                vz_config.setSerialPorts(&serial_array);

                // Vsock device for host<->guest PTY and control channels
                let vsock_config = VZVirtioSocketDeviceConfiguration::new();
                let vsock_super: Retained<VZSocketDeviceConfiguration> =
                    Retained::into_super(vsock_config);
                let socket_array = NSArray::from_retained_slice(&[vsock_super]);
                vz_config.setSocketDevices(&socket_array);

                // Block devices
                let mut storage_devices: Vec<Retained<VZStorageDeviceConfiguration>> = Vec::new();

                // Rootfs (read-only)
                if let Some(ref disk_path) = config.disk_path {
                    let device = attach_disk(disk_path, true, Some("rootfs"))?;
                    storage_devices.push(device);
                }

                // Scratch disk (read-write, ephemeral workspace)
                if let Some(ref scratch_path) = config.scratch_disk_path {
                    let device = attach_disk(scratch_path, false, Some("scratch"))?;
                    storage_devices.push(device);
                }

                if !storage_devices.is_empty() {
                    let storage_array = NSArray::from_retained_slice(&storage_devices);
                    vz_config.setStorageDevices(&storage_array);
                }

                // VirtioFS directory sharing devices
                if !config.virtio_fs_shares.is_empty() {
                    let mut dir_devices: Vec<Retained<VZDirectorySharingDeviceConfiguration>> =
                        Vec::new();
                    for share in &config.virtio_fs_shares {
                        let device =
                            attach_virtiofs_share(&share.tag, &share.host_path, share.read_only)?;
                        dir_devices.push(device);
                    }
                    let dir_array = NSArray::from_retained_slice(&dir_devices);
                    vz_config.setDirectorySharingDevices(&dir_array);
                }

                // Validate
                {
                    let _span = debug_span!("vz_validate").entered();
                    vz_config
                        .validateWithError()
                        .map_err(|e| anyhow::anyhow!("VM config validation failed: {e:?}"))?;
                }

                vz_config
            }
        };

        let vm = {
            let _span = debug_span!("vz_init").entered();
            unsafe {
                ObjcVZVirtualMachine::initWithConfiguration(
                    ObjcVZVirtualMachine::alloc(),
                    &vz_config,
                )
            }
        };

        info!("virtual machine created");

        Ok((
            Self { inner: vm },
            serial_console,
        ))
    }

    /// Access the underlying VZVirtualMachine for embedding in a VZVirtualMachineView.
    pub fn inner_vz(&self) -> &ObjcVZVirtualMachine {
        &self.inner
    }

    /// Access the vsock socket devices for registering listeners post-boot.
    pub fn socket_devices(&self) -> Retained<NSArray<VZSocketDevice>> {
        unsafe { self.inner.socketDevices() }
    }

    /// Start the VM. Must be called on the main thread.
    ///
    /// Also spawns the serial reader thread.
    pub fn start(&self, serial: &AppleVzSerialConsole, checkpoint_path: Option<&std::path::Path>) -> Result<()> {
        let _span = debug_span!("vm_start").entered();

        anyhow::ensure!(
            is_main_thread(),
            "VZVirtualMachine.start() must be called on the main thread"
        );

        // Start the serial reader before the VM
        serial.spawn_reader();

        let (tx, rx) = std::sync::mpsc::channel();

        let completion = RcBlock::new(move |error: *mut objc2_foundation::NSError| {
            if error.is_null() {
                let _ = tx.send(Ok(()));
            } else {
                let desc = unsafe { format!("{:?}", (*error).debugDescription()) };
                let _ = tx.send(Err(anyhow::anyhow!("VM start failed: {desc}")));
            }
        });

        unsafe {
            if let Some(cp) = checkpoint_path {
                let path_str = cp.to_string_lossy().to_string();
                let url = objc2_foundation::NSURL::fileURLWithPath(&objc2_foundation::NSString::from_str(&path_str));
                self.inner.restoreMachineStateFromURL_completionHandler(&url, &completion);
            } else {
                self.inner.startWithCompletionHandler(&completion);
            }
        }

        spin_runloop_until(&rx).context("VM start")?;

        if checkpoint_path.is_some() {
            info!("virtual machine restored from checkpoint");
            // restoreMachineStateFromURL leaves the VM in the paused state per
            // Apple's docs. Resume it so the guest actually runs.
            self.resume().context("VM resume after restore")?;
        } else {
            info!("virtual machine started");
        }
        Ok(())
    }

    /// Request the VM to stop. Must be called on the main thread.
    pub fn stop(&self) -> Result<()> {
        anyhow::ensure!(
            is_main_thread(),
            "VZVirtualMachine.stop() must be called on the main thread"
        );

        let (tx, rx) = std::sync::mpsc::channel();

        let completion = RcBlock::new(move |error: *mut objc2_foundation::NSError| {
            if error.is_null() {
                let _ = tx.send(Ok(()));
            } else {
                let desc = unsafe { format!("{:?}", (*error).debugDescription()) };
                let _ = tx.send(Err(anyhow::anyhow!("VM stop failed: {desc}")));
            }
        });

        unsafe {
            self.inner.stopWithCompletionHandler(&completion);
        }

        spin_runloop_until(&rx).context("VM stop")?;

        info!("virtual machine stopped");
        Ok(())
    }

    pub fn pause(&self) -> Result<()> {
        anyhow::ensure!(
            is_main_thread(),
            "VZVirtualMachine.pause() must be called on the main thread"
        );
        let (tx, rx) = std::sync::mpsc::channel();
        let completion = RcBlock::new(move |error: *mut objc2_foundation::NSError| {
            if error.is_null() {
                let _ = tx.send(Ok(()));
            } else {
                let desc = unsafe { format!("{:?}", (*error).debugDescription()) };
                let _ = tx.send(Err(anyhow::anyhow!("VM pause failed: {desc}")));
            }
        });
        unsafe {
            self.inner.pauseWithCompletionHandler(&completion);
        }
        spin_runloop_until(&rx).context("VM pause")?;
        info!("virtual machine paused");
        Ok(())
    }

    pub fn resume(&self) -> Result<()> {
        anyhow::ensure!(
            is_main_thread(),
            "VZVirtualMachine.resume() must be called on the main thread"
        );
        let (tx, rx) = std::sync::mpsc::channel();
        let completion = RcBlock::new(move |error: *mut objc2_foundation::NSError| {
            if error.is_null() {
                let _ = tx.send(Ok(()));
            } else {
                let desc = unsafe { format!("{:?}", (*error).debugDescription()) };
                let _ = tx.send(Err(anyhow::anyhow!("VM resume failed: {desc}")));
            }
        });
        unsafe {
            self.inner.resumeWithCompletionHandler(&completion);
        }
        spin_runloop_until(&rx).context("VM resume")?;
        info!("virtual machine resumed");
        Ok(())
    }

    #[cfg(target_os = "macos")]
    pub fn save_state(&self, path: &std::path::Path) -> Result<()> {
        anyhow::ensure!(
            is_main_thread(),
            "VZVirtualMachine.saveMachineStateToURL() must be called on the main thread"
        );
        let path_str = path.to_string_lossy().to_string();
        let url = objc2_foundation::NSURL::fileURLWithPath(&objc2_foundation::NSString::from_str(&path_str));

        let (tx, rx) = std::sync::mpsc::channel();
        let completion = RcBlock::new(move |error: *mut objc2_foundation::NSError| {
            if error.is_null() {
                let _ = tx.send(Ok(()));
            } else {
                let desc = unsafe { format!("{:?}", (*error).debugDescription()) };
                let _ = tx.send(Err(anyhow::anyhow!("VM save_state failed: {desc}")));
            }
        });

        unsafe {
            self.inner.saveMachineStateToURL_completionHandler(&url, &completion);
        }

        spin_runloop_until(&rx).context("VM save_state")?;
        info!("virtual machine state saved");
        Ok(())
    }

    pub fn supports_checkpoint(&self) -> bool {
        true
    }

    /// Get the current VM state.
    pub fn state(&self) -> VmState {
        let state = unsafe { self.inner.state() };
        match state {
            VZVirtualMachineState::Stopped => VmState::Stopped,
            VZVirtualMachineState::Running => VmState::Running,
            VZVirtualMachineState::Paused => VmState::Paused,
            VZVirtualMachineState::Error => VmState::Error,
            VZVirtualMachineState::Starting => VmState::Starting,
            VZVirtualMachineState::Stopping => VmState::Stopping,
            VZVirtualMachineState::Pausing => VmState::Pausing,
            VZVirtualMachineState::Resuming => VmState::Resuming,
            VZVirtualMachineState::Saving => VmState::Saving,
            VZVirtualMachineState::Restoring => VmState::Restoring,
            _ => VmState::Unknown,
        }
    }
}

/// Create a VZ block device attachment from a disk image path.
fn attach_disk(
    path: &std::path::Path,
    read_only: bool,
    identifier: Option<&str>,
) -> anyhow::Result<Retained<VZStorageDeviceConfiguration>> {
    unsafe {
        let path_str = path.to_str().context("disk path not valid UTF-8")?;
        let ns_path = NSString::from_str(path_str);
        let disk_url = NSURL::fileURLWithPath(&ns_path);

        let disk_attachment =
            VZDiskImageStorageDeviceAttachment::initWithURL_readOnly_cachingMode_synchronizationMode_error(
                VZDiskImageStorageDeviceAttachment::alloc(),
                &disk_url,
                read_only,
                VZDiskImageCachingMode::Cached,
                VZDiskImageSynchronizationMode::None,
            )
            .map_err(|e| anyhow::anyhow!("disk attach failed for {}: {e:?}", path.display()))?;

        let block_device = VZVirtioBlockDeviceConfiguration::initWithAttachment(
            VZVirtioBlockDeviceConfiguration::alloc(),
            &disk_attachment,
        );

        if let Some(id) = identifier {
            let ns_id = NSString::from_str(id);
            block_device.setBlockDeviceIdentifier(&ns_id);
        }

        Ok(Retained::into_super(block_device))
    }
}

/// Create a VirtioFS directory sharing device from a host directory.
fn attach_virtiofs_share(
    tag: &str,
    host_path: &std::path::Path,
    read_only: bool,
) -> anyhow::Result<Retained<VZDirectorySharingDeviceConfiguration>> {
    unsafe {
        let path_str = host_path
            .to_str()
            .context("VirtioFS path not valid UTF-8")?;
        let ns_path = NSString::from_str(path_str);
        let url = NSURL::fileURLWithPath(&ns_path);

        let shared_dir = VZSharedDirectory::initWithURL_readOnly(
            VZSharedDirectory::alloc(),
            &url,
            read_only,
        );

        let single_share = VZSingleDirectoryShare::initWithDirectory(
            VZSingleDirectoryShare::alloc(),
            &shared_dir,
        );

        let ns_tag = NSString::from_str(tag);
        let fs_config = VZVirtioFileSystemDeviceConfiguration::initWithTag(
            VZVirtioFileSystemDeviceConfiguration::alloc(),
            &ns_tag,
        );

        let share_super: Retained<objc2_virtualization::VZDirectoryShare> =
            Retained::into_super(single_share);
        fs_config.setShare(Some(&share_super));

        Ok(Retained::into_super(fs_config))
    }
}

/// Spin the main CFRunLoop until `rx` has a value.
fn spin_runloop_until(rx: &std::sync::mpsc::Receiver<Result<()>>) -> Result<()> {
    loop {
        match rx.try_recv() {
            Ok(result) => return result,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                return Err(anyhow::anyhow!("completion handler channel closed"));
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                // Pump the run loop so completion handlers can fire.
                // returnAfterSourceHandled=1: return immediately once the
                // completion handler fires instead of waiting for full timeout.
                unsafe {
                    core_foundation_sys::runloop::CFRunLoopRunInMode(
                        core_foundation_sys::runloop::kCFRunLoopDefaultMode,
                        0.005, // 5ms max
                        1,     // return after processing first source
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_main_thread_returns_false_on_worker() {
        let result = std::thread::spawn(is_main_thread).join().unwrap();
        assert!(!result);
    }

    #[test]
    fn is_main_thread_returns_false_in_test_harness() {
        assert!(!is_main_thread());
    }

    #[tokio::test]
    async fn is_main_thread_returns_false_in_tokio() {
        assert!(!is_main_thread());
    }
}
