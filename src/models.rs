use chrono::{DateTime, Utc};
use honk_config::{node::Node, subscription::Subscription};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SubscriptionFilter {
    pub include: Option<String>,
    pub exclude: Option<String>,
    pub protocol: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SubscriptionSettings {
    pub rename_template: Option<String>,
    pub filter: SubscriptionFilter,
    #[serde(default)]
    pub probe_vless_modes: bool,
}

#[derive(Debug, Clone)]
pub struct StoredSubscription {
    pub value: Subscription,
}

#[derive(Debug, Clone, Default)]
pub struct SubscriptionTraffic {
    pub upload: u64,
    pub download: u64,
    pub total: Option<u64>,
    pub expire: Option<DateTime<Utc>>,
}

impl SubscriptionTraffic {
    pub fn used(&self) -> u64 {
        self.upload.saturating_add(self.download)
    }

    pub fn remaining(&self) -> Option<u64> {
        self.total.map(|total| total.saturating_sub(self.used()))
    }
}

#[derive(Debug, Clone)]
pub struct SubscriptionSnapshot {
    pub subscription: Subscription,
    pub settings: SubscriptionSettings,
    pub node_kinds: Vec<String>,
    pub alive_nodes: usize,
    pub probed_nodes: usize,
    pub traffic: Option<SubscriptionTraffic>,
}

#[derive(Debug, Clone)]
pub struct StoredNode {
    pub node: Node,
    pub display_name: String,
    pub region: Option<String>,
    pub sources: Vec<String>,
    #[cfg_attr(not(feature = "honk-probe"), allow(dead_code))]
    pub probes: Vec<ProbeRecord>,
}

#[derive(Debug, Clone)]
pub struct ProbeRecord {
    pub kind: String,
    pub success: bool,
    pub latency_ms: Option<i64>,
    pub status_code: Option<i64>,
    pub error: Option<String>,
    pub probed_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
#[cfg_attr(not(feature = "honk-probe"), allow(dead_code))]
pub struct ProbeOutcome {
    pub node_id: String,
    pub kind: String,
    pub success: bool,
    pub latency_ms: Option<i64>,
    pub status_code: Option<i64>,
    pub error: Option<String>,
    pub probed_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NodeSnapshot {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub address: String,
    pub region: Option<String>,
    pub sources: Vec<String>,
    pub latency_ms: Option<i64>,
    pub stability_percent: Option<f64>,
    pub ai_unlocked: Option<bool>,
    pub netflix_unlocked: Option<bool>,
    pub last_probe_ok: Option<bool>,
    pub last_probe_at: Option<DateTime<Utc>>,
    pub last_probe_status_code: Option<i64>,
    pub last_probe_error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RefreshSummary {
    pub subscription: SubscriptionSnapshot,
    pub imported_nodes: usize,
}
