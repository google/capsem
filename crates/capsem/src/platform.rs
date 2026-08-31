use std::path::PathBuf;

/// How capsem was installed.
#[derive(Debug, Clone, PartialEq)]
pub enum InstallLayout {
    /// macOS .pkg installer (native payload plus ~/.capsem/bin runtime copy)
    MacosPkg,
    /// Linux .deb installer (/usr/bin)
    LinuxDeb,
    /// Linux/macOS user-dir install (~/.capsem/bin)
    UserDir,
    /// Development build (cargo target directory)
    Development,
}

/// Detect the install layout from the current executable path.
/// Uses path component matching (not substring) to avoid false positives.
pub fn detect_install_layout() -> InstallLayout {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return InstallLayout::Development,
    };

    let macos_pkg_marker = cfg!(target_os = "macos")
        && std::path::Path::new("/usr/local/share/capsem/bin/capsem").is_file();
    detect_layout_from_path_with_macos_pkg_marker(&exe, macos_pkg_marker)
}

/// Testable core: detect layout from an arbitrary path.
#[cfg(test)]
fn detect_layout_from_path(exe: &std::path::Path) -> InstallLayout {
    detect_layout_from_path_with_macos_pkg_marker(exe, false)
}

fn detect_layout_from_path_with_macos_pkg_marker(
    exe: &std::path::Path,
    macos_pkg_marker: bool,
) -> InstallLayout {
    use std::path::Component;

    let components: Vec<_> = exe.components().collect();

    // Check for /usr/local/bin as consecutive path components
    let in_usr_local_bin = components.windows(3).any(|w| {
        matches!(
            (&w[0], &w[1], &w[2]),
            (
                Component::Normal(a),
                Component::Normal(b),
                Component::Normal(c),
            ) if *a == "usr" && *b == "local" && *c == "bin"
        )
    });
    if in_usr_local_bin {
        return InstallLayout::MacosPkg;
    }

    // Check for /usr/bin (Linux .deb installs here)
    let in_usr_bin = components.windows(2).any(|w| {
        matches!(
            (&w[0], &w[1]),
            (
                Component::Normal(a),
                Component::Normal(b),
            ) if *a == "usr" && *b == "bin"
        )
    });
    // Only match /usr/bin, not /usr/local/bin (already matched above)
    if in_usr_bin && !in_usr_local_bin {
        return InstallLayout::LinuxDeb;
    }

    // Check for .capsem/bin as consecutive path components
    let in_capsem_bin = components.windows(2).any(|w| {
        matches!(
            (&w[0], &w[1]),
            (
                Component::Normal(a),
                Component::Normal(b),
            ) if *a == ".capsem" && *b == "bin"
        )
    });
    if in_capsem_bin {
        return if macos_pkg_marker {
            InstallLayout::MacosPkg
        } else {
            InstallLayout::UserDir
        };
    }

    InstallLayout::Development
}

/// Return the install bin directory for the current layout.
pub fn install_bin_dir() -> Option<PathBuf> {
    match detect_install_layout() {
        InstallLayout::MacosPkg => Some(capsem_foundation::paths::capsem_bin_dir()),
        InstallLayout::LinuxDeb => Some(PathBuf::from("/usr/bin")),
        InstallLayout::UserDir => Some(capsem_foundation::paths::capsem_bin_dir()),
        InstallLayout::Development => None,
    }
}

#[cfg(test)]
mod tests;
