use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use honk_config::{
    node::Node,
    subscription::{Subscription, SubscriptionHeader},
    types::SubscriptionType,
};
use serde_json::Value;
use uuid::Uuid;

use crate::{
    error::{AppError, AppResult},
    export::{self, ExportFormat, ExportLimits},
    models::{
        NodeSnapshot, RefreshSummary, StoredNode, SubscriptionFilter, SubscriptionSettings,
        SubscriptionSnapshot,
    },
    naming,
    probe::{ProbeEngine, ProbeKind, VLESS_MODE_PROBE_TIMEOUT, latest_probe},
    store::Store,
    subscription::{SubscriptionFetcher, parse_share_link},
};

#[derive(Clone)]
pub struct Service {
    store: Arc<Store>,
    fetcher: Arc<SubscriptionFetcher>,
    probes: Arc<ProbeEngine>,
}

#[derive(Debug, Clone, Default)]
pub struct NodeFilter {
    pub search: Option<String>,
    pub region: Option<String>,
    pub protocol: Option<String>,
    pub max_latency_ms: Option<i64>,
    pub min_stability_percent: Option<f64>,
    pub ai_unlocked: Option<bool>,
    pub netflix_unlocked: Option<bool>,
}

pub struct SubscriptionOptions {
    pub update_interval: i64,
    pub enabled: bool,
    pub user_agent: Option<String>,
    pub headers: Vec<SubscriptionHeader>,
    pub settings: SubscriptionSettings,
}

impl Service {
    pub fn new(store: Arc<Store>) -> anyhow::Result<Self> {
        Ok(Self {
            store,
            fetcher: Arc::new(SubscriptionFetcher::new()?),
            probes: Arc::new(ProbeEngine),
        })
    }

    pub fn health(&self) -> AppResult<()> {
        self.store.health()
    }

    pub fn export(&self, format: ExportFormat, limits: ExportLimits) -> AppResult<String> {
        let nodes = export::select_nodes(self.store.list_nodes()?, limits);
        export::render(format, &nodes).map_err(AppError::from)
    }

    pub fn add_subscription(
        &self,
        name: String,
        url: String,
        kind: String,
        options: SubscriptionOptions,
    ) -> AppResult<SubscriptionSnapshot> {
        let name = required_text(name, "subscription name")?;
        let url = required_text(url, "subscription URL")?;
        let sub_type = parse_subscription_type(&kind)?;
        let value = Subscription {
            id: Uuid::new_v4(),
            name,
            url,
            sub_type,
            update_interval: u64::try_from(options.update_interval.max(0)).unwrap_or(86400),
            enabled: options.enabled,
            user_agent: options.user_agent.filter(|value| !value.trim().is_empty()),
            headers: options.headers,
            ..Default::default()
        };
        self.store
            .save_subscription(&value, &normalize_settings(options.settings))?;
        self.subscription_snapshot(value)
    }

    pub fn subscriptions(&self) -> AppResult<Vec<SubscriptionSnapshot>> {
        self.store
            .list_subscriptions()?
            .into_iter()
            .map(|row| self.subscription_snapshot(row.value))
            .collect()
    }

    pub async fn refresh_subscription(&self, id: String) -> AppResult<RefreshSummary> {
        let subscription = self.store.get_subscription(&id)?;
        let fetched = self
            .fetcher
            .fetch(&subscription)
            .await
            .map_err(AppError::from)?;
        let settings = self.store.subscription_settings(subscription.id)?;
        let fetched_nodes = fetched
            .nodes
            .into_iter()
            .map(|mut node| {
                apply_honk_node_defaults(&mut node);
                node.id = node.derive_id();
                node
            })
            .collect();
        let nodes = filter_subscription_nodes(fetched_nodes, &settings.filter);
        let nodes = if settings.probe_vless_modes {
            self.probes
                .detect_vless_modes(nodes, VLESS_MODE_PROBE_TIMEOUT)
                .await
        } else {
            nodes
        };
        let nodes = rename_subscription_nodes(nodes, &settings, &subscription.name);
        self.store
            .replace_subscription_nodes(&subscription, &nodes)?;
        let updated = self.store.update_subscription_after_refresh(
            subscription.id,
            nodes.len(),
            chrono::Utc::now(),
            fetched.traffic.as_ref(),
        )?;
        Ok(RefreshSummary {
            subscription: self.subscription_snapshot(updated)?,
            imported_nodes: nodes.len(),
        })
    }

