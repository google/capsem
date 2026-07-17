use anyhow::Result;
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{Bool, ProtocolObject};
use objc2::{define_class, msg_send, AnyThread, DefinedClass, Message};
use objc2_foundation::{NSArray, NSError, NSObject, NSObjectProtocol};
use objc2_virtualization::{
    VZSocketDevice, VZVirtioSocketConnection, VZVirtioSocketDevice, VZVirtioSocketListener,
    VZVirtioSocketListenerDelegate,
};
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::hypervisor::VsockConnection;

/// Wrapper to make Retained<VZVirtioSocketConnection> Send.
/// Safety: The connection object is only used as a lifetime anchor
/// to keep the fd valid. It is never accessed after creation.
struct VzConnectionAnchor(#[allow(dead_code)] Retained<VZVirtioSocketConnection>);
unsafe impl Send for VzConnectionAnchor {}

// ---------------------------------------------------------------------------
// Listener delegate (ObjC bridge)
// ---------------------------------------------------------------------------

pub(crate) struct DelegateIvars {
    tx: mpsc::UnboundedSender<VsockConnection>,
}

define_class!(
    // Safety: NSObject has no subclassing requirements.
    #[unsafe(super(NSObject))]
    #[name = "CapsemVsockListenerDelegate"]
    #[ivars = DelegateIvars]
    pub(crate) struct VsockListenerDelegate;

    unsafe impl NSObjectProtocol for VsockListenerDelegate {}

    unsafe impl VZVirtioSocketListenerDelegate for VsockListenerDelegate {
        #[unsafe(method(listener:shouldAcceptNewConnection:fromSocketDevice:))]
        fn listener_should_accept(
            &self,
            _listener: &VZVirtioSocketListener,
            connection: &VZVirtioSocketConnection,
            _socket_device: &VZVirtioSocketDevice,
        ) -> Bool {
            let fd = unsafe { connection.fileDescriptor() };
            let port = unsafe { connection.destinationPort() };
            info!(fd, port, "vsock: accepted connection");

            if fd < 0 {
                warn!("vsock: connection has invalid fd (-1), rejecting");
                return Bool::NO;
            }

            // Retain the connection object so the fd stays open.
            let retained_conn: Retained<VZVirtioSocketConnection> = connection.retain();
            let conn = VsockConnection::new(fd, port, Box::new(VzConnectionAnchor(retained_conn)));

            if let Err(e) = self.ivars().tx.send(conn) {
                warn!("vsock: failed to send connection to manager: {e}");
                return Bool::NO;
            }

            Bool::YES
        }
    }
);

impl VsockListenerDelegate {
    fn new(tx: mpsc::UnboundedSender<VsockConnection>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(DelegateIvars { tx });
        unsafe { msg_send![super(this), init] }
    }
}

pub type VsockListeners = (
    mpsc::UnboundedReceiver<VsockConnection>,
    Retained<VsockListenerDelegate>,
    Vec<Retained<VZVirtioSocketListener>>,
);

/// Set up vsock listeners on the VM's socket devices.
///
/// Returns an unbounded receiver that delivers accepted connections.
/// The returned retained objects must be kept alive for the listeners
/// to remain active.
pub fn setup_vsock_listeners(
    socket_devices: &NSArray<VZSocketDevice>,
    ports: &[u32],
) -> Result<VsockListeners> {
    let device_count = socket_devices.count();
    if device_count == 0 {
        anyhow::bail!("no socket devices configured on VM");
    }

    // There's only one VZVirtioSocketDeviceConfiguration allowed per VM.
    let socket_device = socket_devices.objectAtIndex(0);

    // Downcast VZSocketDevice -> VZVirtioSocketDevice.
    // Safety: We only configure VZVirtioSocketDeviceConfiguration, so the
    // runtime type is always VZVirtioSocketDevice.
    let device_ref: &VZSocketDevice = &socket_device;
    let virtio_device: &VZVirtioSocketDevice =
        unsafe { &*(device_ref as *const VZSocketDevice as *const VZVirtioSocketDevice) };

    let (tx, rx) = mpsc::unbounded_channel();

    let delegate = VsockListenerDelegate::new(tx);
    let delegate_proto =
        ProtocolObject::from_retained(delegate.clone() as Retained<VsockListenerDelegate>);

    let mut listeners = Vec::new();
    for &port in ports {
        let listener = unsafe { VZVirtioSocketListener::new() };
        unsafe {
            listener.setDelegate(Some(&delegate_proto));
            virtio_device.setSocketListener_forPort(&listener, port);
        }
        info!(port, "vsock: listener registered");
        listeners.push(listener);
    }

    Ok((rx, delegate, listeners))
}

/// Open one host-initiated connection to a guest-listening vsock port.
///
/// Virtualization.framework owns the returned fd, so the retained connection
/// object is carried inside `VsockConnection` as its lifetime anchor. This
/// function must run on the main thread because both the VZ call and its
/// completion are serviced by the main run loop.
pub fn connect_to_guest(
    socket_devices: &NSArray<VZSocketDevice>,
    port: u32,
) -> Result<VsockConnection> {
    anyhow::ensure!(
        super::machine::is_main_thread(),
        "VZVirtioSocketDevice.connectToPort() must run on the main thread"
    );
    anyhow::ensure!(
        socket_devices.count() > 0,
        "no socket devices configured on VM"
    );

    let socket_device = socket_devices.objectAtIndex(0);
    let device_ref: &VZSocketDevice = &socket_device;
    // Safety: Capsem configures exactly one VZVirtioSocketDeviceConfiguration,
    // so the runtime device behind the superclass reference is always virtio.
    let virtio_device: &VZVirtioSocketDevice =
        unsafe { &*(device_ref as *const VZSocketDevice as *const VZVirtioSocketDevice) };

    let (tx, rx) = std::sync::mpsc::channel::<Result<VsockConnection>>();
    let completion = RcBlock::new(
        move |connection: *mut VZVirtioSocketConnection, error: *mut NSError| {
            if !error.is_null() {
                let description = unsafe { format!("{:?}", (*error).debugDescription()) };
                let _ = tx.send(Err(anyhow::anyhow!(
                    "VZ vsock connect to guest port {port} failed: {description}"
                )));
                return;
            }
            if connection.is_null() {
                let _ = tx.send(Err(anyhow::anyhow!(
                    "VZ vsock connect to guest port {port} returned no connection"
                )));
                return;
            }

            let connection_ref = unsafe { &*connection };
            let fd = unsafe { connection_ref.fileDescriptor() };
            if fd < 0 {
                let _ = tx.send(Err(anyhow::anyhow!(
                    "VZ vsock connect to guest port {port} returned invalid fd"
                )));
                return;
            }
            let retained = connection_ref.retain();
            let connected = VsockConnection::new(fd, port, Box::new(VzConnectionAnchor(retained)));
            let _ = tx.send(Ok(connected));
        },
    );

    unsafe {
        virtio_device.connectToPort_completionHandler(port, &completion);
    }

    loop {
        match rx.try_recv() {
            Ok(result) => return result,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                anyhow::bail!("VZ vsock connect completion channel closed")
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => unsafe {
                core_foundation_sys::runloop::CFRunLoopRunInMode(
                    core_foundation_sys::runloop::kCFRunLoopDefaultMode,
                    0.005,
                    1,
                );
            },
        }
    }
}
