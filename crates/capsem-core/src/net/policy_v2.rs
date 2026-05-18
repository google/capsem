//! Policy V2 runtime rule types and CEL subject surface.
//!
//! `policy_config` owns the legacy settings/defaults loader. Runtime
//! enforcement should depend on this module so Policy V2 can continue to
//! separate from settings storage without touching every callback site.

pub use super::policy_config::{
    MatchedPolicyRule, PolicyCallback, PolicyConfig, PolicyDecisionKind, PolicyRuleConfig,
    PolicySubject, PolicySubjectValue,
};
