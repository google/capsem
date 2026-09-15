use std::str::FromStr;

use crate::{Error, Result};

/// RAM size in binary megabytes or gigabytes. Zero and overflow are rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Memory {
    Megabytes(u64),
    Gigabytes(u64),
}

impl Memory {
    pub fn megabytes(self) -> Result<u64> {
        let value = match self {
            Self::Megabytes(value) => Some(value),
            Self::Gigabytes(value) => value.checked_mul(1024),
        };
        value.filter(|value| *value > 0).ok_or(Error::InvalidInput(
            "memory must be a positive MB count without overflow",
        ))
    }
}

impl FromStr for Memory {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let invalid = || Error::InvalidInput("memory must be a size such as '512M' or '8G'");
        let unit = value.chars().last().ok_or_else(invalid)?;
        let amount = value.strip_suffix(unit).ok_or_else(invalid)?;
        if !amount.bytes().all(|byte| byte.is_ascii_digit()) || amount.starts_with('0') {
            return Err(invalid());
        }
        let amount = amount.parse().map_err(|_| invalid())?;
        let memory = match unit {
            'm' | 'M' => Self::Megabytes(amount),
            'g' | 'G' => Self::Gigabytes(amount),
            _ => return Err(invalid()),
        };
        memory.megabytes()?;
        Ok(memory)
    }
}

#[cfg(test)]
mod tests;
