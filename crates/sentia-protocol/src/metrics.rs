use crate::bounded::{BoundedString, BoundedVec};
use serde::{Deserialize, Serialize};

pub type MetricName = BoundedString<64>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricUnit {
    Count,
    Milliseconds,
    Bytes,
    Percent,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricSample {
    pub name: MetricName,
    pub unit: MetricUnit,
    pub value: f64,
    pub observed_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouterMetrics {
    pub request_id: BoundedString<64>,
    pub queue_depth: u32,
    pub local_latency_ms: Option<f64>,
    pub remote_latency_ms: Option<f64>,
    pub fallback_count: u32,
    pub cancellation_count: u32,
    pub payload_input_bytes: u32,
    pub payload_output_bytes: u32,
    pub samples: BoundedVec<MetricSample, 128>,
}
