use std::path::PathBuf;

/// How capsem was installed.
#[derive(Debug, Clone, PartialEq)]
pub enum InstallLayout {
    /// macOS .pkg installer (native payload plus ~/.capsem/bin runtime copy)
    MacosPkg,
    /// Linux .deb installer (/usr/bin)
    LinuxDeb,
    /// Linux, installed to /usr/bin by a distribution's own package manager
    /// on a host without apt. That package manager updates the programs;
    /// the updater still refreshes the VM assets.
    SystemPackage,
    /// Linux/macOS user-dir install (~/.capsem/bin)
    UserDir,
    /// Development build (cargo target directory)
    Development,
}

impl InstallLayout {
    /// What to tell the user when the release offers this install no installer.
    pub fn no_installer_message(&self) -> String {
        match self {
            Self::SystemPackage => "This Capsem was installed by your system's package manager, which \
                 also updates it. VM assets are still updated here."
                .to_string(),
            _ => format!("No installer package in release manifest matches this install layout ({self:?})."),
        }
    }
}

/// Detect the install layout from the current executable path.
/// Uses path component matching (not substring) to avoid false positives.
pub fn detect_install_layout() -> InstallLayout {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return InstallLayout::Development,
    };

    let macos_pkg_marker =
        cfg!(target_os = "macos") && std::path::Path::new("/usr/local/share/capsem/bin/capsem").is_file();
    let layout = detect_layout_from_path_with_macos_pkg_marker(&exe, macos_pkg_marker);
    layout_for_package_manager(layout, std::path::Path::new(APT_GET).is_file())
}

/// The apt the .deb update is installed with.
const APT_GET: &str = "/usr/bin/apt-get";

/// A path under /usr/bin only means a .deb install where apt exists. Other
/// distributions package Capsem there too, and the .deb the release channel
/// offers cannot be installed on them.
fn layout_for_package_manager(layout: InstallLayout, apt_get_present: bool) -> InstallLayout {
    match layout {
        InstallLayout::LinuxDeb if !apt_get_present => InstallLayout::SystemPackage,
        other => other,
    }
}

/// Testable core: detect layout from an arbitrary path.
#[cfg(test)]
fn detect_layout_from_path(exe: &std::path::Path) -> InstallLayout {
    detect_layout_from_path_with_macos_pkg_marker(exe, false)
}

fn detect_layout_from_path_with_macos_pkg_marker(exe: &std::path::Path, macos_pkg_marker: bool) -> InstallLayout {
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
    install_bin_dir_for(&detect_install_layout())
}

/// The bin directory a package of this layout installs into.
pub fn install_bin_dir_for(layout: &InstallLayout) -> Option<PathBuf> {
    match layout {
        InstallLayout::MacosPkg => Some(capsem_foundation::paths::capsem_bin_dir()),
        InstallLayout::LinuxDeb => Some(PathBuf::from("/usr/bin")),
        InstallLayout::UserDir => Some(capsem_foundation::paths::capsem_bin_dir()),
        InstallLayout::SystemPackage | InstallLayout::Development => None,
    }
}

#[cfg(test)]
mod tests;