    pub fn update_subscription(
        &self,
        id: String,
        name: String,
        url: String,
        kind: String,
        options: SubscriptionOptions,
    ) -> AppResult<SubscriptionSnapshot> {
        let mut value = self.store.get_subscription(&id)?;
        value.name = required_text(name, "subscription name")?;
        value.url = required_text(url, "subscription URL")?;
        value.sub_type = parse_subscription_type(&kind)?;
        value.update_interval = u64::try_from(options.update_interval.max(0)).unwrap_or(86400);
        value.enabled = options.enabled;
        value.user_agent = options.user_agent.filter(|item| !item.trim().is_empty());
        if !options.headers.is_empty() {
            value.headers = options.headers;
        }
        self.store
            .save_subscription(&value, &normalize_settings(options.settings))?;
        self.subscription_snapshot(value)
    }

    pub fn delete_subscription(&self, id: String) -> AppResult<()> {
        let id = Uuid::parse_str(&id)
            .map_err(|_| AppError::InvalidInput("invalid subscription id".into()))?;
        self.store.delete_subscription(id)
    }

    pub fn rename_subscription_nodes(&self, id: String) -> AppResult<SubscriptionSnapshot> {
        let subscription = self.store.get_subscription(&id)?;
        let node_ids = self
            .store
            .subscription_node_ids(subscription.id)?
            .into_iter()
            .collect::<HashSet<_>>();
        let mut nodes = self
            .nodes(NodeFilter::default())?
            .into_iter()
            .filter(|node| node_ids.contains(&node.id))
            .collect::<Vec<_>>();
        nodes.sort_by_key(|node| node.id.clone());
        let changes = nodes
            .iter()
            .enumerate()
            .map(|(index, node)| {
                (
                    node.id.clone(),
                    naming::renamed(&node.name, &subscription.name, index + 1),
                )
            })
            .collect::<Vec<_>>();
        self.store.rename_nodes(&changes)?;
        self.subscription_snapshot(subscription)
    }

    pub fn add_manual_node(
        &self,
        share_link: Option<String>,
        config: Option<Value>,
        name: Option<String>,
        region: Option<String>,
    ) -> AppResult<Node> {
        let mut node = match share_link.filter(|value| !value.trim().is_empty()) {
            Some(share_link) => parse_share_link(&share_link)
                .map_err(|error| AppError::InvalidInput(error.to_string()))?,
            None => node_from_config(config)?,
        };
        if let Some(name) = name.filter(|value| !value.trim().is_empty()) {
            node.name = name.trim().to_string();
        }
        apply_honk_node_defaults(&mut node);
        node.subscription_id = None;
        node.id = node.derive_id();
        self.store.save_manual_node(&node, region.as_deref())?;
        Ok(node)
    }

    pub fn rename_nodes(&self, ids: Vec<String>, template: String) -> AppResult<Vec<NodeSnapshot>> {
        if ids.is_empty() {
            return Err(AppError::InvalidInput("select at least one node".into()));
        }
        let template = required_text(template, "rename template")?;
        let nodes = self.store.list_nodes()?;
        let by_id: HashMap<String, StoredNode> = nodes
            .into_iter()
            .map(|node| (node.node.id.to_string(), node))
            .collect();
        let changes = ids
            .iter()
            .enumerate()
            .map(|(index, id)| {
                let node = by_id.get(id).ok_or(AppError::NotFound)?;
                let name = render_name(
                    &template,
                    index + 1,
                    &node.display_name,
                    node.region.as_deref(),
                );
                Ok((id.clone(), name))
            })
            .collect::<AppResult<Vec<_>>>()?;
        self.store.rename_nodes(&changes)?;
        self.nodes(NodeFilter::default())
    }

