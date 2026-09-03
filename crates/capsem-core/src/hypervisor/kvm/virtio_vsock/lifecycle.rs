//! Backend lifecycle: quiescing for a checkpoint and resetting for a driver
//! re-initialization, both of which detach the vhost backend from guest RAM.

use super::*;

impl VhostVsockDevice {
    /// STATUS=0 from the guest: detach the vhost backend so it stops touching
    /// the rings the driver is about to free, and let `activate` run again
    /// with the rings the driver programs next. `activate` used to
    /// short-circuit on `activated` forever, so after a rebind vhost kept the
    /// old ring addresses and the guest's new ones were never installed.
    pub(super) fn reset_with<I: VhostIoctl + ?Sized>(&mut self, ioctl: &mut I) -> Result<()> {
        if !self.activated {
            return Ok(());
        }
        let vhost_fd = self
            .vhost_fd
            .as_ref()
            .context("vhost-vsock fd not available")?
            .as_raw_fd();
        let running: libc::c_int = 0;
        ioctl
            .call(vhost_fd, VHOST_VSOCK_SET_RUNNING, &running as *const libc::c_int as u64)
            .context("VHOST_VSOCK_SET_RUNNING=0")?;
        self.activated = false;
        debug!(
            event_name = "virtio.vsock.reset",
            "vhost-vsock backend detached for device reset"
        );
        Ok(())
    }

    pub(super) fn quiesce_with<I: VhostIoctl + ?Sized>(&mut self, ioctl: &mut I) -> Result<()> {
        if !self.activated {
            self.checkpoint_state = Some(encode_vsock_checkpoint(self.guest_cid, None));
            return Ok(());
        }
        let vhost_fd = self
            .vhost_fd
            .as_ref()
            .context("vhost-vsock fd not available")?
            .as_raw_fd();

        // VHOST_VSOCK_SET_RUNNING=0 detaches both backends while holding each
        // vring mutex. It therefore waits for active handlers to leave guest
        // memory, and later queued handlers observe no backend. It must
        // complete before GET_VRING_BASE and before the caller copies RAM.
        let running: libc::c_int = 0;
        ioctl
            .call(vhost_fd, VHOST_VSOCK_SET_RUNNING, &running as *const libc::c_int as u64)
            .context("VHOST_VSOCK_SET_RUNNING=0")?;

        let mut vring_bases = [0u32; VHOST_VSOCK_BACKEND_QUEUES];
        for (index, base) in vring_bases.iter_mut().enumerate() {
            let mut state = VhostVringState {
                index: index as u32,
                num: 0,
            };
            ioctl
                .call(
                    vhost_fd,
                    VHOST_GET_VRING_BASE,
                    &mut state as *mut VhostVringState as u64,
                )
                .with_context(|| format!("VHOST_GET_VRING_BASE queue {index}"))?;
            ensure!(
                u16::try_from(state.num).is_ok(),
                "vhost-vsock queue {index} returned invalid vring base {}",
                state.num
            );
            *base = state.num;
        }

        self.checkpoint_state = Some(encode_vsock_checkpoint(self.guest_cid, Some(vring_bases)));
        debug!(
            event_name = "virtio.vsock.quiesce",
            rx_base = vring_bases[0],
            tx_base = vring_bases[1],
            "vhost-vsock backend stopped and vring state captured"
        );
        Ok(())
    }
}
