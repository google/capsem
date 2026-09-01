//! Settings tree construction with core-owned standard-location loading.

pub use capsem_config::{build_settings_tree, SettingsNode};

pub fn load_settings_tree() -> Vec<SettingsNode> {
    let (user, corp) = super::loader::load_settings_and_corp_files();
    build_settings_tree(&capsem_config::resolve_settings(&user, &corp))
}
