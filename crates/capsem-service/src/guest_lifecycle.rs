//! Guest lifecycle intent is admitted by the service, which owns VM locks.

use super::*;

#[cfg(test)]
mod tests;

pub(super) fn spawn_suspend_listener(state: Arc<ServiceState>, id: String, generation: uuid::Uuid) {
    tokio::runtime::Handle::current().spawn(async move {
        let owner = {
            let instances = state.instances.lock().unwrap();
            instances
                .get(&id)
                .filter(|instance| instance.generation == generation)
                .and_then(|instance| owner_connection::OwnerConnection::capture(instance).ok())
        };
        let Some(owner) = owner else {
            return;
        };

        loop {
            let opened = tokio::select! {
                () = owner.revoked() => return,
                opened = owner.open(&state, "capsem-service-lifecycle", false) => opened,
            };
            if let Ok((_tx, rx)) = opened {
                loop {
                    let message = tokio::select! {
                        () = owner.revoked() => return,
                        message = rx.recv() => message,
                    };
                    match message {
                        Ok(ProcessToService::SuspendRequested { id: requested }) if requested == id => {
                            if owner.validate(&state, false).is_err() {
                                return;
                            }
                            if !state
                                .instances
                                .lock()
                                .unwrap()
                                .get(&id)
                                .is_some_and(|instance| instance.persistent)
                            {
                                warn!(id, "guest suspend request refused for ephemeral VM");
                                continue;
                            }
                            if let Err(error) =
                                vm_lifecycle::handle_suspend(State(Arc::clone(&state)), Path(id.clone())).await
                            {
                                warn!(id, error = %error.body.error, "guest suspend request refused");
                            }
                        }
                        Ok(_) => {}
                        Err(_) => break,
                    }
                }
            }
            tokio::select! {
                () = owner.revoked() => return,
                () = tokio::time::sleep(std::time::Duration::from_millis(100)) => {}
            }
        }
    });
}