    pub fn nodes(&self, filter: NodeFilter) -> AppResult<Vec<NodeSnapshot>> {
        self.store
            .list_nodes()?
            .into_iter()
            .map(snapshot)
            .filter(|result| {
                result
                    .as_ref()
                    .is_ok_and(|node| matches_filter(node, &filter))
            })
            .collect()
    }

    pub async fn probe_nodes(
        &self,
        ids: Vec<String>,
        kinds: Vec<String>,
        timeout_ms: i64,
    ) -> AppResult<Vec<NodeSnapshot>> {
        self.probe_nodes_enabled(ids, kinds, timeout_ms).await
    }

    async fn probe_nodes_enabled(
        &self,
        ids: Vec<String>,
        kinds: Vec<String>,
        timeout_ms: i64,
    ) -> AppResult<Vec<NodeSnapshot>> {
        let selected = if ids.is_empty() {
            self.store.list_nodes()?
        } else {
            self.store
                .list_nodes()?
                .into_iter()
                .filter(|node| ids.contains(&node.node.id.to_string()))
                .collect()
        };
        if selected.is_empty() {
            return Err(AppError::NotFound);
        }
        let kinds = if kinds.is_empty() {
            vec![ProbeKind::Latency, ProbeKind::Ai, ProbeKind::Netflix]
        } else {
            kinds
                .iter()
                .filter_map(|kind| ProbeKind::parse(kind))
                .collect()
        };
        if kinds.is_empty() {
            return Err(AppError::InvalidInput("unknown probe kind".into()));
        }
        let items = selected
            .into_iter()
            .flat_map(|node| kinds.iter().map(move |kind| (node.node.clone(), *kind)))
            .collect();
        let outcomes = self
            .probes
            .many(
                items,
                Duration::from_millis(u64::try_from(timeout_ms.clamp(500, 60000)).unwrap_or(5000)),
            )
            .await;
        #[cfg(not(feature = "honk-probe"))]
        {
            let _ = outcomes;
            Err(AppError::InvalidInput(
                "enable the honk-probe feature for proxy-routed probes".into(),
            ))
        }
        #[cfg(feature = "honk-probe")]
        for outcome in &outcomes {
            self.store.save_probe(outcome)?;
        }
        #[cfg(feature = "honk-probe")]
        self.nodes(NodeFilter::default())
    }

    fn subscription_snapshot(
        &self,
        mut subscription: Subscription,
    ) -> AppResult<SubscriptionSnapshot> {
        let node_ids = self
            .store
            .subscription_node_ids(subscription.id)?
            .into_iter()
            .collect::<HashSet<_>>();
        let related = self
            .nodes(NodeFilter::default())?
            .into_iter()
            .filter(|node| node_ids.contains(&node.id))
            .collect::<Vec<_>>();
        let mut node_kinds = related
            .iter()
            .map(|node| node.protocol.clone())
            .collect::<Vec<_>>();
        node_kinds.sort_unstable();
        node_kinds.dedup();
        subscription.node_count = u32::try_from(related.len()).unwrap_or(u32::MAX);
        let subscription_id = subscription.id;
        Ok(SubscriptionSnapshot {
            subscription,
            node_kinds,
            alive_nodes: related
                .iter()
                .filter(|node| node.last_probe_ok == Some(true))
                .count(),
            probed_nodes: related
                .iter()
                .filter(|node| node.last_probe_ok.is_some())
                .count(),
            settings: self.store.subscription_settings(subscription_id)?,
            traffic: self.store.subscription_traffic(subscription_id)?,
        })
    }
}

#[cfg(test)]
fn apply_subscription_settings(
    nodes: Vec<Node>,
    settings: &SubscriptionSettings,
    provider: &str,
) -> Vec<Node> {
    rename_subscription_nodes(
        filter_subscription_nodes(nodes, &settings.filter),
        settings,
        provider,
    )
}

