//! The shared scripted socket fixture with the CLI unit-test client.

use crate::client::UdsClient;
use std::ops::Deref;

#[path = "../../../tests/helpers/service.rs"]
mod service;

pub(crate) struct FakeService {
    pub client: UdsClient,
    service: service::FakeService,
}

impl FakeService {
    pub fn start() -> Self {
        let service = service::FakeService::start();
        Self {
            client: UdsClient::new(service.socket.clone(), false),
            service,
        }
    }
}

impl Deref for FakeService {
    type Target = service::FakeService;
    fn deref(&self) -> &Self::Target {
        &self.service
    }
}
