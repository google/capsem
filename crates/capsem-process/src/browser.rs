//! One latest browser request per live owner. It is deliberately neither
//! checkpointed nor logged: an auth URL must not outlive this session owner.
use capsem_proto::browser::{browser_url, BrowserRequest};
use tokio::sync::watch;

pub(crate) struct SessionBrowser(watch::Sender<Option<BrowserRequest>>);

impl Default for SessionBrowser {
    fn default() -> Self {
        Self(watch::channel(None).0)
    }
}

impl SessionBrowser {
    pub(crate) fn request(&self, url: &str) -> Result<(), &'static str> {
        let url = browser_url(url)?;
        self.0.send_modify(|current| {
            let id = current.as_ref().map_or(1, |request| request.id.saturating_add(1));
            *current = Some(BrowserRequest { id, url });
        });
        Ok(())
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<Option<BrowserRequest>> {
        self.0.subscribe()
    }
}

#[cfg(test)]
mod tests;
