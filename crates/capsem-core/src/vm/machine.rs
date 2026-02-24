use std::os::unix::io::RawFd;
use std::time::Duration;

use anyhow::{Context, Result};
use block2::RcBlock;
use objc2::AllocAnyThread;
use objc2::rc::Retained;
use objc2_foundation::{NSArray, NSObjectProtocol, NSString, NSURL};
use objc2_virtualization::{
    VZDiskImageCachingMode, VZDiskImageStorageDeviceAttachment, VZDiskImageSynchronizationMode,
    VZEntropyDeviceConfiguration, VZGenericPlatformConfiguration,
    VZGraphicsDeviceConfiguration, VZKeyboardConfiguration, VZPointingDeviceConfiguration,
    VZSerialPortConfiguration, VZStorageDeviceConfiguration, VZUSBKeyboardConfiguration,
    VZUSBScreenCoordinatePointingDeviceConfiguration, VZVirtioBlockDeviceConfiguration,
    VZVirtioEntropyDeviceConfiguration, VZVirtioGraphicsDeviceConfiguration,
    VZVirtioGraphicsScanoutConfiguration, VZVirtualMachine as ObjcVZVirtualMachine,
    VZVirtualMachineConfiguration, VZVirtualMachineState,
};
use tokio::sync::broadcast;
use tracing::info;

use super::boot::create_boot_loader;
use super::config::VmConfig;
use super::serial;

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
    pub fn create(config: &VmConfig) -> Result<(Self, broadcast::Receiver<String>, RawFd)> {
        let boot_loader = create_boot_loader(config)?;

        let (serial_port_config, serial_console, input_fd) = serial::create_serial_port()?;

        let vz_config = unsafe {
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

            // Virtio graphics device (1920x1200 scanout)
            let scanout = VZVirtioGraphicsScanoutConfiguration::initWithWidthInPixels_heightInPixels(
                VZVirtioGraphicsScanoutConfiguration::alloc(),
                1920,
                1200,
            );
            let scanout_array = NSArray::from_retained_slice(&[scanout]);
            let gpu = VZVirtioGraphicsDeviceConfiguration::new();
            gpu.setScanouts(&scanout_array);
            let gpu_super: Retained<VZGraphicsDeviceConfiguration> =
                Retained::into_super(gpu);
            let gpu_array = NSArray::from_retained_slice(&[gpu_super]);
            vz_config.setGraphicsDevices(&gpu_array);

            // USB keyboard
            let keyboard = VZUSBKeyboardConfiguration::new();
            let keyboard_super: Retained<VZKeyboardConfiguration> =
                Retained::into_super(keyboard);
            let keyboard_array = NSArray::from_retained_slice(&[keyboard_super]);
            vz_config.setKeyboards(&keyboard_array);

            // USB screen coordinate pointing device
            let pointing = VZUSBScreenCoordinatePointingDeviceConfiguration::new();
            let pointing_super: Retained<VZPointingDeviceConfiguration> =
                Retained::into_super(pointing);
            let pointing_array = NSArray::from_retained_slice(&[pointing_super]);
            vz_config.setPointingDevices(&pointing_array);

            // Block device (rootfs)
            if let Some(ref disk_path) = config.disk_path {
                let path_str = disk_path.to_str().context("disk path not valid UTF-8")?;
                let ns_path = NSString::from_str(path_str);
                let disk_url = NSURL::fileURLWithPath(&ns_path);

                let disk_attachment =
                    VZDiskImageStorageDeviceAttachment::initWithURL_readOnly_cachingMode_synchronizationMode_error(
                        VZDiskImageStorageDeviceAttachment::alloc(),
                        &disk_url,
                        true, // read-only for M1
                        VZDiskImageCachingMode::Automatic,
                        VZDiskImageSynchronizationMode::Full,
                    )
                    .map_err(|e| anyhow::anyhow!("disk attach failed: {e:?}"))?;

                let block_device = VZVirtioBlockDeviceConfiguration::initWithAttachment(
                    VZVirtioBlockDeviceConfiguration::alloc(),
                    &disk_attachment,
                );

                // Cast to superclass array
                let block_device_super: Retained<VZStorageDeviceConfiguration> =
                    Retained::into_super(block_device);
                let storage_array = NSArray::from_retained_slice(&[block_device_super]);
                vz_config.setStorageDevices(&storage_array);
            }

            // Validate
            vz_config
                .validateWithError()
                .map_err(|e| anyhow::anyhow!("VM config validation failed: {e:?}"))?;

            vz_config
        };

        let vm = unsafe {
            ObjcVZVirtualMachine::initWithConfiguration(
                ObjcVZVirtualMachine::alloc(),
                &vz_config,
            )
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

    /// Start the VM. Must be called on the main thread.
    ///
    /// Spins the CFRunLoop while waiting for the completion handler,
    /// since VZVirtualMachine dispatches callbacks on the main queue.
    pub fn start(&mut self) -> Result<()> {
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

    /// Get the current VM state as a string.
    pub fn state(&self) -> &'static str {
        let state = unsafe { self.inner.state() };
        match state {
            VZVirtualMachineState::Stopped => "stopped",
            VZVirtualMachineState::Running => "running",
            VZVirtualMachineState::Paused => "paused",
            VZVirtualMachineState::Error => "error",
            VZVirtualMachineState::Starting => "starting",
            VZVirtualMachineState::Stopping => "stopping",
            VZVirtualMachineState::Pausing => "pausing",
            VZVirtualMachineState::Resuming => "resuming",
            VZVirtualMachineState::Saving => "saving",
            VZVirtualMachineState::Restoring => "restoring",
            _ => "unknown",
        }
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
