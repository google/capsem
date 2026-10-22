//! Workload browser helper -> the agent's existing serialized control writer.
//! One bounded datagram, no per-request thread or terminal scanning.
use crate::control_writer::CtrlSender;
use capsem_proto::browser::{BROWSER_SOCKET, BROWSER_SOCKET_DIR, MAX_BROWSER_URL_BYTES};
use std::io;
use std::os::unix::{fs::PermissionsExt, net::UnixDatagram};

pub(crate) fn start(sender: CtrlSender) -> io::Result<()> {
    std::fs::create_dir_all(BROWSER_SOCKET_DIR)?;
    std::fs::set_permissions(BROWSER_SOCKET_DIR, std::fs::Permissions::from_mode(0o755))?;
    match std::fs::remove_file(BROWSER_SOCKET) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let socket = UnixDatagram::bind(BROWSER_SOCKET)?;
    std::fs::set_permissions(BROWSER_SOCKET, std::fs::Permissions::from_mode(0o666))?;
    std::thread::spawn(move || {
        let mut buffer = [0; MAX_BROWSER_URL_BYTES + 128];
        while receive(&socket, &sender, &mut buffer).is_ok() {}
    });
    Ok(())
}

fn receive(socket: &UnixDatagram, sender: &CtrlSender, buffer: &mut [u8]) -> io::Result<()> {
    let size = socket.recv(buffer)?;
    // A full buffer may be truncated; valid records are smaller.
    if size < buffer.len() && size >= 4 && u32::from_be_bytes(buffer[..4].try_into().unwrap()) as usize == size - 4 {
        if let Ok(capsem_proto::GuestToHost::OpenUrl { url }) = capsem_proto::decode_guest_msg(&buffer[4..size]) {
            let _ = sender.send_browser(&url.0);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
