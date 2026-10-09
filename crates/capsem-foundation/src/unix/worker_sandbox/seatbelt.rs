use super::{Access, Policy, Role};
use std::ffi::CString;
use std::io;
use std::os::unix::ffi::OsStrExt;

pub(super) struct Compiled {
    source: CString,
    parameters: Vec<(CString, CString)>,
}

pub(super) fn compile(policy: &Policy) -> io::Result<Compiled> {
    let mut source = if matches!(policy.role(), Role::Ledger | Role::Proxy) {
        String::from("(version 1)\n(deny default)\n")
    } else {
        String::from(
            "(version 1)\n\
             (allow default)\n\
             (deny file-read*)\n\
             (deny file-write*)\n\
             (deny network-outbound)\n\
             (deny network-bind)\n\
             (deny network-inbound)\n\
             (deny process-exec)\n\
             (deny signal)\n\
             (allow signal (target same-sandbox))\n",
        )
    };
    if policy.role() == Role::Gateway {
        source.push_str("(allow network-inbound (local ip \"*:*\"))\n");
    }

    let mut parameters = Vec::with_capacity(policy.paths().len());
    for (index, rule) in policy.paths().iter().enumerate() {
        let name = format!("PATH_{index}");
        let filter = format!("(require-any (literal (param \"{name}\")) (subpath (param \"{name}\")))");
        source.push_str(&format!(
            "(allow file-read-metadata (path-ancestors (param \"{name}\")))\n"
        ));
        match rule.access() {
            Access::ReadOnly => source.push_str(&format!("(allow file-read* {filter})\n")),
            Access::ReadWrite => {
                source.push_str(&format!("(allow file-read* file-write* {filter})\n"));
            }
            Access::Executable => {
                source.push_str(&format!(
                    "(allow file-read* (literal (param \"{name}\")))\n\
                     (allow process-exec (with no-sandbox) (literal (param \"{name}\")))\n"
                ));
            }
        }
        parameters.push((
            cstring(name.as_bytes(), "sandbox parameter name")?,
            cstring(rule.path().as_os_str().as_bytes(), "sandbox path")?,
        ));
    }
    // Permission and ownership changes stay brokered even beneath writable
    // grants. These final rules win over the broader file-write allowance.
    source.push_str("(deny file-write-mode file-write-owner file-write-setugid)\n");

    Ok(Compiled {
        source: cstring(source.as_bytes(), "sandbox profile")?,
        parameters,
    })
}

fn cstring(value: &[u8], name: &str) -> io::Result<CString> {
    CString::new(value).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, format!("{name} contains NUL")))
}

impl Compiled {
    pub(super) fn source(&self) -> &std::ffi::CStr {
        &self.source
    }

    #[cfg(target_os = "macos")]
    pub(super) fn parameter_ptrs(&self) -> Vec<*const libc::c_char> {
        let mut pointers = Vec::with_capacity(self.parameters.len() * 2 + 1);
        for (name, value) in &self.parameters {
            pointers.push(name.as_ptr());
            pointers.push(value.as_ptr());
        }
        pointers.push(std::ptr::null());
        pointers
    }

    #[cfg(test)]
    pub(super) fn parameters(&self) -> Vec<(&str, &str)> {
        self.parameters
            .iter()
            .map(|(name, value)| (name.to_str().unwrap(), value.to_str().unwrap()))
            .collect()
    }
}
