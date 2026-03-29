//! Static analysis tests for platform-specific API gating.
//!
//! These tests scan source files for macOS-only and Linux-only symbols
//! and verify they appear inside `#[cfg(target_os = "...")]` blocks.
//! This prevents compile failures when cross-compiling (e.g., Linux app builds).

use std::fs;
use std::path::Path;

/// Symbols that must only appear inside `#[cfg(target_os = "macos")]` blocks.
const MACOS_ONLY_SYMBOLS: &[&str] = &[
    "libc::clonefile",
    "AppleVzHypervisor",
    "core_foundation_sys::",
    "CFRunLoopRunInMode",
    "objc2_virtualization::",
    "objc2_foundation::",
    "pthread_main_np",
];

/// Symbols that must only appear inside `#[cfg(target_os = "linux")]` blocks.
const LINUX_ONLY_SYMBOLS: &[&str] = &[
    "KvmHypervisor",
    "FICLONE",
    "kvm_ioctls::",
    "/dev/kvm",
    "/dev/vhost-vsock",
    "ReflinkSnapshot",
];

/// Collect all .rs files under a directory recursively.
fn collect_rs_files(dir: &Path, files: &mut Vec<std::path::PathBuf>) {
    if !dir.is_dir() {
        return;
    }
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, files);
        } else if path.extension().is_some_and(|e| e == "rs") {
            files.push(path);
        }
    }
}

/// Check that a symbol only appears inside a cfg-gated context.
///
/// Strategy: for each line containing the symbol, walk backwards to find
/// the nearest `#[cfg(target_os = "...")]` or `#[cfg(not(target_os = "..."))]`.
/// The cfg must appear within 30 lines before the usage (covers struct/impl/fn
/// level gating).
///
/// Also accepts file-level `#![cfg(target_os = "...")]` as gating the entire file.
/// Also accepts module-level gating (the module declaration in mod.rs/lib.rs).
fn check_symbol_gated(
    files: &[std::path::PathBuf],
    symbol: &str,
    required_os: &str,
    module_gated_dirs: &[&str],
) -> Vec<String> {
    let mut violations = Vec::new();
    let cfg_pattern = format!("cfg(target_os = \"{}\")", required_os);

    for file in files {
        // Skip files inside module-gated directories.
        let file_str = file.to_string_lossy();
        if module_gated_dirs.iter().any(|d| file_str.contains(d)) {
            continue;
        }

        let content = fs::read_to_string(file).unwrap();
        let lines: Vec<&str> = content.lines().collect();

        // Check for file-level cfg gate.
        let file_gated = lines.iter().any(|l| {
            let trimmed = l.trim();
            trimmed.starts_with("#![cfg(") && trimmed.contains(&cfg_pattern)
        });
        if file_gated {
            continue;
        }

        for (i, line) in lines.iter().enumerate() {
            // Skip comments and string literals (log messages, debug!, warn!, etc.).
            let trimmed = line.trim();
            if trimmed.starts_with("//") || trimmed.starts_with("///") || trimmed.starts_with("*") {
                continue;
            }

            if !line.contains(symbol) {
                continue;
            }

            // Skip if the symbol only appears inside a string literal.
            // Simple heuristic: if the symbol is only inside quotes, skip it.
            let without_strings = {
                let mut s = line.to_string();
                // Remove double-quoted strings.
                while let Some(start) = s.find('"') {
                    if let Some(end) = s[start + 1..].find('"') {
                        s.replace_range(start..=start + 1 + end, "");
                    } else {
                        break;
                    }
                }
                s
            };
            if !without_strings.contains(symbol) {
                continue;
            }

            // Walk backwards up to 30 lines looking for a cfg gate.
            let start = i.saturating_sub(30);
            let mut gated = false;
            for j in (start..i).rev() {
                let prev = lines[j].trim();
                if prev.contains(&cfg_pattern) {
                    gated = true;
                    break;
                }
            }

            if !gated {
                violations.push(format!(
                    "{}:{}: `{}` not gated behind cfg(target_os = \"{}\")",
                    file.display(),
                    i + 1,
                    symbol,
                    required_os,
                ));
            }
        }
    }
    violations
}

#[test]
fn macos_symbols_are_gated() {
    let crates_dir = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut files = Vec::new();
    collect_rs_files(crates_dir, &mut files);

    // Directories whose module declarations are already cfg-gated.
    let macos_modules = &["hypervisor/apple_vz"];

    let mut all_violations = Vec::new();
    for symbol in MACOS_ONLY_SYMBOLS {
        let violations = check_symbol_gated(&files, symbol, "macos", macos_modules);
        all_violations.extend(violations);
    }

    assert!(
        all_violations.is_empty(),
        "Found ungated macOS-only symbols:\n{}",
        all_violations.join("\n"),
    );
}

#[test]
fn linux_symbols_are_gated() {
    let crates_dir = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut files = Vec::new();
    collect_rs_files(crates_dir, &mut files);

    // Directories whose module declarations are already cfg-gated.
    let linux_modules = &["hypervisor/kvm"];

    let mut all_violations = Vec::new();
    for symbol in LINUX_ONLY_SYMBOLS {
        let violations = check_symbol_gated(&files, symbol, "linux", linux_modules);
        all_violations.extend(violations);
    }

    assert!(
        all_violations.is_empty(),
        "Found ungated Linux-only symbols:\n{}",
        all_violations.join("\n"),
    );
}
