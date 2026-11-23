//! The shapes of the usage endpoint's JSON.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

/// One rate-limit window: how much of it is used and when it starts over.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Window {
    /// Percent used, 0 to 100.
    pub utilization: f64,
    pub resets_at: Option<Timestamp>,
}

/// What `/api/oauth/usage` reports. A missing window means no such limit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    #[serde(rename = "five_hour", default)]
    pub five_hour: Option<Window>,
    #[serde(rename = "seven_day", default)]
    pub seven_day: Option<Window>,
}
