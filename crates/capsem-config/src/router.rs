//! Published-port admission budgets. These tune below the companion's hard
//! ceilings. Traffic between VMs has no budget here: it rides the network
//! switch, whose limits are per port.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RouterConfig {
    pub expose: ClassBudget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ClassBudget {
    pub connections: u16,
    pub setups: u16,
    pub rate_per_second: u16,
    pub burst: u16,
}

impl Default for ClassBudget {
    fn default() -> Self {
        Self {
            connections: 64,
            setups: 8,
            rate_per_second: 32,
            burst: 16,
        }
    }
}

impl RouterConfig {
    pub fn validate(&self) -> Result<(), String> {
        let budget = &self.expose;
        for (field, value, maximum) in [
            ("connections", budget.connections, 64),
            ("setups", budget.setups, 8),
            ("rate_per_second", budget.rate_per_second, 1024),
            ("burst", budget.burst, 64),
        ] {
            if value == 0 || value > maximum {
                return Err(format!("network.router.expose.{field} must be between 1 and {maximum}"));
            }
        }
        Ok(())
    }
}
