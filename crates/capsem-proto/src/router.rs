//! Bounded stream termination metadata shared by host and guest routing owners.
use serde::{Deserialize, Serialize};

pub const DATA_HEADER_SIZE: usize = 17;

/// The trusted VM owner chooses a fresh generation for each process boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FlowKey {
    pub generation: u64,
    pub id: u64,
}

impl FlowKey {
    pub fn is_valid(self) -> bool {
        self.generation != 0 && self.id != 0
    }

    pub fn data_header(self, connected: bool) -> [u8; DATA_HEADER_SIZE] {
        let mut header = [0; DATA_HEADER_SIZE];
        header[..8].copy_from_slice(&self.id.to_be_bytes());
        header[8..16].copy_from_slice(&self.generation.to_be_bytes());
        header[16] = u8::from(connected);
        header
    }

    pub fn read_data_header(header: [u8; DATA_HEADER_SIZE]) -> Result<(Self, bool), &'static str> {
        let key = Self {
            id: u64::from_be_bytes(header[..8].try_into().unwrap()),
            generation: u64::from_be_bytes(header[8..16].try_into().unwrap()),
        };
        if !key.is_valid() || header[16] > 1 {
            return Err("invalid router data header");
        }
        Ok((key, header[16] == 1))
    }
}

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

#[cfg(test)]
mod tests;
