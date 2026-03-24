use std::os::unix::io::RawFd;
use std::time::Duration;

use anyhow::{Context, Result};
use block2::RcBlock;
use objc2::AllocAnyThread;
use objc2::rc::Retained;
use objc2_foundation::{NSArray, NSObjectProtocol, NSString, NSURL};
use objc2_virtualization::{
    VZDiskImageCachingMode, VZDiskImageStorageDeviceAttachment, VZDiskImageSynchronizationMode,
    VZDirectorySharingDeviceConfiguration,
    VZEntropyDeviceConfiguration, VZGenericPlatformConfiguration,
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
use tokio::sync::broadcast;
use tracing::{debug_span, info};

use super::boot::create_boot_loader;
use super::config::VmConfig;
use super::serial;

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

/// High-level wrapper around VZVirtualMachine.
pub struct VirtualMachine {
    inner: Retained<ObjcVZVirtualMachine>,
    serial_console: Option<serial::SerialConsole>,
}

// VZVirtualMachine is main-thread-only, but we manage that with dispatch.
// We wrap access behind a Mutex and ensure VZ calls happen on the main queue.
unsafe impl Send for VirtualMachine {}

impl VirtualMachine {
    /// Create a new virtual machine from the given config.
    /// Must be called from the main thread (or dispatched to it).
    ///
    /// Returns the VM, a broadcast receiver for serial console output,
    /// and a RawFd for writing input to the guest's serial console.
    pub fn create(config: &VmConfig) -> Result<(Self, broadcast::Receiver<Vec<u8>>, RawFd)> {
        let boot_loader = {
            let _span = debug_span!("create_boot_loader").entered();
            create_boot_loader(config)?
        };

        let (serial_port_config, serial_console, input_fd) = {
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

                // Platform
                let platform = VZGenericPlatformConfiguration::new();
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

        let rx = serial_console.subscribe();
        Ok((
            Self {
                inner: vm,
                serial_console: Some(serial_console),
            },
            rx,
            input_fd,
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
    /// Spins the CFRunLoop while waiting for the completion handler,
    /// since VZVirtualMachine dispatches callbacks on the main queue.
    pub fn start(&mut self) -> Result<()> {
        let _span = debug_span!("vm_start").entered();

        anyhow::ensure!(
            is_main_thread(),
            "VZVirtualMachine.start() must be called on the main thread"
        );

        // Start the serial reader before the VM
        if let Some(console) = self.serial_console.take() {
            console.spawn_reader();
        }

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
            self.inner.startWithCompletionHandler(&completion);
        }

        spin_runloop_until(&rx).context("VM start")?;

        info!("virtual machine started");
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

    /// Get the current VM state.
    pub fn state(&self) -> super::VmState {
        let state = unsafe { self.inner.state() };
        match state {
            VZVirtualMachineState::Stopped => super::VmState::Stopped,
            VZVirtualMachineState::Running => super::VmState::Running,
            VZVirtualMachineState::Paused => super::VmState::Paused,
            VZVirtualMachineState::Error => super::VmState::Error,
            VZVirtualMachineState::Starting => super::VmState::Starting,
            VZVirtualMachineState::Stopping => super::VmState::Stopping,
            VZVirtualMachineState::Pausing => super::VmState::Pausing,
            VZVirtualMachineState::Resuming => super::VmState::Resuming,
            VZVirtualMachineState::Saving => super::VmState::Saving,
            VZVirtualMachineState::Restoring => super::VmState::Restoring,
            _ => super::VmState::Unknown,
        }
    }
}

/// Create a VZ block device attachment from a disk image path.
///
/// `read_only`: true for rootfs, false for scratch disks.
/// `identifier`: optional virtio block device identifier (max 20 ASCII bytes),
/// exposed in guest as `/dev/disk/by-id/virtio-<identifier>`.
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
///
/// `tag`: mount tag visible in guest (used with `mount -t virtiofs <tag> <mountpoint>`).
/// `host_path`: host directory to share with the guest.
/// `read_only`: if true, guest cannot write to the share.
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
///
/// VZVirtualMachine completion handlers are dispatched on the main queue.
/// If we just block with `rx.recv()`, the main run loop never processes them
/// and we deadlock. This pumps the run loop in short intervals.
fn spin_runloop_until(rx: &std::sync::mpsc::Receiver<Result<()>>) -> Result<()> {
    loop {
        match rx.try_recv() {
            Ok(result) => return result,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                return Err(anyhow::anyhow!("completion handler channel closed"));
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                // Pump the run loop so completion handlers can fire.
                unsafe {
                    core_foundation_sys::runloop::CFRunLoopRunInMode(
                        core_foundation_sys::runloop::kCFRunLoopDefaultMode,
                        0.01, // 10ms
                        0,    // don't return after processing a source
                    );
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_main_thread_returns_false_on_worker() {
        // Cargo test harness runs tests on worker threads, not the main thread.
        // Spawning another thread should also return false.
        let result = std::thread::spawn(is_main_thread).join().unwrap();
        assert!(!result);
    }

    #[test]
    fn is_main_thread_returns_false_in_test_harness() {
        // The test harness itself uses worker threads.
        // This verifies the guard would catch a VZ call from a test thread.
        assert!(!is_main_thread());
    }

    #[tokio::test]
    async fn is_main_thread_returns_false_in_tokio() {
        // tokio::test uses a worker thread -- the exact scenario that caused
        // the crash when gui_boot_vm was called after rootfs download.
        assert!(!is_main_thread());
    }
}
