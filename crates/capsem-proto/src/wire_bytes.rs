//! Serde helpers for optional byte buffers on MessagePack transports.

pub(crate) mod option {
    use serde::{Deserialize, Deserializer, Serializer};

    pub(crate) fn serialize<S>(value: &Option<Vec<u8>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            Some(bytes) => serializer.serialize_some(serde_bytes::Bytes::new(bytes)),
            None => serializer.serialize_none(),
        }
    }

    pub(crate) fn deserialize<'de, D>(deserializer: D) -> Result<Option<Vec<u8>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        Option::<serde_bytes::ByteBuf>::deserialize(deserializer).map(|value| value.map(serde_bytes::ByteBuf::into_vec))
    }
}
