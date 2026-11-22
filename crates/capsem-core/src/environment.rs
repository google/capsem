//! Environment resolution and application semantics live below service jobs.
//! Comparing bounded typed content never confers caller or revision authority.
use capsem_config::environment::EnvironmentSpecification;

mod plugins;
mod policy_inputs;
mod runtime_policy;
pub use plugins::{
    CompiledEnvironmentPlugins, CompiledPluginConfiguration, CompiledPluginOptions, EnvironmentPluginSource,
    PluginDirection,
};
pub use policy_inputs::EnvironmentPolicyInputs;
pub use runtime_policy::EnvironmentRuntimePolicy;
mod catalog_contents;
mod catalog_rules;
mod catalog_sources;
pub use catalog_contents::{
    CatalogContentEntry, CatalogContentError, CatalogContents, RuleCatalogContents, SkillCatalogContents,
};
pub use catalog_rules::RuleCatalogInputs;
mod mcp_launch;
mod mcp_permissions;
pub use mcp_launch::{validate_environment_mcp_launch, SandboxMcp};
mod repositories;
mod resolution;
pub use repositories::{
    RepositoryAdmission, RepositoryHttpsResolver, RepositoryNetworkAuthority, RepositoryResolutionError,
};
mod rules;
pub use catalog_sources::effective_catalog_source;
pub(crate) use catalog_sources::validate_corporate_catalog_sources;
pub use resolution::{
    resolve_environment, EnvironmentPinResolver, EnvironmentResolutionError, EnvironmentResolutionMode,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvironmentChangeKind {
    Name,
    Description,
    Image,
    Startup,
    Variables,
    SecretAccess,
    Repositories,
    PackageAccess,
    AccessRules,
    DetectionRules,
    Plugins,
    McpDefinitions,
    McpPermissions,
    Skills,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvironmentChangeTiming {
    Metadata,
    Live,
    NextStart,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EnvironmentChange {
    pub kind: EnvironmentChangeKind,
    pub timing: EnvironmentChangeTiming,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EnvironmentChangeImpact {
    changes: Vec<EnvironmentChange>,
}
impl EnvironmentChangeImpact {
    /// Classify validated resolved content, independently of revision identity.
    /// Corporate changes are a separate authority input to policy compilation.
    /// This is a change-review result, never a runtime application receipt.
    pub fn classify(before: &EnvironmentSpecification, after: &EnvironmentSpecification) -> Self {
        use EnvironmentChangeKind::*;
        use EnvironmentChangeTiming::*;
        let fields = [
            (Name, Metadata, before.name != after.name),
            (Description, Metadata, before.description != after.description),
            (Image, NextStart, before.image != after.image),
            (Startup, NextStart, before.startup != after.startup),
            (Variables, NextStart, before.variables != after.variables),
            (SecretAccess, Live, before.secrets != after.secrets),
            (Repositories, NextStart, before.repositories != after.repositories),
            (PackageAccess, Live, before.packages != after.packages),
            (AccessRules, Live, before.access != after.access),
            (DetectionRules, NextStart, before.detection != after.detection),
            (Plugins, NextStart, before.plugins != after.plugins),
            (
                McpDefinitions,
                NextStart,
                before.mcps.catalog != after.mcps.catalog
                    || before.mcps.catalog_configuration != after.mcps.catalog_configuration
                    || before.mcps.servers != after.mcps.servers,
            ),
            (McpPermissions, Live, before.mcps.permissions != after.mcps.permissions),
            (Skills, NextStart, before.skills != after.skills),
        ];
        Self {
            changes: fields
                .into_iter()
                .filter_map(|(kind, timing, changed)| changed.then_some(EnvironmentChange { kind, timing }))
                .collect(),
        }
    }
    pub fn changes(&self) -> &[EnvironmentChange] {
        &self.changes
    }
    pub fn has_deferred_changes(&self) -> bool {
        self.changes
            .iter()
            .any(|change| change.timing == EnvironmentChangeTiming::NextStart)
    }
    pub fn affects_live_policy(&self) -> bool {
        self.changes
            .iter()
            .any(|change| change.timing == EnvironmentChangeTiming::Live)
    }
}

#[cfg(test)]
mod tests;
