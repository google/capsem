// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

// Capsem: preserve the activation wire contract using maintained zbus 6.
use crate::SingleInstanceCallback;
use std::time::Duration;
use tauri::{plugin::TauriPlugin, Manager, RunEvent, Runtime};
use zbus::{connection::Builder, Connection};

type Callback = dyn FnMut(Vec<String>, String) + Send + Sync + 'static;
struct Activation(Box<Callback>);

#[zbus::interface(name = "org.SingleInstance.DBus")]
impl Activation {
    fn execute_callback(&mut self, argv: Vec<String>, cwd: String) {
        (self.0)(argv, cwd);
    }
}

enum Admission {
    Primary { connection: Connection, name: String },
    Forwarded,
}

/// Acquire the name without replacing a running instance. A launch succeeds
/// as a secondary only after the primary acknowledges its activation.
async fn admit(
    name: &str,
    argv: Vec<String>,
    cwd: String,
    callback: impl FnMut(Vec<String>, String) + Send + Sync + 'static,
) -> zbus::Result<Admission> {
    let path = format!("/{}", name.replace('.', "/").replace('-', "_"));
    match Builder::session()
        .name(name)
        .replace_existing_names(false)
        .allow_name_replacements(false)
        .serve_at(path.as_str(), Activation(Box::new(callback)))
        .method_timeout(Duration::from_secs(5))
        .build()
        .await
    {
        Ok(connection) => Ok(Admission::Primary {
            connection,
            name: name.to_owned(),
        }),
        Err(zbus::Error::NameTaken) => {
            let connection = Builder::session()
                .method_timeout(Duration::from_secs(5))
                .build()
                .await?;
            connection
                .call_method(
                    Some(name),
                    path.as_str(),
                    Some("org.SingleInstance.DBus"),
                    "ExecuteCallback",
                    &(argv, cwd),
                )
                .await?;
            Ok(Admission::Forwarded)
        }
        Err(error) => Err(error),
    }
}

struct Primary {
    connection: Connection,
    name: String,
}

pub fn init<R: Runtime>(mut callback: Box<SingleInstanceCallback<R>>, dbus_id: Option<String>) -> TauriPlugin<R> {
    tauri::plugin::Builder::new("single-instance")
        .setup(move |app, _| {
            let handle = app.clone();
            let mut name = dbus_id.unwrap_or_else(|| app.config().identifier.clone());
            name.push_str(".SingleInstance");
            #[cfg(feature = "semver")]
            {
                name.push('_');
                name.push_str(crate::semver_compat::semver_compat_string(&app.package_info().version).as_str());
            }
            let admission = tauri::async_runtime::block_on(admit(
                &name,
                std::env::args().collect(),
                std::env::current_dir()?.to_string_lossy().into_owned(),
                move |args, cwd| {
                    callback(&handle, args, cwd);
                },
            ))?;
            match admission {
                Admission::Primary { connection, name } => {
                    app.manage(Primary { connection, name });
                }
                Admission::Forwarded => {
                    app.cleanup_before_exit();
                    std::process::exit(0);
                }
            }
            Ok(())
        })
        .on_event(|app, event| {
            if let RunEvent::Exit = event {
                destroy(app);
            }
        })
        .build()
}

pub fn destroy<R: Runtime, M: Manager<R>>(manager: &M) {
    if let Some(primary) = manager.try_state::<Primary>() {
        let _ = tauri::async_runtime::block_on(primary.connection.release_name(primary.name.as_str()));
    }
}

#[cfg(test)]
#[path = "linux/tests.rs"]
mod tests;
