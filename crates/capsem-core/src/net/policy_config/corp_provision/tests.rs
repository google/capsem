use super::*;

#[test]
fn test_validate_valid_corp_toml() {
    let content = r#"
[settings]
"repository.providers.github.allow" = { value = true, modified = "2024-01-01T00:00:00Z" }
"#;
    let result = validate_corp_toml(content);
    assert!(result.is_ok());
    let file = result.unwrap();
    assert!(file.settings.contains_key("repository.providers.github.allow"));
}

#[test]
fn test_validate_rejects_retired_ai_settings() {
    let content = r#"
[settings]
"ai.anthropic.allow" = { value = true, modified = "2024-01-01T00:00:00Z" }
"#;
    let error = validate_corp_toml(content).unwrap_err().to_string();
    assert!(error.contains("retired AI setting id ai.anthropic.allow"));
}

#[test]
fn test_validate_empty_corp_toml() {
    let result = validate_corp_toml("");
    assert!(result.is_ok());
    assert!(result.unwrap().settings.is_empty());
}

#[test]
fn test_validate_invalid_toml_syntax() {
    let result = validate_corp_toml("this is not [ valid toml {{{");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("invalid corp TOML"));
}

#[test]
fn test_validate_toml_with_unknown_keys() {
    let content = r#"
[settings]
"future.setting.v99" = { value = "hello", modified = "2024-01-01T00:00:00Z" }
"#;
    assert!(validate_corp_toml(content).is_ok());
}

#[test]
fn test_validate_toml_wrong_types() {
    // Raw string without SettingEntry wrapper should fail
    let content = r#"
[settings]
"repository.providers.github.allow" = "yes"
"#;
    assert!(validate_corp_toml(content).is_err());
}

#[test]
fn test_refresh_interval_parsing() {
    assert_eq!(parse_refresh_interval("refresh_policy = \"12h\"\n\n[settings]\n"), 12);
    assert_eq!(parse_refresh_interval("[settings]\n"), DEFAULT_REFRESH_INTERVAL_HOURS);
}

#[test]
fn test_refresh_interval_zero_means_no_refresh() {
    assert_eq!(parse_refresh_interval("refresh_policy = \"0h\"\n\n[settings]\n"), 0);
}

#[test]
fn test_corp_source_roundtrip() {
    let source = CorpSource {
        url: Some("https://example.com/corp.toml".into()),
        file_path: None,
        fetched_at: 1718444400,
        etag: Some("\"abc123\"".into()),
        content_hash: "a".repeat(64),
        refresh_interval_hours: 12,
    };
    let json = serde_json::to_string(&source).unwrap();
    let rt: CorpSource = serde_json::from_str(&json).unwrap();
    assert_eq!(rt.url, source.url);
    assert_eq!(rt.etag, source.etag);
    assert_eq!(rt.refresh_interval_hours, 12);
    assert_eq!(rt.fetched_at, 1718444400);
    assert!(rt.file_path.is_none());
}

fn tmp_dir() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

fn sample_source() -> CorpSource {
    CorpSource {
        url: Some("https://example.com/corp.toml".into()),
        file_path: None,
        fetched_at: 1_718_000_000,
        etag: None,
        content_hash: "h".repeat(64),
        refresh_interval_hours: 6,
    }
}

#[tokio::test]
async fn fetch_corp_config_rejects_bare_file_path_source() {
    let dir = tmp_dir();
    let corp_path = dir.path().join("corp.toml");
    std::fs::write(&corp_path, "refresh_policy = \"6h\"\n").unwrap();
    let client = reqwest::Client::new();

    let error = fetch_corp_config(&client, &corp_path.display().to_string())
        .await
        .unwrap_err();

    assert!(
        format!("{error:#}").contains("corp config source must be a URL"),
        "{error:#}"
    );
}

#[tokio::test]
async fn fetch_corp_config_rejects_file_url_shorthand_path_source() {
    let client = reqwest::Client::new();

    let error = fetch_corp_config(&client, "file:corp.toml").await.unwrap_err();

    assert!(
        format!("{error:#}").contains("corp config file URL must start with file://"),
        "{error:#}"
    );
}

#[tokio::test]
async fn fetch_corp_config_accepts_file_url_source() {
    let dir = tmp_dir();
    let corp_path = dir.path().join("corp.toml");
    std::fs::write(&corp_path, "refresh_policy = \"6h\"\n").unwrap();
    let client = reqwest::Client::new();
    let source = format!("file://{}", corp_path.display());

    let (body, etag) = fetch_corp_config(&client, &source)
        .await
        .expect("file URL corp config fetches");

    assert_eq!(body, "refresh_policy = \"6h\"\n");
    assert!(etag.is_none());
}

#[test]
fn parse_refresh_interval_rejects_negative() {
    // Negative values must fall back to the default rather than wrap.
    let content = "refresh_policy = \"-5h\"\n";
    assert_eq!(parse_refresh_interval(content), DEFAULT_REFRESH_INTERVAL_HOURS);
}

