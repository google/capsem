use super::*;

#[test]
fn identifiers_share_one_length_and_character_contract() {
    assert!(validate_identifier("fixture id", "alpha-2_beta").is_ok());
    assert!(validate_identifier("fixture id", &"a".repeat(64)).is_ok());

    for invalid in ["", " ", "Upper", "dot.id", &"a".repeat(65)] {
        let error = validate_identifier("fixture id", invalid).unwrap_err();
        assert!(error.contains("fixture id"), "{error}");
    }
}

#[test]
fn policy_targets_share_one_traversal_and_padding_contract() {
    assert!(validate_policy_target("fixture target", "server/tool.name").is_ok());
    assert!(validate_policy_target("fixture target", &"a".repeat(128)).is_ok());

    for invalid in ["", " ", "../tool", "server\\tool", " padded", &"a".repeat(129)] {
        let error = validate_policy_target("fixture target", invalid).unwrap_err();
        assert!(error.contains("fixture target"), "{error}");
    }
}

fn definition(
    setting_type: crate::types::SettingType,
    metadata: crate::types::SettingMetadata,
) -> crate::types::SettingDef {
    crate::types::SettingDef {
        id: "fixture.setting".into(),
        category: "fixture".into(),
        name: "Fixture".into(),
        description: String::new(),
        setting_type,
        default_value: crate::types::SettingValue::Bool(false),
        enabled_by: None,
        metadata,
    }
}

#[test]
fn a_setting_value_must_have_its_definitions_type() {
    use crate::types::{SettingType as T, SettingValue as V};
    let accepted = [
        (T::Bool, V::Bool(true)),
        (T::Number, V::Number(3)),
        (T::Text, V::Text("x".into())),
        (T::Url, V::Text("https://example.com".into())),
        (T::Email, V::Text("a@example.com".into())),
        (T::ApiKey, V::Text("credential:blake3:00".into())),
        (T::StringList, V::StringList(vec!["a".into()])),
        (T::IntList, V::IntList(vec![1])),
        (T::FloatList, V::FloatList(vec![1.5])),
        (T::KvMap, V::KvMap(Default::default())),
        (
            T::File,
            V::File {
                path: "/root/a".into(),
                content: String::new(),
            },
        ),
    ];
    for (setting_type, value) in accepted {
        let def = definition(setting_type, Default::default());
        assert!(
            validate_setting_value_shape(&def, &value).is_ok(),
            "{setting_type:?} refused {value:?}"
        );
    }
    for (setting_type, value) in [
        (T::Bool, V::Text("true".into())),
        (T::Number, V::Bool(true)),
        (T::Text, V::Number(1)),
        (T::StringList, V::Text("a".into())),
        (T::KvMap, V::StringList(vec![])),
        (T::File, V::Text("/root/a".into())),
    ] {
        let def = definition(setting_type, Default::default());
        let error = validate_setting_value_shape(&def, &value).unwrap_err();
        assert!(error.contains("fixture.setting"), "{error}");
    }
}

#[test]
fn a_number_stays_in_range_and_a_choice_among_its_choices() {
    use crate::types::{SettingMetadata, SettingType as T, SettingValue as V};
    let ranged = definition(
        T::Number,
        SettingMetadata {
            min: Some(1),
            max: Some(8),
            ..Default::default()
        },
    );
    assert!(validate_setting_value_shape(&ranged, &V::Number(4)).is_ok());
    assert!(validate_setting_value_shape(&ranged, &V::Number(0))
        .unwrap_err()
        .contains("minimum"));
    assert!(validate_setting_value_shape(&ranged, &V::Number(9))
        .unwrap_err()
        .contains("maximum"));

    let choice = definition(
        T::Text,
        SettingMetadata {
            choices: vec!["dark".into(), "light".into()],
            ..Default::default()
        },
    );
    assert!(validate_setting_value_shape(&choice, &V::Text("dark".into())).is_ok());
    assert!(validate_setting_value_shape(&choice, &V::Text("pink".into()))
        .unwrap_err()
        .contains("one of"));
}
