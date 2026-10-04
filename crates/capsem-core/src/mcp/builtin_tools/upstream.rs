//! Where a built-in HTTP tool is allowed to connect.
//!
//! The security rules judge `http.host`, but the socket goes to whatever
//! that name resolves to, and the guest can pick a name that resolves to
//! loopback, link-local or a private range -- or one whose answer changes
//! between the check and the dial (DNS rebinding). So the boundary judges
//! the resolved addresses too, refuses a non-public address that no rule
//! explicitly allows, and pins the connection to the exact addresses it
//! judged.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use reqwest::Client;

/// The HTTP settings the built-in tools connect with. Redirects are never
/// followed: the boundary evaluated the requested URL, and a 3xx to another
/// host would reach a name it never judged.
#[derive(Clone, Debug)]
pub struct BuiltinHttpClient {
    request_timeout: Duration,
    connect_timeout: Duration,
    user_agent: Option<String>,
}

impl BuiltinHttpClient {
    pub fn new(request_timeout: Duration, connect_timeout: Duration) -> Self {
        Self {
            request_timeout,
            connect_timeout,
            user_agent: None,
        }
    }

    pub fn with_user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = Some(user_agent.into());
        self
    }

    fn builder(&self) -> reqwest::ClientBuilder {
        let builder = Client::builder()
            .timeout(self.request_timeout)
            .connect_timeout(self.connect_timeout)
            .redirect(reqwest::redirect::Policy::none());
        match &self.user_agent {
            Some(user_agent) => builder.user_agent(user_agent.clone()),
            None => builder,
        }
    }

    /// A client whose connections for `host` go only to `addresses`: the
    /// ones the boundary judged. A name that re-resolves elsewhere between
    /// the check and the dial reaches nothing new.
    pub fn pinned(&self, host: &str, addresses: &[SocketAddr]) -> reqwest::Result<Client> {
        self.builder().resolve_to_addrs(host, addresses).build()
    }
}

/// The reason a non-public address is refused when no rule allowed it.
pub fn non_public_refusal(domain: &str, address: IpAddr) -> String {
    format!(
        "HTTP request blocked: {domain} resolves to {address}, a non-public address; \
         only an explicit allow rule may reach it"
    )
}