fn filter_subscription_nodes(mut nodes: Vec<Node>, filter: &SubscriptionFilter) -> Vec<Node> {
    nodes.retain(|node| matches_subscription_filter(node, filter));
    nodes
}

fn rename_subscription_nodes(
    mut nodes: Vec<Node>,
    settings: &SubscriptionSettings,
    provider: &str,
) -> Vec<Node> {
    if let Some(template) = settings.rename_template.as_deref() {
        nodes.sort_by_key(|node| node.id);
        for (index, node) in nodes.iter_mut().enumerate() {
            node.name = naming::rendered(&node.name, provider, index + 1, template);
        }
    }
    nodes
}

fn matches_subscription_filter(node: &Node, filter: &SubscriptionFilter) -> bool {
    let searchable = format!("{} {}", node.name, node.address).to_ascii_lowercase();
    let matches_include = filter
        .include
        .as_deref()
        .is_none_or(|value| searchable.contains(&value.to_ascii_lowercase()));
    let matches_exclude = filter
        .exclude
        .as_deref()
        .is_none_or(|value| !searchable.contains(&value.to_ascii_lowercase()));
    let matches_protocol = filter
        .protocol
        .as_deref()
        .is_none_or(|value| node.protocol.as_str().eq_ignore_ascii_case(value));
    matches_include && matches_exclude && matches_protocol
}

fn normalize_settings(mut settings: SubscriptionSettings) -> SubscriptionSettings {
    settings.rename_template = clean_optional(settings.rename_template);
    settings.filter.include = clean_optional(settings.filter.include);
    settings.filter.exclude = clean_optional(settings.filter.exclude);
    settings.filter.protocol = clean_optional(settings.filter.protocol);
    settings
}

fn clean_optional(value: Option<String>) -> Option<String> {
    value
        .filter(|item| !item.trim().is_empty())
        .map(|item| item.trim().to_string())
}

fn node_from_config(config: Option<Value>) -> AppResult<Node> {
    let mut object = config
        .ok_or_else(|| AppError::InvalidInput("share link or node config is required".into()))?
        .as_object()
        .cloned()
        .ok_or_else(|| AppError::InvalidInput("node config must be a JSON object".into()))?;
    object
        .entry("protocol")
        .or_insert_with(|| Value::String("ss".into()));
    object.entry("port").or_insert_with(|| Value::from(443));
    object
        .entry("name")
        .or_insert_with(|| Value::String("manual-node".into()));
    apply_honk_config_defaults(&mut object);
    let address = object
        .get("address")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .or_else(|| {
            object
                .get("host")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(str::to_owned)
        })
        .ok_or_else(|| AppError::InvalidInput("node config requires address or host".into()))?;
    object.insert("address".into(), Value::String(address));
    let mut node: Node = serde_json::from_value(Value::Object(object))
        .map_err(|error| AppError::InvalidInput(format!("invalid node config: {error}")))?;
    if node.host.is_empty() {
        node.host = node.address.clone();
    }
    apply_honk_node_defaults(&mut node);
    node.id = node.derive_id();
    Ok(node)
}

