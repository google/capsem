//! Metric export for the service: its own facade metrics, and every
//! session's totals observed from the ledgers' counter snapshots.
//!
//! Off unless the corp config's `open_telemetry` or `OTEL_EXPORTER_OTLP_*`
//! asks for it (see `capsem_telemetry::export`). The session instruments are
//! observable: OpenTelemetry calls them on each export, synchronously, so a
//! background task keeps a table of the latest snapshots and the callbacks
//! read that. Attributes are exactly `session.id`, `profile.id` and
//! `persistent`.

use std::sync::{Arc, RwLock};
use std::time::Duration;

use capsem_logger::counters::{usd_from_micro, LedgerCounters};
use capsem_telemetry::export::{Destination, Exporter, KeyValue, Meter};
use capsem_telemetry::session as names;

use super::*;

/// How often the session table is refreshed. Well inside the SDK's default
/// 60 s export interval, so each export sees totals at most this old.
const REFRESH_INTERVAL: Duration = Duration::from_secs(15);

/// Install export for the service when the corp config or environment asks
/// for it. `None` means export is off; a failure to install is logged and
/// leaves it off -- metrics are never a reason the service does not start.
pub(crate) fn install() -> Option<Exporter> {
    let (_, corp) = capsem_core::net::policy_config::load_settings_and_corp_files();
    let destination = Destination::resolve(corp.corp_rule_files.open_telemetry.as_deref(), |name| {
        std::env::var(name).ok()
    })?;
    match capsem_telemetry::export::install(&destination, "capsem-service", Vec::new()) {
        Ok(exporter) => {
            info!(?destination, "metric export installed");
            Some(exporter)
        }
        Err(error) => {
            warn!(%error, ?destination, "metric export not installed");
            None
        }
    }
}

/// One session's totals and the attributes its series carry.
#[derive(Clone, Debug)]
pub(crate) struct SessionTotals {
    pub(crate) attributes: Vec<KeyValue>,
    pub(crate) counters: LedgerCounters,
}

/// The table the observable callbacks read.
pub(crate) type SessionTable = Arc<RwLock<Vec<SessionTotals>>>;

/// Register the session instruments on `meter`, reading `table`.
///
/// The instruments are returned so the caller keeps them; OpenTelemetry
/// observes them for as long as they, and the provider, live.
pub(crate) fn register(meter: &Meter, table: &SessionTable) -> Vec<Box<dyn std::any::Any + Send + Sync>> {
    fn describe(name: &'static str) -> &'static str {
        capsem_telemetry::all()
            .find(|spec| spec.name == name)
            .map(|spec| spec.description)
            .unwrap_or_default()
    }
    type Observe = fn(&LedgerCounters, &mut dyn FnMut(u64, &[KeyValue]));
    let counts: [(&'static str, Observe); 6] = [
        (names::SESSION_REQUESTS_TOTAL, |c, emit| {
            emit(c.net.allowed, &[KeyValue::new("decision", "allowed")]);
            emit(c.net.denied, &[KeyValue::new("decision", "denied")]);
            emit(c.net.error, &[KeyValue::new("decision", "error")]);
        }),
        (names::SESSION_TOKENS_TOTAL, |c, emit| {
            emit(c.model.total.input_tokens, &[KeyValue::new("direction", "input")]);
            emit(c.model.total.output_tokens, &[KeyValue::new("direction", "output")]);
        }),
        (names::SESSION_MODEL_CALLS_TOTAL, |c, emit| {
            emit(c.model.total.calls, &[])
        }),
        (names::SESSION_TOOL_CALLS_TOTAL, |c, emit| emit(c.tools.calls, &[])),
        (names::SESSION_FILE_EVENTS_TOTAL, |c, emit| {
            let overflow = c
                .files
                .by_action
                .get(capsem_logger::FileAction::Overflow.as_str())
                .copied()
                .unwrap_or_default();
            emit(c.files.events.saturating_sub(overflow), &[]);
        }),
        (names::SESSION_RULE_MATCHES_TOTAL, |c, emit| {
            emit(c.security.matches, &[])
        }),
    ];
    let mut instruments: Vec<Box<dyn std::any::Any + Send + Sync>> = Vec::new();
    for (name, observe) in counts {
        let table = Arc::clone(table);
        instruments.push(Box::new(
            meter
                .u64_observable_counter(name)
                .with_description(describe(name))
                .with_unit("{count}")
                .with_callback(move |observer| {
                    for session in table.read().unwrap_or_else(|poisoned| poisoned.into_inner()).iter() {
                        observe(&session.counters, &mut |value, extra| {
                            let mut attributes = session.attributes.clone();
                            attributes.extend_from_slice(extra);
                            observer.observe(value, &attributes);
                        });
                    }
                })
                .build(),
        ));
    }
    let cost_table = Arc::clone(table);
    instruments.push(Box::new(
        meter
            .f64_observable_counter(names::SESSION_COST_USD_TOTAL)
            .with_description(describe(names::SESSION_COST_USD_TOTAL))
            .with_unit("USD")
            .with_callback(move |observer| {
                for session in cost_table
                    .read()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .iter()
                {
                    observer.observe(
                        usd_from_micro(session.counters.model.total.cost_micro_usd),
                        &session.attributes,
                    );
                }
            })
            .build(),
    ));
    instruments
}

/// Every listed VM's totals, from its ledger's counter snapshot. A VM whose
/// ledger is not ready has no series, not zeros.
pub(crate) async fn collect(state: &Arc<ServiceState>) -> Result<Vec<SessionTotals>, AppError> {
    let (listed, session_dirs) = state
        .off_worker(|state| sandbox_info::build_list_response(&state))
        .await?;
    let mut totals = Vec::with_capacity(listed.sandboxes.len());
    for (info, session_dir) in listed.sandboxes.iter().zip(&session_dirs) {
        let Some(counters) = ledger_routes::activity::counters_if_ready(state, &info.id, session_dir).await else {
            continue;
        };
        totals.push(SessionTotals {
            attributes: vec![
                KeyValue::new("session.id", info.id.clone()),
                KeyValue::new("profile.id", info.profile_id.clone()),
                KeyValue::new("persistent", info.persistent),
            ],
            counters,
        });
    }
    Ok(totals)
}

/// Keep the table current for as long as the service runs.
pub(crate) fn spawn_refresh(state: Arc<ServiceState>, table: SessionTable) {
    tokio::spawn(async move {
        loop {
            match collect(&state).await {
                Ok(totals) => *table.write().unwrap_or_else(|poisoned| poisoned.into_inner()) = totals,
                Err(error) => warn!(error = %error.1, "session metric refresh failed"),
            }
            tokio::time::sleep(REFRESH_INTERVAL).await;
        }
    });
}