#[test]
fn parse_refresh_interval_ignores_wrong_type() {
    let content = "refresh_policy = \"twelve\"\n";
    assert_eq!(parse_refresh_interval(content), DEFAULT_REFRESH_INTERVAL_HOURS);
}

#[test]
fn parse_refresh_interval_on_invalid_toml_returns_default() {
    assert_eq!(parse_refresh_interval("{{ not toml"), DEFAULT_REFRESH_INTERVAL_HOURS);
}

#[test]
fn install_corp_config_writes_both_files_and_creates_dir() {
    let dir = tmp_dir();
    let nested = dir.path().join("capsem-home");
    let source = sample_source();
    install_corp_config(&nested, "refresh_policy = \"6h\"\n", &source).unwrap();

    assert!(nested.join("corp.toml").exists());
    assert!(nested.join("corp-source.json").exists());

    let corp = std::fs::read_to_string(nested.join("corp.toml")).unwrap();
    assert!(corp.contains("refresh_policy = \"6h\""));

    let roundtrip: CorpSource =
        serde_json::from_str(&std::fs::read_to_string(nested.join("corp-source.json")).unwrap()).unwrap();
    assert_eq!(roundtrip.refresh_interval_hours, 6);
}

#[test]
fn read_corp_source_missing_returns_none() {
    let dir = tmp_dir();
    assert!(read_corp_source(dir.path()).is_none());
}

#[test]
fn read_corp_source_invalid_json_returns_none() {
    let dir = tmp_dir();
    std::fs::write(dir.path().join("corp-source.json"), "not json").unwrap();
    assert!(read_corp_source(dir.path()).is_none());
}

#[test]
fn read_corp_source_roundtrips_installed_data() {
    let dir = tmp_dir();
    let source = sample_source();
    install_corp_config(dir.path(), "", &source).unwrap();
    let got = read_corp_source(dir.path()).unwrap();
    assert_eq!(got.url, source.url);
    assert_eq!(got.refresh_interval_hours, source.refresh_interval_hours);
    assert_eq!(got.content_hash, source.content_hash);
}

#[test]
fn install_inline_corp_config_validates_and_writes() {
    let dir = tmp_dir();
    let content = "refresh_policy = \"3h\"\n\n[settings]\n";
    install_inline_corp_config(dir.path(), content).unwrap();

    let src = read_corp_source(dir.path()).unwrap();
    assert!(src.url.is_none());
    assert!(src.file_path.is_none());
    assert_eq!(src.refresh_interval_hours, 3);
    assert_eq!(src.content_hash.len(), 64); // blake3 hex
}

#[test]
fn install_inline_corp_config_rejects_invalid_toml() {
    let dir = tmp_dir();
    let err = install_inline_corp_config(dir.path(), "this is [ broken").unwrap_err();
    assert!(err.to_string().contains("invalid corp TOML"));
    assert!(!dir.path().join("corp.toml").exists());
}

#[tokio::test]
async fn refresh_noops_when_no_source_file() {
    let dir = tmp_dir();
    // No corp-source.json exists; must not panic or create anything.
    refresh_corp_config_if_stale(dir.path().to_path_buf()).await;
    assert!(!dir.path().join("corp.toml").exists());
}

#[tokio::test]
async fn refresh_noops_when_provisioned_from_file() {
    let dir = tmp_dir();
    let mut source = sample_source();
    source.url = None;
    source.file_path = Some("/tmp/local.toml".into());
    install_corp_config(dir.path(), "[settings]\n", &source).unwrap();

    refresh_corp_config_if_stale(dir.path().to_path_buf()).await;
    // corp.toml must remain untouched.
    let body = std::fs::read_to_string(dir.path().join("corp.toml")).unwrap();
    assert_eq!(body, "[settings]\n");
}

#[tokio::test]
async fn refresh_noops_when_interval_zero() {
    let dir = tmp_dir();
    let mut source = sample_source();
    source.refresh_interval_hours = 0;
    source.fetched_at = 0; // Ancient — but interval=0 disables refresh.
    install_corp_config(dir.path(), "[settings]\n", &source).unwrap();

    refresh_corp_config_if_stale(dir.path().to_path_buf()).await;
    let body = std::fs::read_to_string(dir.path().join("corp.toml")).unwrap();
    assert_eq!(body, "[settings]\n");
}

#[tokio::test]
async fn refresh_noops_when_not_yet_stale() {
    let dir = tmp_dir();
    let mut source = sample_source();
    source.fetched_at = now_secs(); // Fresh — TTL not expired.
    source.refresh_interval_hours = 24;
    install_corp_config(dir.path(), "[settings]\n", &source).unwrap();

    refresh_corp_config_if_stale(dir.path().to_path_buf()).await;
    // Body still the original; no network call attempted.
    let body = std::fs::read_to_string(dir.path().join("corp.toml")).unwrap();
    assert_eq!(body, "[settings]\n");
}
