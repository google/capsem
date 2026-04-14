use std::path::PathBuf;

/// How capsem was installed.
#[derive(Debug, Clone, PartialEq)]
pub enum InstallLayout {
    /// macOS .pkg installer (/usr/local/bin)
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

    detect_layout_from_path(&exe)
}

/// Testable core: detect layout from an arbitrary path.
fn detect_layout_from_path(exe: &std::path::Path) -> InstallLayout {
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
        return InstallLayout::UserDir;
    }

    InstallLayout::Development
}

/// Return the install bin directory for the current layout.
pub fn install_bin_dir() -> Option<PathBuf> {
    match detect_install_layout() {
        InstallLayout::MacosPkg => Some(PathBuf::from("/usr/local/bin")),
        InstallLayout::LinuxDeb => Some(PathBuf::from("/usr/bin")),
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
    use std::path::Path;

    #[test]
    fn detect_returns_development_in_test() {
        let layout = detect_install_layout();
        assert_eq!(layout, InstallLayout::Development);
    }

    #[test]
    fn detect_macos_pkg_layout() {
        let path = Path::new("/usr/local/bin/capsem");
        assert_eq!(detect_layout_from_path(path), InstallLayout::MacosPkg);
    }

    #[test]
    fn detect_user_dir_layout() {
        let path = Path::new("/Users/elie/.capsem/bin/capsem");
        assert_eq!(detect_layout_from_path(path), InstallLayout::UserDir);
    }

    #[test]
    fn detect_user_dir_linux() {
        let path = Path::new("/home/user/.capsem/bin/capsem-service");
        assert_eq!(detect_layout_from_path(path), InstallLayout::UserDir);
    }

    #[test]
    fn detect_development_layout() {
        let path = Path::new("/Users/elie/git/capsem/target/debug/capsem");
        assert_eq!(detect_layout_from_path(path), InstallLayout::Development);
    }

    #[test]
    fn detect_linux_deb_layout() {
        let path = Path::new("/usr/bin/capsem");
        assert_eq!(detect_layout_from_path(path), InstallLayout::LinuxDeb);
    }

    #[test]
    fn detect_no_false_positive_on_substring() {
        // Path that contains "/usr/local/bin" as a substring of a component name
        let path = Path::new("/home/usr/local/bin-tools/capsem");
        // "bin-tools" != "bin", so this should NOT match MacosPkg
        assert_eq!(detect_layout_from_path(path), InstallLayout::Development);
    }

    #[test]
    fn detect_no_false_positive_capsem_in_name() {
        // ".capsem" appears but not followed by "bin" component
        let path = Path::new("/home/user/.capsem/data/capsem");
        assert_eq!(detect_layout_from_path(path), InstallLayout::Development);
    }

    #[test]
    fn install_bin_dir_development_returns_none() {
        // In test context we're Development
        assert_eq!(install_bin_dir(), None);
    }
}
