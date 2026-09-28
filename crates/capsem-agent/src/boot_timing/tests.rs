use super::*;

#[test]
fn parse_boot_timing_valid_jsonl() {
    let dir = std::env::temp_dir();
    let path = dir.join("capsem-test-boot-timing");
    std::fs::write(
        &path,
        "{\"name\":\"squashfs\",\"duration_ms\":50}\n{\"name\":\"network\",\"duration_ms\":120}\n",
    )
    .unwrap();
    let result = parse_boot_timing(path.to_str().unwrap());
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].name, "squashfs");
    assert_eq!(result[0].duration_ms, 50);
    assert_eq!(result[1].name, "network");
    assert_eq!(result[1].duration_ms, 120);
    std::fs::remove_file(&path).ok();
}

/// capsem-init records per-stage steal for the in-guest doctor only; the
/// vsock `BootStage` does not carry it. The field -- sane or hostile -- must
/// not cost the host its stage report.
#[test]
fn parse_boot_timing_ignores_steal_field() {
    let dir = std::env::temp_dir();
    let path = dir.join("capsem-test-boot-timing-steal");
    std::fs::write(
        &path,
        concat!(
            "{\"name\":\"kernel\",\"duration_ms\":900,\"steal_ms\":12}\n",
            "{\"name\":\"network\",\"duration_ms\":510,\"steal_ms\":-4}\n",
            "{\"name\":\"deploy\",\"duration_ms\":30,\"steal_ms\":\"<script>\"}\n",
            "{\"name\":\"venv\",\"duration_ms\":20,\"steal_ms\":{\"nested\":[1,2]}}\n",
        ),
    )
    .unwrap();
    let result = parse_boot_timing(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();
    let got: Vec<(&str, u64)> = result.iter().map(|s| (s.name.as_str(), s.duration_ms)).collect();
    assert_eq!(got, [("kernel", 900), ("network", 510), ("deploy", 30), ("venv", 20)]);
}

#[test]
fn parse_boot_timing_missing_file() {
    let result = parse_boot_timing("/nonexistent/capsem-boot-timing");
    assert!(result.is_empty());
}

#[test]
fn parse_boot_timing_skips_malformed_lines() {
    let dir = std::env::temp_dir();
    let path = dir.join("capsem-test-boot-timing-bad");
    std::fs::write(
        &path,
        "{\"name\":\"good\",\"duration_ms\":100}\nnot json\n{\"name\":\"also_good\",\"duration_ms\":200}\n",
    )
    .unwrap();
    let result = parse_boot_timing(path.to_str().unwrap());
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].name, "good");
    assert_eq!(result[1].name, "also_good");
    std::fs::remove_file(&path).ok();
}

#[test]
fn parse_boot_timing_rejects_xss_names() {
    let dir = std::env::temp_dir();
    let path = dir.join("capsem-test-boot-timing-xss");
    std::fs::write(
        &path,
        concat!(
            "{\"name\":\"<script>alert(1)</script>\",\"duration_ms\":10}\n",
            "{\"name\":\"normal\",\"duration_ms\":20}\n",
            "{\"name\":\"a]};fetch('http://evil')\",\"duration_ms\":30}\n",
            "{\"name\":\"\",\"duration_ms\":40}\n",
            "{\"name\":\"has spaces\",\"duration_ms\":50}\n",
            "{\"name\":\"path/../traversal\",\"duration_ms\":60}\n",
        ),
    )
    .unwrap();
    let result = parse_boot_timing(path.to_str().unwrap());
    assert_eq!(result.len(), 1, "only 'normal' should survive: {result:?}");
    assert_eq!(result[0].name, "normal");
    std::fs::remove_file(&path).ok();
}

#[test]
fn parse_boot_timing_rejects_huge_duration() {
    let dir = std::env::temp_dir();
    let path = dir.join("capsem-test-boot-timing-huge");
    std::fs::write(
        &path,
        concat!(
            "{\"name\":\"ok\",\"duration_ms\":1000}\n",
            "{\"name\":\"huge\",\"duration_ms\":999999999}\n",
        ),
    )
    .unwrap();
    let result = parse_boot_timing(path.to_str().unwrap());
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].name, "ok");
    std::fs::remove_file(&path).ok();
}

#[test]
fn parse_boot_timing_caps_at_32_entries() {
    let dir = std::env::temp_dir();
    let path = dir.join("capsem-test-boot-timing-cap");
    let mut lines = String::new();
    for i in 0..50 {
        use std::fmt::Write as _;
        writeln!(lines, "{{\"name\":\"stage{i}\",\"duration_ms\":{i}}}").unwrap();
    }
    std::fs::write(&path, &lines).unwrap();
    let result = parse_boot_timing(path.to_str().unwrap());
    assert_eq!(result.len(), 32);
    std::fs::remove_file(&path).ok();
}

#[test]
fn parse_boot_timing_empty_file() {
    let dir = std::env::temp_dir();
    let path = dir.join("capsem-test-boot-timing-empty");
    std::fs::write(&path, "").unwrap();
    let result = parse_boot_timing(path.to_str().unwrap());
    assert!(result.is_empty());
    std::fs::remove_file(&path).ok();
}

#[test]
fn parse_boot_timing_rejects_long_names() {
    let dir = std::env::temp_dir();
    let path = dir.join("capsem-test-boot-timing-longname");
    let long_name = "a".repeat(65);
    std::fs::write(
        &path,
        format!(
            "{{\"name\":\"{long_name}\",\"duration_ms\":10}}\n\
             {{\"name\":\"ok\",\"duration_ms\":20}}\n"
        ),
    )
    .unwrap();
    let result = parse_boot_timing(path.to_str().unwrap());
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].name, "ok");
    std::fs::remove_file(&path).ok();
}

#[test]
fn parse_boot_timing_name_at_exact_boundary() {
    let dir = std::env::temp_dir();
    let path = dir.join("capsem-test-boot-timing-boundary");
    let name_64 = "a".repeat(64); // exactly at limit, should pass
    std::fs::write(&path, format!("{{\"name\":\"{name_64}\",\"duration_ms\":10}}\n")).unwrap();
    let result = parse_boot_timing(path.to_str().unwrap());
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].name, name_64);
    std::fs::remove_file(&path).ok();
}

#[test]
fn parse_boot_timing_duration_at_exact_boundary() {
    let dir = std::env::temp_dir();
    let path = dir.join("capsem-test-boot-timing-dur-boundary");
    // 600_000 is exactly at limit, should pass
    std::fs::write(&path, "{\"name\":\"ok\",\"duration_ms\":600000}\n").unwrap();
    let result = parse_boot_timing(path.to_str().unwrap());
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].duration_ms, 600_000);

    // 600_001 is over limit, should be rejected
    std::fs::write(&path, "{\"name\":\"bad\",\"duration_ms\":600001}\n").unwrap();
    let result = parse_boot_timing(path.to_str().unwrap());
    assert!(result.is_empty());
    std::fs::remove_file(&path).ok();
}
