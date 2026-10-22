//! Thin native client adapter. The SDK owns callback transport and lifecycle.
use capsem_sdk::browser::BrowserCallback;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Context {
    gateway_url: String,
    token: String,
    vm_id: String,
}

/// One pending browser sign-in per client window. A newer guest request
/// explicitly replaces it; ordinary documentation links leave it alone.
#[derive(Default)]
pub(crate) struct Callbacks(tokio::sync::Mutex<Option<BrowserCallback>>);

pub(crate) async fn open(
    callbacks: &Callbacks,
    url: &str,
    context: Option<Context>,
    opener: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let Some(context) = context else { return opener() };
    let vm = capsem_sdk::VM::new(
        &context.gateway_url,
        &context.token,
        capsem_sdk::VmSelector::Id(context.vm_id),
    )
    .map_err(|_| "invalid browser session".to_owned())?;
    let mut current = callbacks.0.lock().await;
    if let Some(previous) = current.take() {
        previous
            .close()
            .await
            .map_err(|_| "could not close previous browser callback".to_owned())?;
    }
    let prepared = BrowserCallback::prepare(&vm, url)
        .await
        .map_err(|_| "could not prepare browser callback".to_owned())?;
    if let Err(error) = opener() {
        if let Some(callback) = prepared {
            callback
                .close()
                .await
                .map_err(|_| "could not close browser callback".to_owned())?;
        }
        return Err(error);
    }
    *current = prepared;
    drop(current);
    Ok(())
}

#[cfg(test)]
mod tests;
