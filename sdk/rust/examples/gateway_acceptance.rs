//! Run against the disposable stopped-workspace Ironbank fixture.

use capsem_sdk::{DiagnosticOptions, Error, Hypervisor, TriageOptions, VmSelector};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::var("SDK_GATEWAY_URL")?;
    let token = std::env::var("SDK_GATEWAY_TOKEN")?;
    let id = std::env::var("SDK_VM_ID")?;
    let denied = Hypervisor::new(&url, "incorrect-token")?;
    assert!(matches!(denied.list().await, Err(Error::Http { status: 401, .. })));
    let hv = Hypervisor::new(&url, &token)?;
    assert!(!hv.info().await?.gateway_version.is_empty());
    let profiles = hv.profiles().list().await?;
    let profile = profiles.first().expect("fixture profile");
    assert_eq!(hv.profiles().mcp(profile).info().await?.profile_id, profile.id);
    hv.debug()
        .panics(DiagnosticOptions {
            limit: Some(2),
            ..Default::default()
        })
        .await?;
    hv.debug()
        .triage(TriageOptions {
            since: Some("1h".into()),
            limit: Some(2),
            ..Default::default()
        })
        .await?;
    assert!(hv.list().await?.sandboxes.iter().any(|vm| vm.id == id));
    let vm = hv.vm(VmSelector::Name("route-workspace".into()))?;
    assert!(vm
        .files()
        .list("", None)
        .await?
        .entries
        .iter()
        .any(|entry| entry.name == "created.txt"));
    assert_eq!(vm.id(), Some(id.as_str()));
    assert!(
        matches!(vm.files().read("/root/created.txt").await, Err(Error::Http { status: 409, body })
        if String::from_utf8_lossy(&body).contains("running sandbox security ledger"))
    );
    assert!(
        matches!(vm.files().write("/root/refused.txt", vec![1]).await, Err(Error::Http { status: 409, body })
        if String::from_utf8_lossy(&body).contains("running sandbox security ledger"))
    );
    assert!(!vm
        .files()
        .list("", None)
        .await?
        .entries
        .iter()
        .any(|entry| entry.name == "refused.txt"));
    assert_eq!(hv.vm(VmSelector::Id(id.clone()))?.id(), Some(id.as_str()));
    println!("BRAAVOS_SDK_ACCEPTANCE_OK");
    Ok(())
}
