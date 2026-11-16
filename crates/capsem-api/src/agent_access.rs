//! Caller evidence and scoped access contracts. Actor evidence is produced by
//! trusted authentication, never deserialized from a request or caller header.
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Authenticated provenance for mutations and approval decisions. Service
/// extractors require their own sealed authentication proof, not this DTO.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CallerActor {
    Operator,
    DelegatedAgent {
        access_id: String,
    },
    VmOwner {
        session_id: String,
        owner_generation: String,
    },
    Corporate,
}

impl CallerActor {
    pub fn ledger_id(&self) -> String {
        match self {
            Self::Operator => "operator".into(),
            Self::DelegatedAgent { access_id } => format!("agent:{access_id}"),
            Self::VmOwner {
                session_id,
                owner_generation,
            } => format!("vm:{session_id}/{owner_generation}"),
            Self::Corporate => "corporate".into(),
        }
    }
}

/// These are the only delegable operations. Each still requires target
/// visibility/ownership; none conveys approval or configuration authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentOperation {
    Discover,
    Inspect,
    Propose,
    CancelOwn,
    Execute,
    Launch,
    ManageOwnedSessions,
}

/// Exact existing generation; deletion/name reuse cannot retarget a grant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(try_from = "SessionWire")]
pub struct AgentSessionScope {
    session_id: String,
    owner_generation: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionWire {
    session_id: String,
    owner_generation: String,
}

impl AgentSessionScope {
    pub fn new(session_id: String, owner_generation: String) -> Result<Self, &'static str> {
        if !crate::identity::canonical_uuid(&session_id) || !crate::identity::canonical_uuid(&owner_generation) {
            return Err("agent session scope requires canonical session and generation UUIDs");
        }
        Ok(Self {
            session_id,
            owner_generation,
        })
    }
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
    pub fn owner_generation(&self) -> &str {
        &self.owner_generation
    }
}

impl TryFrom<SessionWire> for AgentSessionScope {
    type Error = &'static str;
    fn try_from(wire: SessionWire) -> Result<Self, Self::Error> {
        Self::new(wire.session_id, wire.owner_generation)
    }
}

/// Empty target lists convey no target access; there is no wildcard or default
/// environment. New sessions launched within this scope are bound by the
/// service to this access id and their actual generation before use.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(try_from = "ScopeWire")]
pub struct AgentAccessScope {
    operations: Vec<AgentOperation>,
    environment_ids: Vec<String>,
    sessions: Vec<AgentSessionScope>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScopeWire {
    operations: Vec<AgentOperation>,
    #[serde(default)]
    environment_ids: Vec<String>,
    #[serde(default)]
    sessions: Vec<AgentSessionScope>,
}

impl AgentAccessScope {
    pub fn new(
        operations: Vec<AgentOperation>,
        environment_ids: Vec<String>,
        sessions: Vec<AgentSessionScope>,
    ) -> Result<Self, &'static str> {
        if operations.is_empty() {
            return Err("agent access requires explicit operations");
        }
        if environment_ids.len() > capsem_proto::AGENT_ACCESS_MAX_ENVIRONMENT_IDS
            || sessions.len() > capsem_proto::AGENT_ACCESS_MAX_SESSION_BINDINGS
        {
            return Err("agent access scope exceeds its supported target count");
        }
        if environment_ids.iter().any(|id| !crate::identity::canonical_uuid(id)) {
            return Err("agent environment scope requires canonical UUIDs");
        }
        // Preserve the supplied display order while rejecting duplicate
        // authority. The operation enum has a small closed set; targets are
        // bounded before this work and no hashing is needed.
        for (index, operation) in operations.iter().enumerate() {
            if operations[..index].contains(operation) {
                return Err("agent access operations must be unique");
            }
        }
        for (index, id) in environment_ids.iter().enumerate() {
            if environment_ids[..index].contains(id) {
                return Err("agent environment ids must be unique");
            }
        }
        for (index, session) in sessions.iter().enumerate() {
            if sessions[..index]
                .iter()
                .any(|previous| previous.session_id == session.session_id)
            {
                return Err("agent session ids must be unique");
            }
        }
        if operations.contains(&AgentOperation::Launch) && environment_ids.is_empty() {
            return Err("agent launch requires explicit environment ids");
        }
        Ok(Self {
            operations,
            environment_ids,
            sessions,
        })
    }

    pub fn operations(&self) -> &[AgentOperation] {
        &self.operations
    }
    pub fn environment_ids(&self) -> &[String] {
        &self.environment_ids
    }
    pub fn sessions(&self) -> &[AgentSessionScope] {
        &self.sessions
    }
    pub fn permits(&self, operation: AgentOperation) -> bool {
        self.operations.contains(&operation)
    }
    pub fn permits_launch(&self, environment_id: &str) -> bool {
        self.permits(AgentOperation::Launch) && self.environment_ids.iter().any(|id| id == environment_id)
    }
    pub fn permits_session(&self, session_id: &str, owner_generation: &str) -> bool {
        self.sessions
            .iter()
            .any(|scope| scope.session_id == session_id && scope.owner_generation == owner_generation)
    }
}

impl TryFrom<ScopeWire> for AgentAccessScope {
    type Error = &'static str;
    fn try_from(wire: ScopeWire) -> Result<Self, Self::Error> {
        Self::new(wire.operations, wire.environment_ids, wire.sessions)
    }
}

/// The operator supplies a random managed capability using the same format as
/// managed-session ownership. It is write-only; responses never echo it.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateAgentAccessRequest {
    #[serde(deserialize_with = "crate::managed_sessions::deserialize_request_id")]
    #[schema(format = "uuid")]
    pub request_id: String,
    #[schema(value_type = String, write_only = true, pattern = "^[0-9a-f]{64}$")]
    pub ownership_token: crate::ManagedOwnershipToken,
    pub scope: AgentAccessScope,
    #[serde(deserialize_with = "deserialize_lease_seconds")]
    pub lease_seconds: u32,
}

fn deserialize_lease_seconds<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
    let seconds = u32::deserialize(deserializer)?;
    if seconds == 0 || u64::from(seconds) > capsem_proto::AGENT_ACCESS_MAX_LEASE_SECONDS as u64 {
        return Err(serde::de::Error::custom(
            "agent access lease duration is outside the supported range",
        ));
    }
    Ok(seconds)
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentAccessSummary {
    pub id: String,
    pub scope: AgentAccessScope,
    pub expires_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateAgentAccessResponse {
    pub access: AgentAccessSummary,
}

#[cfg(test)]
mod tests;
