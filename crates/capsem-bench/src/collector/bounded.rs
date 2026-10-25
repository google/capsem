//! Deserialize samples with a shared budget before growing their vectors.

use super::{Collected, RawMetric};
use crate::sample_limits::Limits;
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use std::{collections::BTreeMap, fmt};

pub(super) struct Budget {
    pub limits: Limits,
    pub total: usize,
}
pub(super) struct Document<'a>(pub &'a mut Budget);
struct Metrics<'a>(&'a mut Budget);
struct Metric<'a>(&'a mut Budget);
struct Samples<'a>(&'a mut Budget);

impl<'de> DeserializeSeed<'de> for Document<'_> {
    type Value = Collected;
    fn deserialize<D: de::Deserializer<'de>>(self, deserializer: D) -> Result<Collected, D::Error> {
        deserializer.deserialize_map(self)
    }
}
impl<'de> Visitor<'de> for Document<'_> {
    type Value = Collected;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("one collector document")
    }
    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Collected, M::Error> {
        let (mut metrics, mut sidecar, mut seen_sidecar) = (None, None, false);
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "metrics" if metrics.is_none() => metrics = Some(map.next_value_seed(Metrics(self.0))?),
                "sidecar" if !seen_sidecar => {
                    sidecar = map.next_value()?;
                    seen_sidecar = true;
                }
                _ => return Err(de::Error::custom(format!("unknown or duplicate field {key}"))),
            }
        }
        Ok(Collected {
            metrics: metrics.ok_or_else(|| de::Error::missing_field("metrics"))?,
            sidecar,
        })
    }
}
impl<'de> DeserializeSeed<'de> for Metrics<'_> {
    type Value = BTreeMap<String, RawMetric>;
    fn deserialize<D: de::Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_map(self)
    }
}
impl<'de> Visitor<'de> for Metrics<'_> {
    type Value = BTreeMap<String, RawMetric>;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("bounded metric samples")
    }
    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
        let mut metrics = BTreeMap::new();
        while let Some(key) = map.next_key::<String>()? {
            if metrics.contains_key(&key) {
                return Err(de::Error::custom("duplicate metric key"));
            }
            metrics.insert(key, map.next_value_seed(Metric(self.0))?);
        }
        Ok(metrics)
    }
}
impl<'de> DeserializeSeed<'de> for Metric<'_> {
    type Value = RawMetric;
    fn deserialize<D: de::Deserializer<'de>>(self, deserializer: D) -> Result<RawMetric, D::Error> {
        deserializer.deserialize_map(self)
    }
}
impl<'de> Visitor<'de> for Metric<'_> {
    type Value = RawMetric;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a unit and bounded samples")
    }
    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<RawMetric, M::Error> {
        let (mut unit, mut samples) = (None, None);
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "unit" if unit.is_none() => unit = Some(map.next_value()?),
                "samples" if samples.is_none() => samples = Some(map.next_value_seed(Samples(self.0))?),
                _ => return Err(de::Error::custom(format!("unknown or duplicate metric field {key}"))),
            }
        }
        Ok(RawMetric {
            unit: unit.ok_or_else(|| de::Error::missing_field("unit"))?,
            samples: samples.ok_or_else(|| de::Error::missing_field("samples"))?,
        })
    }
}
impl<'de> DeserializeSeed<'de> for Samples<'_> {
    type Value = Vec<f64>;
    fn deserialize<D: de::Deserializer<'de>>(self, deserializer: D) -> Result<Vec<f64>, D::Error> {
        deserializer.deserialize_seq(self)
    }
}
impl<'de> Visitor<'de> for Samples<'_> {
    type Value = Vec<f64>;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a bounded finite sample array")
    }
    fn visit_seq<S: SeqAccess<'de>>(self, mut seq: S) -> Result<Vec<f64>, S::Error> {
        let mut values = Vec::new();
        while let Some(value) = seq.next_element::<f64>()? {
            if !value.is_finite() {
                return Err(de::Error::custom("non-finite sample"));
            }
            if values.len() >= self.0.limits.max_samples_per_metric || self.0.total >= self.0.limits.max_samples_per_run
            {
                return Err(de::Error::custom("collector sample limit exceeded"));
            }
            self.0.total += 1;
            values.push(value);
        }
        Ok(values)
    }
}
