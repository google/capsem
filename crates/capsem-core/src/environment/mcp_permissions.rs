//! Apply live permissions to acknowledged server definitions, never desired ones.
use super::EnvironmentPolicyInputs;
use capsem_config::{environment::EnvironmentMcpPermissions, CompiledSecurityRule};
use std::collections::BTreeSet;

impl EnvironmentPolicyInputs<'_> {
    /// Compile only the boot snapshot's configured IDs and the reserved builtin.
    /// Permission changes are live; a permission for a new deferred definition
    /// cannot add that server to the running scope. Missing permission denies.
    /// Core's complete policy compiler must compose these rules with current
    /// corporate constraints; transport, enablement and pin gates remain separate.
    pub fn compile_mcp_permissions(&self) -> Result<Vec<CompiledSecurityRule>, String> {
        if self
            .mcp_servers()
            .len()
            .saturating_add(self.mcp_catalog().selected.len())
            > capsem_proto::ENVIRONMENT_MAX_MCP_SERVERS
            || self.mcp_permissions().len() > capsem_proto::ENVIRONMENT_MAX_MCP_SERVERS + 1
        {
            return Err("too many environment MCP policy entries".into());
        }
        let mut servers = BTreeSet::new();
        for id in self
            .mcp_servers()
            .keys()
            .map(String::as_str)
            .chain(self.mcp_catalog().selected.iter().map(|pin| pin.id.as_str()))
        {
            if id == capsem_proto::mcp_contracts::BUILTIN_SERVER_NAME || !servers.insert(id) {
                return Err("reserved or duplicate environment MCP server identity".into());
            }
        }
        servers.insert(capsem_proto::mcp_contracts::BUILTIN_SERVER_NAME);
        let default = EnvironmentMcpPermissions::default();
        let mut rules = Vec::new();
        for server in servers {
            let permissions = self.mcp_permissions().get(server).unwrap_or(&default);
            rules.extend(capsem_config::compile_environment_mcp_permissions(server, permissions)?);
        }
        Ok(rules)
    }
}