fn apply_honk_config_defaults(object: &mut serde_json::Map<String, Value>) {
    let protocol = object
        .get("protocol")
        .and_then(Value::as_str)
        .unwrap_or("ss")
        .to_ascii_lowercase();
    object
        .entry("transport")
        .or_insert_with(|| Value::String("tcp".into()));
    match protocol.as_str() {
        "ss" | "shadowsocks" => {
            object
                .entry("encryption")
                .or_insert_with(|| Value::String("aes-128-gcm".into()));
        }
        "trojan" | "vless" | "anytls" => {
            object.entry("tls").or_insert_with(|| Value::Bool(true));
        }
        _ => {}
    }
    match protocol.as_str() {
        "hysteria2" | "hysteria" => {
            object
                .entry("quic_mtu")
                .or_insert_with(|| Value::from(1252));
            object
                .entry("hy2_init_stream_recv_window")
                .or_insert_with(|| Value::from(8_u64 << 20));
            object
                .entry("hy2_init_conn_recv_window")
                .or_insert_with(|| Value::from(32_u64 << 20));
        }
        "tuic" => {
            object
                .entry("quic_mtu")
                .or_insert_with(|| Value::from(1252));
            object
                .entry("tuic_congestion")
                .or_insert_with(|| Value::String("cubic".into()));
            object
                .entry("tuic_alpn")
                .or_insert_with(|| Value::String("tuic".into()));
            object
                .entry("tuic_init_stream_recv_window")
                .or_insert_with(|| Value::from(8_u64 << 20));
            object
                .entry("tuic_init_conn_recv_window")
                .or_insert_with(|| Value::from(32_u64 << 20));
        }
        "juicity" => {
            object
                .entry("quic_mtu")
                .or_insert_with(|| Value::from(1252));
        }
        "anytls" => {
            object
                .entry("anytls_idle_session_timeout")
                .or_insert_with(|| Value::from(30));
        }
        _ => {}
    }
    if protocol == "vless"
        && object.contains_key("reality_public_key")
        && !object.contains_key("reality_spider_x")
    {
        object.insert("reality_spider_x".into(), Value::String("/".into()));
    }
}

fn apply_honk_node_defaults(node: &mut Node) {
    if node.transport.trim().is_empty() {
        node.transport = "tcp".into();
    }
    if node
        .encryption
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        node.encryption = None;
    }
    if node.protocol == honk_config::types::NodeProtocol::SS
        && node
            .encryption
            .as_deref()
            .is_none_or(|value| value.trim().is_empty())
    {
        node.encryption = Some("aes-128-gcm".into());
    }
    if node.protocol == honk_config::types::NodeProtocol::AnyTLS
        && node
            .anytls_password
            .as_deref()
            .is_none_or(|value| value.trim().is_empty())
    {
        node.anytls_password = node.password.clone();
    }
    match node.protocol {
        honk_config::types::NodeProtocol::Hysteria2 => {
            node.quic_mtu.get_or_insert(1252);
            node.hy2_init_stream_recv_window.get_or_insert(8 << 20);
            node.hy2_init_conn_recv_window.get_or_insert(32 << 20);
        }
        honk_config::types::NodeProtocol::Tuic => {
            node.quic_mtu.get_or_insert(1252);
            node.tuic_congestion.get_or_insert_with(|| "cubic".into());
            node.tuic_alpn.get_or_insert_with(|| "tuic".into());
            node.tuic_init_stream_recv_window.get_or_insert(8 << 20);
            node.tuic_init_conn_recv_window.get_or_insert(32 << 20);
        }
        honk_config::types::NodeProtocol::Juicity => {
            node.quic_mtu.get_or_insert(1252);
        }
        honk_config::types::NodeProtocol::AnyTLS => {
            node.anytls_idle_session_timeout.get_or_insert(30);
        }
        _ => {}
    }
    if node.protocol == honk_config::types::NodeProtocol::VLess && node.reality_public_key.is_some()
    {
        node.reality_spider_x.get_or_insert_with(|| "/".into());
    }
}

