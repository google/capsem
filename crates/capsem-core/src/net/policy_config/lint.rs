//! Configuration linting with core-owned standard-location loading.

pub use capsem_config::{config_lint, ConfigIssue};

pub fn load_merged_lint() -> Vec<ConfigIssue> {
    let (user, corp) = super::loader::load_settings_and_corp_files();
    config_lint(&capsem_config::resolve_settings(&user, &corp))
}
