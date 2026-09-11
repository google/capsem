//! Network boundaries use the same rule/plugin stages and the same DB producer.
use super::*;
use crate::net::policy_config::{SecurityRuleSet, SharedPluginPolicy};
use crate::security_engine::{
    apply_event_decision_to_enforcement, emit_matching_security_rules_with_decision, emit_security_write,
    evaluate_security_boundary, prepare_evaluated_event_for_security_rule_ledger, SecurityEnforcementDecision,
    SecurityEvent, SecurityEventId,
};
use capsem_logger::{DbWriter, TransportEvent, TransportEventKind, WriteOp};
use std::sync::{Arc, RwLock};

pub struct NetworkSecurity {
    pub db: Arc<DbWriter>,
    pub rules: Arc<RwLock<Arc<SecurityRuleSet>>>,
    pub plugins: SharedPluginPolicy,
}

impl NetworkSecurity {
    /// Return only after primary audit admission. The caller owns its deadline
    /// and must honor the returned final decision before creating an endpoint.
    pub async fn evaluate_and_record(
        &self,
        event: SecurityEvent,
    ) -> Result<SecurityEnforcementDecision, SecurityActionError> {
        let rules = self
            .rules
            .read()
            .map_err(|_| SecurityActionError::new("network rule snapshot poisoned"))?
            .clone();
        let plugins = self
            .plugins
            .read()
            .map_err(|_| SecurityActionError::new("network plugin snapshot poisoned"))?
            .clone();
        let evaluated = evaluate_security_boundary(&rules, plugins.clone(), event)?;
        let mut decision = evaluated.enforcement;
        let event = prepare_evaluated_event_for_security_rule_ledger(plugins, evaluated.event)
            .map_err(SecurityActionError::new)?;
        apply_event_decision_to_enforcement(&event, &mut decision);
        let kind = transport_kind(event.event_type)?;
        let network = event
            .network
            .as_ref()
            .ok_or_else(|| SecurityActionError::new("network routing facts missing"))?;
        let (network_id, connection_id) = match network {
            NetworkSecurityEvent::Lifecycle { network, .. } => (Some(network.id), None),
            NetworkSecurityEvent::Flow(flow) => {
                let network_id = match &flow.route {
                    NetworkRoute::Private { network } => Some(network.id),
                    NetworkRoute::Expose { .. } => None,
                };
                (network_id, Some(flow.connection_id))
            }
        };
        let id = SecurityEventId::new_uuid4();
        let timestamp = super::super::current_unix_ms();
        let row = TransportEvent::new(
            id.as_str().into(),
            timestamp,
            kind,
            network_id,
            connection_id,
            &event.serializable(),
        )
        .map_err(SecurityActionError::new)?;
        emit_security_write(&self.db, WriteOp::TransportEvent(row))
            .await
            .ok_or_else(|| SecurityActionError::new("network primary audit admission failed"))?;
        let emission =
            emit_matching_security_rules_with_decision(&self.db, id, event.event_type, &rules, &event, timestamp)
                .await
                .map_err(SecurityActionError::new)?;
        decision.ask_id = emission.enforcement.ask_id;
        Ok(decision)
    }
}

fn transport_kind(kind: RuntimeSecurityEventType) -> Result<TransportEventKind, SecurityActionError> {
    Ok(match kind {
        RuntimeSecurityEventType::NetworkConnect => TransportEventKind::Connect,
        RuntimeSecurityEventType::NetworkConnectResult => TransportEventKind::ConnectResult,
        RuntimeSecurityEventType::NetworkClose => TransportEventKind::Close,
        RuntimeSecurityEventType::NetworkLifecycle => TransportEventKind::Lifecycle,
        RuntimeSecurityEventType::NetworkProbe => TransportEventKind::Probe,
        RuntimeSecurityEventType::NetworkProbeResult => TransportEventKind::ProbeResult,
        _ => return Err(SecurityActionError::new("non-network event at routing boundary")),
    })
}

#[cfg(test)]
mod tests;
