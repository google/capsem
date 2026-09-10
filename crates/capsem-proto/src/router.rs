//! Bounded stream termination metadata shared by host and guest routing owners.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum CloseReason {
    Complete = 0,
    WriteStall = 1,
    HalfCloseTimeout = 2,
    Reset = 3,
    Io = 4,
    Cancelled = 5,
}

impl TryFrom<u8> for CloseReason {
    type Error = &'static str;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Complete),
            1 => Ok(Self::WriteStall),
            2 => Ok(Self::HalfCloseTimeout),
            3 => Ok(Self::Reset),
            4 => Ok(Self::Io),
            5 => Ok(Self::Cancelled),
            _ => Err("invalid router close reason"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloseReport {
    pub reason: CloseReason,
    pub from_source: u64,
    pub to_source: u64,
}
