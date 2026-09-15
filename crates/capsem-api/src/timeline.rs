use serde::{Deserialize, Deserializer, Serialize};
use utoipa::{IntoParams, ToSchema};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TimelineLayer {
    Exec,
    Tool,
    Net,
    Fs,
    Model,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ToolDecision {
    Allowed,
    Denied,
    Warned,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum TimelineReference {
    Id(i64),
    EventId(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum TimelineStatus {
    Code(i32),
    Decision(ToolDecision),
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TimelineEvent {
    pub timestamp: String,
    pub layer: TimelineLayer,
    #[serde(rename = "ref")]
    pub reference: TimelineReference,
    pub summary: String,
    /// Exit code for exec, HTTP code for net/model, decision for tool, null for fs.
    pub status: Option<TimelineStatus>,
    pub duration_ms: Option<u64>,
    pub trace_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TimelineResponse {
    pub events: Vec<TimelineEvent>,
}

#[derive(Debug, Clone, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct TimelineQuery {
    /// Filter to a trace, also retaining legacy records without a trace ID.
    pub trace_id: Option<String>,
    /// Lookback duration (30m, 1h, 7d) or RFC3339 timestamp.
    pub since: Option<String>,
    /// Maximum events. Defaults to 200, capped at 2000.
    pub limit: Option<usize>,
    /// Comma-separated layers. Defaults to all five layers.
    #[serde(default, deserialize_with = "deserialize_layers")]
    #[param(style = Form, explode = false)]
    pub layers: Option<Vec<TimelineLayer>>,
}

fn deserialize_layers<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Vec<TimelineLayer>>, D::Error> {
    let value = Option::<String>::deserialize(deserializer)?;
    value
        .map(|value| {
            value
                .split(',')
                .map(|layer| TimelineLayer::deserialize(serde::de::value::StrDeserializer::<D::Error>::new(layer)))
                .collect()
        })
        .transpose()
}
