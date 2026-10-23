//! The native shell discovers authority from one private runtime owner.

use std::io::Read;
use std::path::Path;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Connection {
    pub gateway_url: String,
    pub token: String,
}

pub(crate) fn read_connection(run_dir: &Path) -> Result<Connection, String> {
    connection_with(
        || read_token(&run_dir.join("gateway.token")),
        || read_port(&run_dir.join("gateway.port")),
    )
}

fn connection_with(
    mut token: impl FnMut() -> Result<String, String>,
    port: impl FnOnce() -> Result<u16, String>,
) -> Result<Connection, String> {
    // The gateway publishes its token last as the readiness marker. A second
    // read refuses a replacement owner rather than pairing its token with an
    // earlier port. No caller-provided URL can receive this authority.
    let first = token()?;
    let port = port()?;
    if first != token()? {
        return Err("gateway owner changed; retry connection".into());
    }
    Ok(Connection {
        gateway_url: format!("http://127.0.0.1:{port}"),
        token: first,
    })
}

fn private_value(path: &Path, maximum: u64) -> Result<String, String> {
    let file = capsem_foundation::unix::fs::open_regular_file_no_follow(path)
        .map_err(|_| "gateway runtime file is unavailable".to_owned())?;
    let metadata = file
        .metadata()
        .map_err(|_| "gateway runtime file is unavailable".to_owned())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err("gateway runtime file is not private".into());
        }
    }
    let mut value = String::new();
    file.take(maximum + 1)
        .read_to_string(&mut value)
        .map_err(|_| "gateway runtime file is unavailable".to_owned())?;
    if value.len() as u64 > maximum {
        return Err("gateway runtime file exceeds its bound".into());
    }
    Ok(value)
}

pub(super) fn read_token(path: &Path) -> Result<String, String> {
    let token = private_value(path, 64)?;
    if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return Err("gateway token is invalid".into());
    }
    Ok(token)
}

fn read_port(path: &Path) -> Result<u16, String> {
    let raw = private_value(path, 5)?;
    let port = raw.parse::<u16>().ok().filter(|port| *port != 0);
    if !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("gateway port is invalid".into());
    }
    port.ok_or_else(|| "gateway port is invalid".into())
}

#[cfg(test)]
mod tests;