fn snapshot(stored: StoredNode) -> AppResult<NodeSnapshot> {
    #[cfg(feature = "honk-probe")]
    let probes = stored.probes.as_slice();
    #[cfg(not(feature = "honk-probe"))]
    let probes: &[crate::models::ProbeRecord] = &[];
    let latency = latest_probe(probes, ProbeKind::Latency);
    let ai = latest_probe(probes, ProbeKind::Ai);
    let netflix = latest_probe(probes, ProbeKind::Netflix);
    let latency_history = probes
        .iter()
        .filter(|probe| probe.kind == ProbeKind::Latency.as_str())
        .take(20)
        .collect::<Vec<_>>();
    let stability = if latency_history.is_empty() {
        None
    } else {
        Some(
            latency_history.iter().filter(|probe| probe.success).count() as f64
                / latency_history.len() as f64
                * 100.0,
        )
    };
    let last = probes.first();
    Ok(NodeSnapshot {
        id: stored.node.id.to_string(),
        name: stored.display_name,
        protocol: stored.node.protocol.as_str().into(),
        address: stored.node.address,
        region: stored.region,
        sources: stored.sources,
        latency_ms: latency.and_then(|probe| probe.latency_ms),
        stability_percent: stability,
        ai_unlocked: ai.map(|probe| probe.success),
        netflix_unlocked: netflix.map(|probe| probe.success),
        last_probe_ok: last.map(|probe| probe.success),
        last_probe_at: last.map(|probe| probe.probed_at),
        last_probe_status_code: last.and_then(|probe| probe.status_code),
        last_probe_error: last.and_then(|probe| probe.error.clone()),
    })
}

fn matches_filter(node: &NodeSnapshot, filter: &NodeFilter) -> bool {
    let search = filter
        .search
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let matches_search = search.is_empty()
        || node.name.to_ascii_lowercase().contains(&search)
        || node.address.to_ascii_lowercase().contains(&search);
    let matches_region = filter.region.as_deref().is_none_or(|region| {
        node.region
            .as_deref()
            .is_some_and(|value| value.eq_ignore_ascii_case(region))
    });
    let matches_protocol = filter
        .protocol
        .as_deref()
        .is_none_or(|protocol| node.protocol.eq_ignore_ascii_case(protocol));
    let matches_latency = filter
        .max_latency_ms
        .is_none_or(|max| node.latency_ms.is_some_and(|value| value <= max));
    let matches_stability = filter
        .min_stability_percent
        .is_none_or(|min| node.stability_percent.is_some_and(|value| value >= min));
    let matches_ai = filter
        .ai_unlocked
        .is_none_or(|value| node.ai_unlocked == Some(value));
    let matches_netflix = filter
        .netflix_unlocked
        .is_none_or(|value| node.netflix_unlocked == Some(value));
    matches_search
        && matches_region
        && matches_protocol
        && matches_latency
        && matches_stability
        && matches_ai
        && matches_netflix
}

fn render_name(template: &str, index: usize, old: &str, region: Option<&str>) -> String {
    template
        .replace("{index}", &index.to_string())
        .replace("{name}", old)
        .replace("{region}", region.unwrap_or("未分区"))
        .trim()
        .to_string()
}

fn required_text(value: String, field: &str) -> AppResult<String> {
    let value = value.trim().to_string();
    if value.is_empty() {
        Err(AppError::InvalidInput(format!("{field} is required")))
    } else {
        Ok(value)
    }
}

