//! Configuration-owned evidence bounds shared by ingestion and storage.

use anyhow::{bail, Result};
use serde::Deserialize;

#[derive(Debug, Clone, Copy, Deserialize)]
pub(crate) struct Limits {
    pub max_collector_bytes: usize,
    pub max_samples_per_metric: usize,
    pub max_samples_per_run: usize,
}

impl Limits {
    pub fn configured() -> Self {
        static LIMITS: std::sync::OnceLock<Limits> = std::sync::OnceLock::new();
        *LIMITS.get_or_init(|| {
            Self::from_config(include_str!("../../../config/gate.toml")).expect("checked-in benchmark bounds are valid")
        })
    }

    pub fn from_config(text: &str) -> Result<Self> {
        #[derive(Deserialize)]
        struct Config {
            benchmark: Benchmark,
        }
        #[derive(Deserialize)]
        struct Benchmark {
            run: Limits,
        }
        let limits = toml::from_str::<Config>(text)?.benchmark.run;
        if limits.max_collector_bytes == 0
            || limits.max_collector_bytes == usize::MAX
            || limits.max_samples_per_metric == 0
            || limits.max_samples_per_run == 0
            || limits.max_samples_per_run > usize::MAX / 8
        {
            bail!("benchmark evidence bounds must be positive and representable");
        }
        Ok(limits)
    }
}
