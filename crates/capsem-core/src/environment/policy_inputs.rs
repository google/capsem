//! Select policy sections without publishing desired workload configuration.
use capsem_config::{environment::*, plugins::EnvironmentPlugins, SettingsFile};
use std::collections::BTreeMap;

/// Borrowed compilation inputs for an acknowledged boot/live composition.
///
/// The service verifies revision ownership, generation and current corporate
/// authority before constructing this view. This selection has no authority
/// of its own and performs no I/O. Compilation still validates and resolves the
/// selected sections, including mandatory corporate constraints and catalog
/// availability. Desired settings are deliberately not an input here.
#[derive(Clone, Copy)]
pub struct EnvironmentPolicyInputs<'a> {
    access: &'a EnvironmentAccessRules,
    secrets: &'a BTreeMap<String, EnvironmentSecretBinding>,
    packages: &'a EnvironmentPackages,
    mcp_permissions: &'a BTreeMap<String, EnvironmentMcpPermissions>,
    detection: &'a EnvironmentDetectionRules,
    plugins: &'a EnvironmentPlugins,
    mcp_catalog: &'a EnvironmentCatalogSelections<EnvironmentCatalogPin>,
    mcp_servers: &'a BTreeMap<String, EnvironmentMcpDefinition>,
    corporate: &'a SettingsFile,
}
impl<'a> EnvironmentPolicyInputs<'a> {
    pub fn new(
        boot: &'a EnvironmentSpecification,
        live: &'a EnvironmentSpecification,
        corporate: &'a SettingsFile,
    ) -> Self {
        Self {
            access: &live.access,
            secrets: &live.secrets,
            packages: &live.packages,
            mcp_permissions: &live.mcps.permissions,
            detection: &boot.detection,
            plugins: &boot.plugins,
            mcp_catalog: &boot.mcps.catalog,
            mcp_servers: &boot.mcps.servers,
            corporate,
        }
    }
    pub fn access_rules(&self) -> &'a EnvironmentAccessRules {
        self.access
    }
    pub fn secret_access(&self) -> &'a BTreeMap<String, EnvironmentSecretBinding> {
        self.secrets
    }
    pub fn package_access(&self) -> &'a EnvironmentPackages {
        self.packages
    }
    pub fn mcp_permissions(&self) -> &'a BTreeMap<String, EnvironmentMcpPermissions> {
        self.mcp_permissions
    }
    pub fn detection_rules(&self) -> &'a EnvironmentDetectionRules {
        self.detection
    }
    pub fn plugins(&self) -> &'a EnvironmentPlugins {
        self.plugins
    }
    pub fn mcp_catalog(&self) -> &'a EnvironmentCatalogSelections<EnvironmentCatalogPin> {
        self.mcp_catalog
    }
    pub fn mcp_servers(&self) -> &'a BTreeMap<String, EnvironmentMcpDefinition> {
        self.mcp_servers
    }
    pub fn corporate(&self) -> &'a SettingsFile {
        self.corporate
    }
}
