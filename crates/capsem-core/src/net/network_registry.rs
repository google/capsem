//! Named networks as groups of VMs, each with its own logger-owned database.
//!
//! A network is an immutable id and a name that is unique among active
//! networks; retiring one frees the name, and a new network under the same
//! name is a different network with a different database. Membership is the
//! VM and the lifetime address it already has -- there is nothing to lease
//! per network, so several networks can hold the same VM.
//!
//! The registry is the in-memory authority while the service runs. Every
//! change is written to the network's database and flushed before the call
//! returns, and `load` rebuilds the registry from those databases at start;
//! a database that cannot be read is a startup failure, not an empty network.

use capsem_foundation::paths::network_db_path_in;
use capsem_logger::{network_db, DbHandle, MembershipState, NetworkMembership, NetworkRecord, WriteOp};
use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

mod logs;
pub use logs::{LogEvent, LogPage, LogQuery, DEFAULT_LOG_LIMIT, MAX_LOG_LIMIT};

/// How long a retired network's database, and with it its audit history,
/// stays on disk: long enough to answer "what talked to what last month",
/// bounded so a host that creates and retires networks daily is not keeping
/// every one of them forever.
pub const NETWORK_AUDIT_RETENTION: std::time::Duration = std::time::Duration::from_secs(30 * 24 * 60 * 60);

/// What a deleted VM left behind in one network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Departure {
    pub network: Uuid,
    /// The VM was the last member, so the network retired with it.
    pub retired: bool,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum NetworkError {
    #[error("network name {0:?} must be a DNS label: 1-63 lowercase letters, digits or hyphens, not at the ends")]
    InvalidName(String),
    #[error("network name {name:?} is already used by network {id}")]
    NameTaken { name: String, id: Uuid },
    #[error("network {0} does not exist")]
    NotFound(Uuid),
    #[error("network {id} still has {members} member(s); disconnect them first")]
    HasMembers { id: Uuid, members: usize },
    #[error("VM {vm_id} is not a member of network {id}")]
    NotAMember { id: Uuid, vm_id: String },
    #[error("network database {path}: {error}")]
    Database { path: PathBuf, error: String },
    #[error("invalid log cursor or limit: {0}")]
    Cursor(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkSummary {
    pub id: Uuid,
    pub name: String,
    pub created_unix_ms: i64,
    pub member_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub vm_id: String,
    pub address: Ipv4Addr,
    pub state: MembershipState,
    pub updated_unix_ms: i64,
}

struct NetworkEntry {
    name: String,
    created_unix_ms: i64,
    members: BTreeMap<String, Member>,
    handle: Arc<DbHandle>,
}

pub struct NetworkRegistry {
    root: PathBuf,
    networks: BTreeMap<Uuid, NetworkEntry>,
    /// Readers of retired networks, opened on first history read.
    retired_readers: BTreeMap<Uuid, Arc<DbHandle>>,
    /// Retired databases still on disk, by the moment they retired: what the
    /// retention sweep works from.
    retired: BTreeMap<Uuid, i64>,
}

impl std::fmt::Debug for NetworkRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NetworkRegistry")
            .field("root", &self.root)
            .field("networks", &self.list())
            .finish()
    }
}

