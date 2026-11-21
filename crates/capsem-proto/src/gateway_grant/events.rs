//! Fixed-size records on the gateway's private HTTP event feed.

pub const PATH: &str = "/internal/gateway/events";
pub const CONTENT_TYPE: &str = "application/octet-stream";
pub const FRAME_BYTES: usize = 17;

const APPROVAL_PENDING: u8 = 1;
const REQUEST_DENIED: u8 = 2;
const ENVIRONMENT_REQUESTS_CHANGED: u8 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionEvent {
    ApprovalPending([u8; 16]),
    RequestDenied([u8; 16]),
    EnvironmentRequestsChanged([u8; 16]),
}

impl SessionEvent {
    pub fn encode(self) -> [u8; FRAME_BYTES] {
        let (kind, id) = match self {
            Self::ApprovalPending(id) => (APPROVAL_PENDING, id),
            Self::RequestDenied(id) => (REQUEST_DENIED, id),
            Self::EnvironmentRequestsChanged(id) => (ENVIRONMENT_REQUESTS_CHANGED, id),
        };
        let mut frame = [0; FRAME_BYTES];
        frame[0] = kind;
        frame[1..].copy_from_slice(&id);
        frame
    }

    pub fn decode(frame: [u8; FRAME_BYTES]) -> Option<Self> {
        let (kind, id) = frame.split_first()?;
        let id: [u8; 16] = id.try_into().ok()?;
        match *kind {
            APPROVAL_PENDING if id != [0; 16] => Some(Self::ApprovalPending(id)),
            REQUEST_DENIED if id != [0; 16] => Some(Self::RequestDenied(id)),
            ENVIRONMENT_REQUESTS_CHANGED if id != [0; 16] => Some(Self::EnvironmentRequestsChanged(id)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests;
