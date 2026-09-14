//! The Virtualization framework's word on why a VM stopped.
//!
//! Without a delegate the owner never hears that its VM ended: every VSOCK
//! stream reads EOF at once and the service goes on reporting the VM as
//! running while every exec times out. The witness records the framework's
//! own reason, at error level in the owner's log and for whoever asks.
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{define_class, msg_send, AnyThread, DefinedClass};
use objc2_foundation::{NSError, NSObject, NSObjectProtocol};
use objc2_virtualization::{VZNetworkDevice, VZVirtualMachine, VZVirtualMachineDelegate};
use std::sync::{Arc, Mutex};
use tracing::{error, warn};

/// What the framework said when the VM stopped, if it has.
#[derive(Default)]
pub struct StopWitness {
    reason: Mutex<Option<String>>,
}

impl StopWitness {
    /// Record the reason and end the owner the way a signal would: the VM
    /// is gone, so the drain runs and the service's reaper sees an exit
    /// instead of answering `Running` for a dead VM until every exec has
    /// timed out.
    fn record(&self, reason: String) {
        error!(reason, "virtual machine stopped");
        *self.reason.lock().unwrap() = Some(reason);
        let ended = capsem_foundation::unix::process::ProcessId::try_from(std::process::id())
            .map_err(std::io::Error::other)
            .and_then(|own| {
                capsem_foundation::unix::process::send_signal(own, capsem_foundation::unix::process::Signal::Terminate)
            });
        if let Err(error) = ended {
            error!(%error, "could not end the owner after the virtual machine stopped");
        }
    }

    pub fn reason(&self) -> Option<String> {
        self.reason.lock().unwrap().clone()
    }
}

pub(crate) struct DelegateIvars {
    witness: Arc<StopWitness>,
}

define_class!(
    // Safety: NSObject has no subclassing requirements.
    #[unsafe(super(NSObject))]
    #[name = "CapsemVmDelegate"]
    #[ivars = DelegateIvars]
    pub(crate) struct VmDelegate;

    unsafe impl NSObjectProtocol for VmDelegate {}

    unsafe impl VZVirtualMachineDelegate for VmDelegate {
        #[unsafe(method(guestDidStopVirtualMachine:))]
        fn guest_did_stop(&self, _machine: &VZVirtualMachine) {
            self.ivars()
                .witness
                .record("the guest stopped the virtual machine".into());
        }

        #[unsafe(method(virtualMachine:didStopWithError:))]
        fn did_stop_with_error(&self, _machine: &VZVirtualMachine, failure: &NSError) {
            self.ivars().witness.record(format!(
                "the framework stopped the virtual machine: {} (domain {}, code {})",
                failure.localizedDescription(),
                failure.domain(),
                failure.code()
            ));
        }

        #[unsafe(method(virtualMachine:networkDevice:attachmentWasDisconnectedWithError:))]
        fn network_disconnected(&self, _machine: &VZVirtualMachine, _device: &VZNetworkDevice, failure: &NSError) {
            warn!(error = %failure.localizedDescription(), "virtual machine network attachment disconnected");
        }
    }
);

impl VmDelegate {
    pub(crate) fn new(witness: Arc<StopWitness>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(DelegateIvars { witness });
        unsafe { msg_send![super(this), init] }
    }

    pub(crate) fn protocol(this: &Retained<Self>) -> Retained<ProtocolObject<dyn VZVirtualMachineDelegate>> {
        ProtocolObject::from_retained(this.clone())
    }
}
