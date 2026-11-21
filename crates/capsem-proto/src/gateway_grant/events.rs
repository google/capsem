//! Fixed-size records on the gateway's private HTTP event feed.

pub const PATH: &str = "/internal/gateway/events";
pub const CONTENT_TYPE: &str = "application/octet-stream";
pub const FRAME_BYTES: usize = 17;

const APPROVAL_PENDING: u8 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionEvent {
    ApprovalPending([u8; 16]),
}

impl SessionEvent {
    pub fn encode(self) -> [u8; FRAME_BYTES] {
        let (kind, id) = match self {
            Self::ApprovalPending(id) => (APPROVAL_PENDING, id),
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
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests;
