use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SecurityEventId(String);

impl SecurityEventId {
    pub fn new_uuid4() -> Self {
        let value = Uuid::new_v4().simple().to_string();
        Self(value[..12].to_string())
    }

    pub fn parse(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        if value.len() == 12 && value.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')) {
            Ok(Self(value))
        } else {
            Err("security event id must be 12 lowercase hex characters".to_string())
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}
