use std::time::Duration;

use crate::transport::{CallOptions, Transport};
use crate::{Error, Result};

#[derive(Debug, Clone)]
pub(crate) struct Client {
    pub transport: Transport,
    pub options: CallOptions,
}

impl Client {
    pub fn new(url: &str, token: &str) -> Result<Self> {
        Ok(Self {
            transport: Transport::new(url, token, Duration::from_secs(30))?,
            options: CallOptions::default(),
        })
    }

    pub fn set_timeout(&mut self, timeout: Duration) -> Result<()> {
        if timeout.is_zero() {
            return Err(Error::InvalidInput("request timeout must be positive"));
        }
        self.options.timeout = Some(timeout);
        Ok(())
    }
}
