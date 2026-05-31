use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub(crate) struct RuntimeRuleMatchAccumulator {
    inner: Arc<Mutex<BTreeMap<String, RuntimeRuleMatchStats>>>,
}

#[derive(Clone, Default)]
struct RuntimeRuleMatchStats {
    match_count: u64,
    last_matched_event: Option<String>,
    last_matched_unix_ms: Option<u64>,
}

impl RuntimeRuleMatchAccumulator {
    pub(crate) fn drain(&self) -> Vec<capsem_proto::ipc::RuntimeRuleMatchSnapshot> {
        let mut matches = self.inner.lock().unwrap();
        let drained = std::mem::take(&mut *matches);
        drained
            .into_iter()
            .map(
                |(rule_id, stats)| capsem_proto::ipc::RuntimeRuleMatchSnapshot {
                    rule_id,
                    match_count: stats.match_count,
                    last_matched_event: stats.last_matched_event,
                    last_matched_unix_ms: stats.last_matched_unix_ms,
                },
            )
            .collect()
    }
}

impl capsem_security_engine::RuleMatchRecorder for RuntimeRuleMatchAccumulator {
    fn record_rule_match(
        &mut self,
        rule_id: &str,
        event_id: &str,
        timestamp_unix_ms: u64,
    ) -> Result<(), capsem_security_engine::SecurityEngineError> {
        let mut matches = self.inner.lock().map_err(|error| {
            capsem_security_engine::SecurityEngineError::PhaseFailed {
                phase: capsem_security_engine::SecurityEnginePhase::Detection,
                message: format!("runtime rule match accumulator lock poisoned: {error}"),
            }
        })?;
        let stats = matches.entry(rule_id.to_owned()).or_default();
        stats.match_count += 1;
        stats.last_matched_event = Some(event_id.to_owned());
        stats.last_matched_unix_ms = Some(timestamp_unix_ms);
        Ok(())
    }
}
