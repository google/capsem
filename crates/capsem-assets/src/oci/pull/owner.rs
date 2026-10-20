//! Request transports borrow credential-free, long-lived cache ownership.

use super::super::ImageCache;
use super::*;

impl Puller {
    /// Share installed content and readiness with a long-lived owner while
    /// retaining this request's authentication, certificates and architecture.
    pub fn with_cache(mut self, cache: &ImageCache) -> Self {
        self.cache = Some(cache.inner.clone());
        self
    }
}

#[cfg(test)]
mod tests;
