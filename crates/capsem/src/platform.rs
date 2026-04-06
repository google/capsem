use std::path::PathBuf;

/// How capsem was installed.
#[derive(Debug, Clone, PartialEq)]
pub enum InstallLayout {
    /// macOS .pkg installer (/usr/local/bin)
    MacosPkg,
    /// Linux/macOS user-dir install (~/.capsem/bin)
    UserDir,
    /// Development build (cargo target directory)
    Development,
}

/// Detect the install layout from the current executable path.
pub fn detect_install_layout() -> InstallLayout {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return InstallLayout::Development,
    };

    let exe_str = exe.to_string_lossy();

    if exe_str.contains("/usr/local/bin") {
        return InstallLayout::MacosPkg;
    }

    if exe_str.contains(".capsem/bin") {
        return InstallLayout::UserDir;
    }

    InstallLayout::Development
}

/// Return the install bin directory for the current layout.
pub fn install_bin_dir() -> Option<PathBuf> {
    match detect_install_layout() {
        InstallLayout::MacosPkg => Some(PathBuf::from("/usr/local/bin")),
        InstallLayout::UserDir => {
            std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".capsem").join("bin"))
        }
        InstallLayout::Development => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_returns_development_in_test() {
        // In test context, exe is in target/debug
        let layout = detect_install_layout();
        assert_eq!(layout, InstallLayout::Development);
    }
}
