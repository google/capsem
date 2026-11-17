//! Shared compiled rule evaluation; observations carry no access action.
use super::*;

impl SecurityRuleSet {
    pub fn new(rules: Vec<CompiledSecurityRule>) -> Self {
        Self::with_observations(rules, Vec::new())
    }

    /// Observations share compiled matching, but cannot enter action selection.
    pub fn with_observations(
        mut rules: Vec<CompiledSecurityRule>,
        mut observations: Vec<CompiledEnvironmentDetectionRule>,
    ) -> Self {
        rules.sort_by(|left, right| {
            left.priority
                .cmp(&right.priority)
                .then_with(|| left.rule_id.cmp(&right.rule_id))
        });
        observations.sort_by(|left, right| left.rule_id.cmp(&right.rule_id));
        Self { rules, observations }
    }

    pub fn compile_profile(profile: &SecurityRuleProfile, source: SecurityRuleSource) -> Result<Self, String> {
        profile.compile(source).map(Self::new)
    }

    pub fn rules(&self) -> &[CompiledSecurityRule] {
        &self.rules
    }

    pub fn observations(&self) -> &[CompiledEnvironmentDetectionRule] {
        &self.observations
    }

    pub fn evaluate<S>(&self, subject: &S) -> Result<SecurityRuleEvaluation<'_>, String>
    where
        S: PolicySubject + ?Sized,
    {
        let mut matched_rules = Vec::new();
        for rule in &self.rules {
            if !rule.enabled {
                continue;
            }
            if rule.matches_security_event(subject)? {
                matched_rules.push(rule);
            }
        }
        let mut observations = Vec::new();
        for observation in &self.observations {
            if observation.evaluate(subject)?.is_some() {
                observations.push(observation);
            }
        }
        Ok(SecurityRuleEvaluation {
            matched_rules,
            observations,
        })
    }
}

impl<'a> SecurityRuleEvaluation<'a> {
    pub fn observations(&self) -> &[&'a CompiledEnvironmentDetectionRule] {
        &self.observations
    }

    pub fn matched_rules(&self) -> &[&'a CompiledSecurityRule] {
        &self.matched_rules
    }

    pub fn detections(&self) -> Vec<&'a CompiledSecurityRule> {
        self.matched_rules
            .iter()
            .copied()
            .filter(|rule| rule.detection_level.is_some())
            .collect()
    }

    pub fn rules_for_action(&self, action: SecurityRuleAction) -> Vec<&'a CompiledSecurityRule> {
        self.matched_rules
            .iter()
            .copied()
            .filter(|rule| rule.action == action)
            .collect()
    }

    pub fn preprocess_rules(&self) -> Vec<&'a CompiledSecurityRule> {
        self.matched_rules
            .iter()
            .copied()
            .filter(|rule| {
                matches!(
                    rule.action,
                    SecurityRuleAction::Preprocess | SecurityRuleAction::Rewrite
                )
            })
            .collect()
    }

    pub fn postprocess_rules(&self) -> Vec<&'a CompiledSecurityRule> {
        self.rules_for_action(SecurityRuleAction::Postprocess)
    }

    pub fn enforcement_rules(&self) -> Vec<&'a CompiledSecurityRule> {
        self.matched_rules
            .iter()
            .copied()
            .filter(|rule| {
                matches!(
                    rule.action,
                    SecurityRuleAction::Allow | SecurityRuleAction::Ask | SecurityRuleAction::Block
                )
            })
            .collect::<Vec<_>>()
    }
}