fn parse_subscription_type(value: &str) -> AppResult<SubscriptionType> {
    match value.to_ascii_lowercase().as_str() {
        "simple" => Ok(SubscriptionType::Simple),
        "clash" => Ok(SubscriptionType::Clash),
        "sip008" => Ok(SubscriptionType::Sip008),
        "custom" | "" => Ok(SubscriptionType::Custom),
        _ => Err(AppError::InvalidInput(
            "unsupported subscription type".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use serde_json::json;

    use crate::models::{ProbeRecord, StoredNode, SubscriptionFilter, SubscriptionSettings};

    use super::{apply_subscription_settings, node_from_config};

    #[test]
    fn manual_config_uses_honk_protocol_defaults() {
        let ss = node_from_config(Some(json!({
            "protocol": "ss",
            "host": "ss.example.com",
            "password": "secret"
        })))
        .expect("ss config should parse");
        assert_eq!(ss.encryption.as_deref(), Some("aes-128-gcm"));
        assert_eq!(ss.transport, "tcp");

        let trojan = node_from_config(Some(json!({
            "protocol": "trojan",
            "host": "trojan.example.com",
            "password": "secret"
        })))
        .expect("trojan config should parse");
        assert!(trojan.tls);
        assert_eq!(trojan.transport, "tcp");

        let vless = node_from_config(Some(json!({
            "protocol": "vless",
            "host": "vless.example.com",
            "password": "uuid"
        })))
        .expect("vless config should parse");
        assert!(vless.tls);
        assert_eq!(vless.encryption, None);

        let anytls = node_from_config(Some(json!({
            "protocol": "anytls",
            "host": "anytls.example.com",
            "password": "secret"
        })))
        .expect("anytls config should parse");
        assert!(anytls.tls);
        assert_eq!(anytls.anytls_password.as_deref(), Some("secret"));

        let hysteria2 = node_from_config(Some(json!({
            "protocol": "hysteria2",
            "host": "hy2.example.com",
            "password": "secret"
        })))
        .expect("hysteria2 config should parse");
        assert_eq!(hysteria2.quic_mtu, Some(1252));
        assert_eq!(hysteria2.hy2_init_stream_recv_window, Some(8 << 20));
        assert_eq!(hysteria2.hy2_init_conn_recv_window, Some(32 << 20));

        let tuic = node_from_config(Some(json!({
            "protocol": "tuic",
            "host": "tuic.example.com",
            "username": "00000000-0000-0000-0000-000000000000",
            "password": "secret"
        })))
        .expect("tuic config should parse");
        assert_eq!(tuic.quic_mtu, Some(1252));
        assert_eq!(tuic.tuic_congestion.as_deref(), Some("cubic"));
        assert_eq!(tuic.tuic_alpn.as_deref(), Some("tuic"));

        let explicit = node_from_config(Some(json!({
            "protocol": "ss",
            "host": "ss.example.com",
            "password": "secret",
            "encryption": "chacha20-ietf-poly1305",
            "transport": "tcp",
            "tls": true
        })))
        .expect("explicit values should parse");
        assert_eq!(
            explicit.encryption.as_deref(),
            Some("chacha20-ietf-poly1305")
        );
        assert!(explicit.tls);
    }

    #[test]
    fn manual_config_uses_optional_defaults() {
        let node = node_from_config(Some(json!({
            "protocol": "trojan",
            "host": "example.com",
            "password": "secret"
        })))
        .expect("partial node config is valid");
        assert_eq!(node.port, 443);
        assert_eq!(node.address, "example.com");
        assert_eq!(node.name, "manual-node");
    }

    #[test]
    fn subscription_settings_filter_then_rename() {
        let nodes = vec![
            node_from_config(Some(json!({
                "protocol": "trojan",
                "host": "keep.example.com",
                "name": "keep line"
            })))
            .expect("keep node"),
            node_from_config(Some(json!({
                "protocol": "ss",
                "host": "keep.example.com",
                "name": "keep other"
            })))
            .expect("protocol-filtered node"),
        ];
        let settings = SubscriptionSettings {
            rename_template: Some("{index} · {name}".into()),
            filter: SubscriptionFilter {
                include: Some("keep".into()),
                exclude: None,
                protocol: Some("trojan".into()),
            },
            probe_vless_modes: false,
        };
        let result = apply_subscription_settings(nodes, &settings, "Edit QA Provider");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "01 · keep line");
    }

    #[cfg(not(feature = "honk-probe"))]
    #[test]
    fn default_build_hides_persisted_probe_history_without_proxy_probe_support() {
        let node = node_from_config(Some(json!({
            "protocol": "trojan",
            "host": "example.com",
            "password": "secret"
        })))
        .expect("probe fixture should parse");
        let snapshot = super::snapshot(StoredNode {
            node,
            display_name: "probe".into(),
            region: None,
            sources: vec!["manual".into()],
            probes: vec![ProbeRecord {
                kind: "latency".into(),
                success: true,
                latency_ms: Some(42),
                status_code: None,
                error: None,
                probed_at: Utc::now(),
            }],
        })
        .expect("snapshot should render");
        assert_eq!(snapshot.latency_ms, None);
        assert_eq!(snapshot.last_probe_ok, None);
    }
}
