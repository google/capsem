use super::*;

fn make_map() -> HashMap<String, String> {
    let mut m = HashMap::new();
    m.insert("k".into(), "v".into());
    m
}

#[test]
fn setting_value_as_bool_returns_value_only_for_bool_variant() {
    assert_eq!(SettingValue::Bool(true).as_bool(), Some(true));
    assert_eq!(SettingValue::Bool(false).as_bool(), Some(false));
    assert_eq!(SettingValue::Number(1).as_bool(), None);
    assert_eq!(SettingValue::Text("x".into()).as_bool(), None);
}

#[test]
fn setting_value_as_number_returns_value_only_for_number_variant() {
    assert_eq!(SettingValue::Number(42).as_number(), Some(42));
    assert_eq!(SettingValue::Float(1.0).as_number(), None);
    assert_eq!(SettingValue::Text("42".into()).as_number(), None);
}

#[test]
fn setting_value_as_text_returns_borrowed_str() {
    assert_eq!(SettingValue::Text("hi".into()).as_text(), Some("hi"));
    assert_eq!(SettingValue::Bool(true).as_text(), None);
}

#[test]
fn setting_value_as_file_returns_tuple() {
    let v = SettingValue::File {
        path: "/tmp/x".into(),
        content: "body".into(),
    };
    assert_eq!(v.as_file(), Some(("/tmp/x", "body")));
    assert_eq!(SettingValue::Bool(true).as_file(), None);
}

#[test]
fn setting_value_as_float_accepts_number_and_float() {
    assert_eq!(SettingValue::Float(1.5).as_float(), Some(1.5));
    // Number -> float coercion.
    assert_eq!(SettingValue::Number(3).as_float(), Some(3.0));
    assert_eq!(SettingValue::Text("1.5".into()).as_float(), None);
}

#[test]
fn setting_value_list_accessors_return_slices() {
    let s = SettingValue::StringList(vec!["a".into(), "b".into()]);
    assert_eq!(s.as_string_list(), Some(&["a".to_string(), "b".to_string()][..]));
    assert_eq!(s.as_int_list(), None);
    assert_eq!(s.as_float_list(), None);

    let i = SettingValue::IntList(vec![1, 2]);
    assert_eq!(i.as_int_list(), Some(&[1i64, 2][..]));
    assert_eq!(i.as_string_list(), None);

    let f = SettingValue::FloatList(vec![1.0, 2.5]);
    assert_eq!(f.as_float_list(), Some(&[1.0f64, 2.5][..]));
    assert_eq!(f.as_int_list(), None);
}

#[test]
fn setting_value_as_kv_map_returns_map() {
    let m = make_map();
    let v = SettingValue::KvMap(m.clone());
    assert_eq!(v.as_kv_map(), Some(&m));
    assert_eq!(SettingValue::Bool(true).as_kv_map(), None);
}

#[test]
fn setting_value_deserializes_file_before_text() {
    // File variant must win over Text when input is a table.
    let toml = r#"path = "/etc/x"
content = "hello""#;
    let v: SettingValue = toml::from_str(toml).unwrap();
    match v {
        SettingValue::File { path, content } => {
            assert_eq!(path, "/etc/x");
            assert_eq!(content, "hello");
        }
        other => panic!("expected File variant, got {other:?}"),
    }
}

#[test]
fn setting_value_deserializes_string_list_before_text() {
    let v: SettingValue = toml::from_str("value = [\"a\", \"b\"]")
        .and_then(|t: toml::Value| toml::Value::try_into(t["value"].clone()))
        .unwrap();
    match v {
        SettingValue::StringList(list) => assert_eq!(list, vec!["a", "b"]),
        other => panic!("expected StringList, got {other:?}"),
    }
}

#[test]
fn default_true_helper_returns_true() {
    assert!(default_true());
}

#[test]
fn policy_source_default_is_default_variant() {
    assert_eq!(PolicySource::default(), PolicySource::Default);
}

#[test]
fn http_method_permissions_default_all_off() {
    let p = HttpMethodPermissions::default();
    assert!(!p.get && !p.post && !p.put && !p.delete && !p.other);
    assert!(p.domains.is_empty());
    assert!(p.path.is_none());
}

#[test]
fn settings_file_default_has_empty_settings_and_no_mcp() {
    let f = SettingsFile::default();
    assert!(f.settings.is_empty());
    assert!(f.mcp.is_none());
}

#[test]
fn setting_value_round_trips_through_json() {
    let cases = vec![
        SettingValue::Bool(true),
        SettingValue::Number(7),
        SettingValue::Float(2.5),
        SettingValue::Text("hello".into()),
        SettingValue::StringList(vec!["a".into()]),
        SettingValue::IntList(vec![1, 2, 3]),
        SettingValue::FloatList(vec![1.0, 2.0]),
        SettingValue::KvMap(make_map()),
        SettingValue::File {
            path: "/x".into(),
            content: "y".into(),
        },
    ];
    for v in cases {
        let j = serde_json::to_string(&v).unwrap();
        let back: SettingValue = serde_json::from_str(&j).unwrap();
        assert_eq!(v, back);
    }
}

#[test]
fn enum_variants_serialize_with_snake_case() {
    assert_eq!(serde_json::to_string(&SettingType::ApiKey).unwrap(), "\"apikey\"");
    assert_eq!(serde_json::to_string(&SettingType::KvMap).unwrap(), "\"kv_map\"");
    assert_eq!(
        serde_json::to_string(&Widget::PasswordInput).unwrap(),
        "\"password_input\""
    );
    assert_eq!(
        serde_json::to_string(&SideEffect::ToggleTheme).unwrap(),
        "\"toggle_theme\""
    );
    assert_eq!(
        serde_json::to_string(&ActionKind::CheckUpdate).unwrap(),
        "\"check_update\""
    );
    assert_eq!(serde_json::to_string(&McpTransport::Stdio).unwrap(), "\"stdio\"");
    assert_eq!(serde_json::to_string(&McpToolOrigin::InVm).unwrap(), "\"in_vm\"");
    assert_eq!(serde_json::to_string(&PolicySource::Corp).unwrap(), "\"corp\"");
}