impl NetworkRegistry {
    /// An empty registry over `root`; nothing on disk is read.
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            networks: BTreeMap::new(),
            retired_readers: BTreeMap::new(),
            retired: BTreeMap::new(),
        }
    }

    /// Rebuild the registry from every network database under `root`.
    /// Retired networks are left on disk for their audit history and are
    /// not loaded.
    pub async fn load(root: PathBuf) -> Result<Self, NetworkError> {
        let mut registry = Self::new(root.clone());
        let Ok(entries) = std::fs::read_dir(&root) else {
            return Ok(registry);
        };
        let mut ids: Vec<Uuid> = entries
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| entry.file_name().to_str().and_then(|name| Uuid::parse_str(name).ok()))
            .filter(|id| network_db_path_in(&root, &id.to_string()).exists())
            .collect();
        ids.sort();
        for id in ids {
            let path = network_db_path_in(&root, &id.to_string());
            let handle = Arc::new(open(&path)?);
            let network = handle
                .query(
                    "SELECT name, state, created_unix_ms, retired_unix_ms FROM network WHERE id = ?1",
                    &[id.to_string().into()],
                )
                .await
                .map_err(|error| database(&path, error))?;
            let Some(row) = rows(&path, &network)?.into_iter().next() else {
                return Err(database(&path, "network table has no row for this database"));
            };
            if row[1].as_str() != Some("active") {
                registry.retired.insert(id, integer(&path, &row[3])?);
                continue;
            }
            let name = text(&path, &row[0])?;
            let created_unix_ms = integer(&path, &row[2])?;
            let members_json = handle
                .query(
                    "SELECT vm_id, address, state, updated_unix_ms FROM network_members WHERE network_id = ?1 AND state != 'detached' ORDER BY vm_id",
                    &[id.to_string().into()],
                )
                .await
                .map_err(|error| database(&path, error))?;
            let mut members = BTreeMap::new();
            for row in rows(&path, &members_json)? {
                let vm_id = text(&path, &row[0])?;
                let address = text(&path, &row[1])?
                    .parse()
                    .map_err(|_| database(&path, "member address is not IPv4"))?;
                let state = parse_state(&path, &row[2])?;
                let updated_unix_ms = integer(&path, &row[3])?;
                members.insert(
                    vm_id.clone(),
                    Member {
                        vm_id,
                        address,
                        state,
                        updated_unix_ms,
                    },
                );
            }
            registry.networks.insert(
                id,
                NetworkEntry {
                    name,
                    created_unix_ms,
                    members,
                    handle,
                },
            );
        }
        Ok(registry)
    }

    pub async fn create(&mut self, name: &str, now_unix_ms: i64) -> Result<NetworkSummary, NetworkError> {
        validate_name(name)?;
        if let Some(id) = self.find(name) {
            return Err(NetworkError::NameTaken {
                name: name.to_string(),
                id,
            });
        }
        let id = Uuid::new_v4();
        let path = network_db_path_in(&self.root, &id.to_string());
        let handle = Arc::new(open(&path)?);
        let record = NetworkRecord::new(id, name, now_unix_ms).map_err(|error| database(&path, error))?;
        write_durably(&handle, &path, WriteOp::Network(record)).await?;
        self.networks.insert(
            id,
            NetworkEntry {
                name: name.to_string(),
                created_unix_ms: now_unix_ms,
                members: BTreeMap::new(),
                handle,
            },
        );
        Ok(self.summary(id).expect("just inserted"))
    }

    pub fn list(&self) -> Vec<NetworkSummary> {
        self.networks.keys().filter_map(|id| self.summary(*id)).collect()
    }

    pub fn summary(&self, id: Uuid) -> Option<NetworkSummary> {
        self.networks.get(&id).map(|entry| NetworkSummary {
            id,
            name: entry.name.clone(),
            created_unix_ms: entry.created_unix_ms,
            member_count: entry.members.len(),
        })
    }

    /// The active network with this name, if any.
    pub fn find(&self, name: &str) -> Option<Uuid> {
        self.networks
            .iter()
            .find(|(_, entry)| entry.name == name)
            .map(|(id, _)| *id)
    }

    pub fn members(&self, id: Uuid) -> Option<Vec<Member>> {
        self.networks
            .get(&id)
            .map(|entry| entry.members.values().cloned().collect())
    }

    /// Networks a VM belongs to.
    pub fn memberships_of(&self, vm_id: &str) -> Vec<Uuid> {
        self.networks
            .iter()
            .filter(|(_, entry)| entry.members.contains_key(vm_id))
            .map(|(id, _)| *id)
            .collect()
    }

    /// Retire an empty network. Its database stays for the retention window;
    /// its name is free again from this call on.
    pub async fn retire(&mut self, id: Uuid, now_unix_ms: i64) -> Result<(), NetworkError> {
        let entry = self.networks.get(&id).ok_or(NetworkError::NotFound(id))?;
        if !entry.members.is_empty() {
            return Err(NetworkError::HasMembers {
                id,
                members: entry.members.len(),
            });
        }
        let path = network_db_path_in(&self.root, &id.to_string());
        let record = NetworkRecord::new(id, &entry.name, entry.created_unix_ms)
            .and_then(|record| record.retired(now_unix_ms))
            .map_err(|error| database(&path, error))?;
        write_durably(&entry.handle, &path, WriteOp::Network(record)).await?;
        self.networks.remove(&id);
        self.retired.insert(id, now_unix_ms);
        Ok(())
    }

    /// Remove retired databases older than `retention`, releasing their
    /// readers first; the ids removed. One that cannot be removed stays
    /// listed and is tried again on the next sweep.
    pub fn sweep_retired(&mut self, now_unix_ms: i64, retention: std::time::Duration) -> Vec<Uuid> {
        let cutoff = now_unix_ms.saturating_sub(i64::try_from(retention.as_millis()).unwrap_or(i64::MAX));
        let expired: Vec<Uuid> = self
            .retired
            .iter()
            .filter(|(_, retired_unix_ms)| **retired_unix_ms <= cutoff)
            .map(|(id, _)| *id)
            .collect();
        let mut removed = Vec::new();
        for id in expired {
            self.retired_readers.remove(&id);
            let dir = self.root.join(id.to_string());
            match std::fs::remove_dir_all(&dir) {
                Ok(()) => {
                    self.retired.remove(&id);
                    removed.push(id);
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    self.retired.remove(&id);
                }
                Err(error) => {
                    tracing::warn!(network = %id, path = %dir.display(), %error, "retired network database not removed");
                }
            }
        }
        removed
    }

    /// Record a VM in a network with its lifetime address; a second call for
    /// the same VM updates its state.
    pub async fn attach(
        &mut self,
        id: Uuid,
        vm_id: &str,
        address: Ipv4Addr,
        state: MembershipState,
        now_unix_ms: i64,
    ) -> Result<(), NetworkError> {
        let entry = self.networks.get_mut(&id).ok_or(NetworkError::NotFound(id))?;
        let path = network_db_path_in(&self.root, &id.to_string());
        let membership =
            NetworkMembership::new(id, vm_id, address, state, now_unix_ms).map_err(|error| database(&path, error))?;
        write_durably(&entry.handle, &path, WriteOp::NetworkMembership(membership)).await?;
        entry.members.insert(
            vm_id.to_string(),
            Member {
                vm_id: vm_id.to_string(),
                address,
                state,
                updated_unix_ms: now_unix_ms,
            },
        );
        Ok(())
    }

    pub async fn detach(&mut self, id: Uuid, vm_id: &str, now_unix_ms: i64) -> Result<(), NetworkError> {
        let entry = self.networks.get_mut(&id).ok_or(NetworkError::NotFound(id))?;
        let member = entry.members.get(vm_id).ok_or_else(|| NetworkError::NotAMember {
            id,
            vm_id: vm_id.to_string(),
        })?;
        let path = network_db_path_in(&self.root, &id.to_string());
        let membership = NetworkMembership::new(id, vm_id, member.address, MembershipState::Detached, now_unix_ms)
            .map_err(|error| database(&path, error))?;
        write_durably(&entry.handle, &path, WriteOp::NetworkMembership(membership)).await?;
        entry.members.remove(vm_id);
        Ok(())
    }

    /// A deleted VM leaves every network it was in, and a network it leaves
    /// empty retires with it: its members were its purpose, and its history
    /// stays for the retention window. A disconnect is different -- the VM
    /// is still there and may come back -- so it never retires anything.
    pub async fn vm_deleted(&mut self, vm_id: &str, now_unix_ms: i64) -> Result<Vec<Departure>, NetworkError> {
        let mut departures = Vec::new();
        for network in self.memberships_of(vm_id) {
            self.detach(network, vm_id, now_unix_ms).await?;
            let retired = self.members(network).is_some_and(|members| members.is_empty());
            if retired {
                self.retire(network, now_unix_ms).await?;
            }
            departures.push(Departure { network, retired });
        }
        Ok(departures)
    }
}

