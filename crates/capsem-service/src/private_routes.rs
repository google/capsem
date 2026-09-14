//! Private names: a VM owner asks what a member of its networks is called or
//! where it is, and gets an answer only about members it shares a network
//! with. Traffic never comes here: every protocol rides each member's cable
//! to the network's switch (`switches`).
//!
//! The owner proves itself with the secret minted for its VM at spawn.
use super::*;

const OWNER_SECRET_FILE: &str = "owner-secret";

/// Mint the secret an owner will show, and leave it in its session directory
/// for that owner alone.
pub(super) fn mint_owner_secret(session_dir: &std::path::Path) -> Result<String> {
    let secret = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
    let path = session_dir.join(OWNER_SECRET_FILE);
    let mut file = std::fs::OpenOptions::new();
    file.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        file.mode(0o600);
    }
    std::io::Write::write_all(
        &mut file.open(&path).with_context(|| format!("create {}", path.display()))?,
        secret.as_bytes(),
    )
    .with_context(|| format!("write {}", path.display()))?;
    Ok(secret)
}

fn secrets_match(presented: &str, expected: &str) -> bool {
    // Same length, then every byte compared: a mismatch costs the same as a
    // match, so timing says nothing about how much of the secret was right.
    presented.len() == expected.len()
        && presented
            .bytes()
            .zip(expected.bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

/// The asker is the running owner of `vm`, or nobody.
fn owner_of(state: &ServiceState, vm: &str, secret: &str) -> Result<(), AppError> {
    let expected = state
        .instances
        .lock()
        .unwrap()
        .get(vm)
        .map(|instance| instance.owner_secret.clone());
    match expected {
        Some(minted) if secrets_match(secret, &minted) => Ok(()),
        _ => Err(AppError(
            StatusCode::FORBIDDEN,
            format!("VM {vm} has no running owner presenting this secret"),
        )),
    }
}

/// What a VM is called on the private zone: its display name.
fn vm_display_name(state: &ServiceState, vm_id: &str) -> String {
    if let Some(instance) = state.instances.lock().unwrap().get(vm_id) {
        return instance.name.clone();
    }
    vm_lifecycle::find_persistent_entry_by_route_id(state, vm_id)
        .map(|entry| entry.name)
        .unwrap_or_else(|| vm_id.to_string())
}

/// One member as the asker may name it.
struct Visible {
    vm_id: String,
    vm_name: String,
    network_name: String,
    address: std::net::Ipv4Addr,
}

impl Visible {
    fn full_name(&self) -> String {
        format!(
            "{}.{}.{}",
            self.vm_name,
            self.network_name,
            capsem_core::net::dns::private::PRIVATE_ZONE
        )
    }
}

/// Every member of every network the asker is in, itself included, in a
/// deterministic order: by network name, then VM name.
async fn visible_to(state: &ServiceState, source_vm: &str) -> Vec<Visible> {
    let registry = state.networks.lock().await;
    let mut visible = Vec::new();
    for network in registry.memberships_of(source_vm) {
        let Some(summary) = registry.summary(network) else {
            continue;
        };
        for member in registry.members(network).unwrap_or_default() {
            visible.push(Visible {
                vm_name: vm_display_name(state, &member.vm_id),
                vm_id: member.vm_id,
                network_name: summary.name.clone(),
                address: member.address,
            });
        }
    }
    drop(registry);
    visible.sort_by(|a, b| (&a.network_name, &a.vm_name).cmp(&(&b.network_name, &b.vm_name)));
    visible
}

/// A private name or a pool address, answered with a member the asker may
/// see: `<vm>.<network>` names one member, `<vm>` alone only when one
/// member across the asker's networks has that name.
pub(super) async fn handle_private_resolve(
    State(state): State<Arc<ServiceState>>,
    Json(request): Json<PrivateResolveRequest>,
) -> Result<Json<PrivateResolveResponse>, AppError> {
    owner_of(&state, &request.source_vm, &request.owner_secret)?;
    let visible = visible_to(&state, &request.source_vm).await;
    let found = match (&request.name, request.address) {
        (Some(name), None) => {
            let name = name.trim_matches('.').to_ascii_lowercase();
            let (vm_name, network_name) = match name.split_once('.') {
                Some((vm, network)) => (vm, Some(network)),
                None => (name.as_str(), None),
            };
            let mut matches = visible.iter().filter(|member| {
                member.vm_name.eq_ignore_ascii_case(vm_name)
                    && network_name.is_none_or(|network| member.network_name.eq_ignore_ascii_case(network))
            });
            match (matches.next(), matches.next()) {
                (Some(member), None) => Some(member),
                // Two networks answer a short name differently: ambiguous is
                // nobody's, the full name still works.
                _ => None,
            }
        }
        (None, Some(address)) => visible.iter().find(|member| member.address == address),
        _ => {
            return Err(AppError(
                StatusCode::BAD_REQUEST,
                "resolve names exactly one of a name or an address".into(),
            ))
        }
    };
    let member = found.ok_or_else(|| AppError(StatusCode::NOT_FOUND, "no such member".into()))?;
    Ok(Json(PrivateResolveResponse {
        name: member.full_name(),
        address: member.address,
        vm: member.vm_id.clone(),
        network: member.network_name.clone(),
    }))
}
