//! A real gateway/native VM proves the managed callback awaits cleanup.

use capsem_sdk::{models::ExecOutputEncoding, CreateOptions, Hypervisor};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let hv = Hypervisor::new(&std::env::var("SDK_GATEWAY_URL")?, &std::env::var("SDK_GATEWAY_TOKEN")?)?;
    let id = hv
        .with_managed(
            CreateOptions {
                cpus: Some(4),
                memory: Some(12),
                ..Default::default()
            },
            |vm, cancelled| async move {
                let info = vm.info().await?;
                assert_eq!(vm.id(), Some(info.id.as_str()));
                assert!(!info.persistent && !cancelled.is_cancelled());
                assert_eq!(info.cpus, Some(4));
                assert_eq!(info.ram_mb, Some(12288));
                let result = vm.exec("printf MANAGED_SDK_NATIVE_READY", Some(60)).await?;
                assert_eq!(result.exit_code, 0);
                assert_eq!(result.stdout.encoding, ExecOutputEncoding::Utf8);
                assert_eq!(result.stdout.data, "MANAGED_SDK_NATIVE_READY");
                Ok(info.id)
            },
        )
        .await?;
    assert!(!hv.list().await?.sandboxes.iter().any(|vm| vm.id == id));
    println!("SDK_MANAGED_ACCEPTANCE_OK");
    Ok(())
}