/// A DNS label, because a network name is what members resolve each other
/// under once private DNS lands.
fn validate_name(name: &str) -> Result<(), NetworkError> {
    let valid = (1..=63).contains(&name.len())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !name.starts_with('-')
        && !name.ends_with('-');
    if valid {
        Ok(())
    } else {
        Err(NetworkError::InvalidName(name.to_string()))
    }
}

fn open(path: &Path) -> Result<DbHandle, NetworkError> {
    network_db::open(path).map_err(|error| database(path, error.to_string()))
}

async fn write_durably(handle: &DbHandle, path: &Path, op: WriteOp) -> Result<(), NetworkError> {
    handle.write(op).await.map_err(|error| database(path, error))?;
    handle.flush().await.map_err(|error| database(path, error))
}

fn database(path: &Path, error: impl ToString) -> NetworkError {
    NetworkError::Database {
        path: path.to_path_buf(),
        error: error.to_string(),
    }
}

fn rows(path: &Path, json: &str) -> Result<Vec<Vec<serde_json::Value>>, NetworkError> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|error| database(path, error))?;
    serde_json::from_value(value["rows"].clone()).map_err(|error| database(path, error))
}

fn text(path: &Path, value: &serde_json::Value) -> Result<String, NetworkError> {
    value
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| database(path, format!("expected text, found {value}")))
}

fn integer(path: &Path, value: &serde_json::Value) -> Result<i64, NetworkError> {
    value
        .as_i64()
        .ok_or_else(|| database(path, format!("expected an integer, found {value}")))
}

fn parse_state(path: &Path, value: &serde_json::Value) -> Result<MembershipState, NetworkError> {
    let state = text(path, value)?;
    [
        MembershipState::Declared,
        MembershipState::Attaching,
        MembershipState::Ready,
        MembershipState::Failed,
        MembershipState::Detached,
    ]
    .into_iter()
    .find(|candidate| candidate.as_str() == state)
    .ok_or_else(|| database(path, format!("unknown membership state {state:?}")))
}

#[cfg(test)]
mod tests;
