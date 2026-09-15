//! This VM's view of the private zone: every question is the service's to
//! answer, with the members of the networks this VM is in and nothing else.
//! No cache here: the answer carries no TTL and a membership that changed
//! is gone from the next answer.
use capsem_core::net::dns::private::{Lookup, PrivateNames};
use std::net::Ipv4Addr;
use std::path::PathBuf;

pub(crate) struct ServicePrivateNames {
    service_socket: PathBuf,
    owner_secret: String,
    vm_id: String,
}

impl ServicePrivateNames {
    pub(crate) fn new(service_socket: PathBuf, owner_secret: String, vm_id: String) -> Self {
        Self {
            service_socket,
            owner_secret,
            vm_id,
        }
    }

    async fn ask(&self, question: serde_json::Value) -> Option<serde_json::Value> {
        let mut request = question;
        request["source_vm"] = serde_json::Value::String(self.vm_id.clone());
        request["owner_secret"] = serde_json::Value::String(self.owner_secret.clone());
        match capsem_core::service_uds::post_json(&self.service_socket, "/networks/private/resolve", &request).await {
            Ok((200, answer)) => Some(answer),
            Ok((404, _)) => None,
            Ok((status, answer)) => {
                tracing::warn!(status, %answer, "private name lookup refused");
                None
            }
            Err(error) => {
                tracing::warn!(%error, "private name lookup failed");
                None
            }
        }
    }
}

impl PrivateNames for ServicePrivateNames {
    fn address_of<'a>(&'a self, name: &'a str) -> Lookup<'a, Ipv4Addr> {
        Box::pin(async move {
            let answer = self.ask(serde_json::json!({ "name": name })).await?;
            answer["address"].as_str()?.parse().ok()
        })
    }

    fn name_of(&self, address: Ipv4Addr) -> Lookup<'_, String> {
        Box::pin(async move {
            let answer = self.ask(serde_json::json!({ "address": address.to_string() })).await?;
            answer["name"].as_str().map(str::to_string)
        })
    }
}

#[cfg(test)]
mod tests;
