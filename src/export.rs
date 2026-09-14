use std::{cmp::Ordering, collections::HashMap, fmt::Write as _};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use honk_config::{
    node::{Node, WireMode},
    types::NodeProtocol,
};
use serde_json::{Map, Value, json};

use crate::models::StoredNode;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExportLimits {
    pub total: Option<usize>,
    pub per_region: Option<usize>,
    pub per_provider: Option<usize>,
    pub providers: Vec<String>,
    pub exclude_dead: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Sip002,
    Sip008,
    ShareLinks,
    Clash,
    SingBox,
    Surge,
    Dae,
}

impl ExportFormat {
    pub fn parse(value: Option<&str>) -> Result<Self, String> {
        match value
            .unwrap_or("clash")
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "sip002" | "sip-002" | "ss" => Ok(Self::Sip002),
            "sip008" | "sip-008" => Ok(Self::Sip008),
            "simple" | "share-link" | "share-links" | "links" | "honk" => Ok(Self::ShareLinks),
            "clash" | "clash-meta" | "yaml" | "yml" => Ok(Self::Clash),
            "sing-box" | "singbox" | "json" => Ok(Self::SingBox),
            "surge" => Ok(Self::Surge),
            "dae" | "toml" => Ok(Self::Dae),
            _ => {
                Err("format must be sip002, sip008, simple, clash, sing-box, surge, or dae".into())
            }
        }
    }

    pub const fn content_type(self) -> &'static str {
        match self {
            Self::Sip002 | Self::ShareLinks | Self::Clash | Self::Surge | Self::Dae => {
                "text/plain; charset=utf-8"
            }
            Self::Sip008 | Self::SingBox => "application/json; charset=utf-8",
        }
    }

    pub const fn extension(self) -> &'static str {
        match self {
            Self::Sip002 => "txt",
            Self::Sip008 => "json",
            Self::ShareLinks => "txt",
            Self::Clash => "yaml",
            Self::SingBox => "json",
            Self::Surge => "conf",
            Self::Dae => "dae",
        }
    }
}

pub fn render(format: ExportFormat, nodes: &[StoredNode]) -> anyhow::Result<String> {
    match format {
        ExportFormat::Sip002 => Ok(sip002_config(nodes)),
        ExportFormat::Sip008 => Ok(serde_json::to_string_pretty(&sip008_config(nodes))?),
        ExportFormat::ShareLinks => Ok(share_link_config(nodes)),
        ExportFormat::Clash => Ok(serde_yaml::to_string(&clash_config(nodes))?),
        ExportFormat::SingBox => Ok(serde_json::to_string_pretty(&sing_box_config(nodes))?),
        ExportFormat::Surge => Ok(surge_config(nodes)),
        ExportFormat::Dae => Ok(dae_config(nodes)),
    }
}

pub fn select_nodes(mut nodes: Vec<StoredNode>, limits: ExportLimits) -> Vec<StoredNode> {
    if limits.exclude_dead {
        nodes.retain(|node| !is_dead(node));
    }
    let has_caps =
        limits.total.is_some() || limits.per_region.is_some() || limits.per_provider.is_some();
    if !has_caps {
        return if limits.providers.is_empty() {
            nodes
        } else {
            nodes
                .into_iter()
                .filter(|node| provider_matches(node, &limits.providers))
                .collect()
        };
    }
    nodes.sort_by(node_quality_cmp);
    let mut selected = Vec::new();
    let mut regions = HashMap::<String, usize>::new();
    let mut providers = HashMap::<String, usize>::new();
    for node in nodes {
        let provider_names = node_provider_names(&node);
        if !provider_matches(&node, &limits.providers) {
            continue;
        }
        if limits.total.is_some_and(|limit| selected.len() >= limit) {
            break;
        }
        let region = node.region.as_deref().unwrap_or("未分区").to_string();
        if limits
            .per_region
            .is_some_and(|limit| regions.get(&region).copied().unwrap_or_default() >= limit)
        {
            continue;
        }
        if limits.per_provider.is_some_and(|limit| {
            provider_names
                .iter()
                .any(|provider| providers.get(provider).copied().unwrap_or_default() >= limit)
        }) {
            continue;
        }
        *regions.entry(region).or_default() += 1;
        for provider in provider_names {
            *providers.entry(provider).or_default() += 1;
        }
        selected.push(node);
    }
    selected
}

fn is_dead(node: &StoredNode) -> bool {
    node.probes
        .iter()
        .find(|probe| probe.kind == "latency")
        .is_some_and(|probe| !probe.success)
}

fn node_provider_names(node: &StoredNode) -> Vec<String> {
    if node.sources.is_empty() {
        vec!["手动节点".to_string()]
    } else {
        node.sources.clone()
    }
}

fn provider_matches(node: &StoredNode, selected: &[String]) -> bool {
    selected.is_empty()
        || node_provider_names(node).iter().any(|provider| {
            selected
                .iter()
                .any(|wanted| provider.eq_ignore_ascii_case(wanted.trim()))
        })
}

fn node_quality_cmp(left: &StoredNode, right: &StoredNode) -> Ordering {
    let left = node_quality(left);
    let right = node_quality(right);
    right
        .health
        .cmp(&left.health)
        .then_with(|| right.stability.total_cmp(&left.stability))
        .then_with(|| compare_latency(left.latency, right.latency))
        .then_with(|| right.unlocks.cmp(&left.unlocks))
        .then_with(|| left.name.cmp(&right.name))
        .then_with(|| left.id.cmp(&right.id))
}

struct NodeQuality {
    id: String,
    name: String,
    health: u8,
    stability: f64,
    latency: Option<i64>,
    unlocks: u8,
}

fn node_quality(node: &StoredNode) -> NodeQuality {
    let latest_latency = node.probes.iter().find(|probe| probe.kind == "latency");
    let latency_history = node
        .probes
        .iter()
        .filter(|probe| probe.kind == "latency")
        .take(20)
        .collect::<Vec<_>>();
    let stability = if latency_history.is_empty() {
        0.0
    } else {
        latency_history.iter().filter(|probe| probe.success).count() as f64
            / latency_history.len() as f64
    };
    let unlocks = node
        .probes
        .iter()
        .filter(|probe| matches!(probe.kind.as_str(), "ai" | "netflix") && probe.success)
        .count()
        .min(u8::MAX as usize) as u8;
    NodeQuality {
        id: node.node.id.to_string(),
        name: node.display_name.clone(),
        health: match latest_latency.map(|probe| probe.success) {
            Some(true) => 2,
            Some(false) => 0,
            None => 1,
        },
        stability,
        latency: latest_latency.and_then(|probe| probe.latency_ms),
        unlocks,
    }
}

fn compare_latency(left: Option<i64>, right: Option<i64>) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => left.cmp(&right),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn sip002_config(nodes: &[StoredNode]) -> String {
    nodes
        .iter()
        .filter_map(|stored| sip002_link(&stored.node))
        .collect::<Vec<_>>()
        .join("\n")
}

fn sip002_link(node: &Node) -> Option<String> {
    if node.protocol != NodeProtocol::SS {
        return None;
    }
    let method = node.encryption.as_deref().unwrap_or("aes-128-gcm");
    let password = node.password.as_deref().unwrap_or_default();
    let credentials = URL_SAFE_NO_PAD.encode(format!("{method}:{password}"));
    let mut link = format!("ss://{credentials}@{}:{}", format_host(node), node.port);
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    if let Some(plugin) = node.plugin.as_deref() {
        let plugin = node.plugin_opts.as_deref().map_or_else(
            || plugin.to_string(),
            |options| format!("{plugin};{options}"),
        );
        query.append_pair("plugin", &plugin);
    }
    let query = query.finish();
    if !query.is_empty() {
        link.push('?');
        link.push_str(&query);
    }
    if !node.name.is_empty() {
        link.push('#');
        link.push_str(&percent_encode(&node.name));
    }
    Some(link)
}

fn sip008_config(nodes: &[StoredNode]) -> Value {
    json!({
        "version": 1,
        "servers": nodes.iter().filter_map(sip008_node).collect::<Vec<_>>()
    })
}

fn sip008_node(stored: &StoredNode) -> Option<Value> {
    let node = &stored.node;
    if node.protocol != NodeProtocol::SS {
        return None;
    }
    let mut value = Map::from_iter([
        ("remarks".into(), Value::String(stored.display_name.clone())),
        ("server".into(), Value::String(node.host().to_string())),
        ("server_port".into(), Value::from(node.port)),
        (
            "password".into(),
            Value::String(node.password.clone().unwrap_or_default()),
        ),
        (
            "method".into(),
            Value::String(
                node.encryption
                    .clone()
                    .unwrap_or_else(|| "aes-128-gcm".into()),
            ),
        ),
    ]);
    insert_string(&mut value, "plugin", node.plugin.as_deref());
    insert_string(&mut value, "plugin_opts", node.plugin_opts.as_deref());
    Some(Value::Object(value))
}

fn surge_config(nodes: &[StoredNode]) -> String {
    let entries = nodes.iter().filter_map(surge_node).collect::<Vec<_>>();
    let names = nodes
        .iter()
        .filter_map(|stored| surge_node(stored).map(|_| surge_name(&stored.display_name)))
        .collect::<Vec<_>>();
    let mut output =
        String::from("[General]\nskip-proxy = 192.168.0.0/16, 10.0.0.0/8\n\n[Proxy]\n");
    output.push_str(&entries.join("\n"));
    output.push_str("\n\n[Proxy Group]\nAUTO = url-test, ");
    output.push_str(&names.join(", "));
    output.push_str(", url=http://www.gstatic.com/generate_204\n\n[Rule]\nFINAL, AUTO\n");
    output
}

fn surge_node(stored: &StoredNode) -> Option<String> {
    let node = &stored.node;
    let host = format_host(node);
    let name = surge_name(&stored.display_name);
    let mut fields = vec![
        name,
        node.protocol.as_str().to_string(),
        host,
        node.port.to_string(),
    ];
    match node.protocol {
        NodeProtocol::SS => {
            fields.push(format!(
                "encrypt-method={}",
                node.encryption.as_deref().unwrap_or("aes-128-gcm")
            ));
            fields.push(format!(
                "password={}",
                node.password.as_deref().unwrap_or_default()
            ));
            if let Some(plugin) = node.plugin.as_deref() {
                fields.push(format!("obfs={plugin}"));
            }
            if let Some(options) = node.plugin_opts.as_deref() {
                fields.push(format!("obfs-host={options}"));
            }
        }
        NodeProtocol::Trojan => {
            fields.push(format!(
                "password={}",
                node.password.as_deref().unwrap_or_default()
            ));
            if let Some(sni) = node.sni.as_deref() {
                fields.push(format!("sni={sni}"));
            }
            if node.skip_cert_verify {
                fields.push("skip-cert-verify=true".into());
            }
        }
        NodeProtocol::VMess | NodeProtocol::VLess => {
            fields.push(format!(
                "username={}",
                node.password.as_deref().unwrap_or_default()
            ));
            fields.push(format!("tls={}", node.tls));
            if node.transport == "ws" {
                fields.push("ws=true".into());
                if let Some(path) = node.ws_path.as_deref() {
                    fields.push(format!("ws-path={path}"));
                }
            }
            if let Some(sni) = node.sni.as_deref() {
                fields.push(format!("sni={sni}"));
            }
        }
        NodeProtocol::Socks5 => {
            if let Some(username) = node.username.as_deref() {
                fields.push(format!("username={username}"));
            }
            if let Some(password) = node.password.as_deref() {
                fields.push(format!("password={password}"));
            }
        }
        NodeProtocol::Hysteria2 => {
            fields.push(format!(
                "password={}",
                node.hy2_auth
                    .as_deref()
                    .or(node.password.as_deref())
                    .unwrap_or_default()
            ));
            if let Some(sni) = node.sni.as_deref() {
                fields.push(format!("sni={sni}"));
            }
        }
        NodeProtocol::Tuic => {
            fields.push(format!(
                "username={}",
                node.tuic_uuid
                    .as_deref()
                    .or(node.username.as_deref())
                    .unwrap_or_default()
            ));
            fields.push(format!(
                "password={}",
                node.tuic_password
                    .as_deref()
                    .or(node.password.as_deref())
                    .unwrap_or_default()
            ));
        }
        NodeProtocol::Juicity
        | NodeProtocol::AnyTLS
        | NodeProtocol::Direct
        | NodeProtocol::Block => return None,
    }
    Some(fields.join(", "))
}

fn surge_name(value: &str) -> String {
    value.replace([',', '\n', '\r'], " ")
}

fn clash_config(nodes: &[StoredNode]) -> Value {
    let proxies = nodes
        .iter()
        .map(|stored| clash_node(&stored.node))
        .collect::<Vec<_>>();
    let names = nodes
        .iter()
        .map(|stored| stored.display_name.clone())
        .collect::<Vec<_>>();
    json!({
        "proxies": proxies,
        "proxy-groups": [{
            "name": "AUTO",
            "type": "url-test",
            "proxies": names,
            "url": "https://www.gstatic.com/generate_204",
            "interval": 300
        }],
        "rules": ["MATCH,AUTO"]
    })
}

fn clash_node(node: &Node) -> Value {
    let mut value = Map::new();
    value.insert("name".into(), Value::String(node.name.clone()));
    value.insert("server".into(), Value::String(node.host().to_string()));
    value.insert("port".into(), Value::from(node.port));
    match node.protocol {
        NodeProtocol::SS => {
            value.insert("type".into(), Value::String("ss".into()));
            insert_string(&mut value, "cipher", node.encryption.as_deref());
            insert_string(&mut value, "password", node.password.as_deref());
            insert_string(&mut value, "plugin", node.plugin.as_deref());
            insert_string(&mut value, "plugin-opts", node.plugin_opts.as_deref());
        }
        NodeProtocol::Trojan => {
            value.insert("type".into(), Value::String("trojan".into()));
            insert_string(&mut value, "password", node.password.as_deref());
            tls_fields(&mut value, node);
            transport_fields(&mut value, node);
        }
        NodeProtocol::VMess => {
            value.insert("type".into(), Value::String("vmess".into()));
            insert_string(&mut value, "uuid", node.password.as_deref());
            insert_string(&mut value, "cipher", node.encryption.as_deref());
            insert_string(&mut value, "flow", node.flow.as_deref());
            tls_fields(&mut value, node);
            transport_fields(&mut value, node);
        }
        NodeProtocol::VLess => {
            value.insert("type".into(), Value::String("vless".into()));
            insert_string(&mut value, "uuid", node.password.as_deref());
            insert_string(&mut value, "encryption", node.encryption.as_deref());
            insert_string(&mut value, "flow", node.flow.as_deref());
            tls_fields(&mut value, node);
            transport_fields(&mut value, node);
            clash_vless_mode_fields(&mut value, node);
        }
        NodeProtocol::Socks5 => {
            value.insert("type".into(), Value::String("socks5".into()));
            insert_string(&mut value, "username", node.username.as_deref());
            insert_string(&mut value, "password", node.password.as_deref());
        }
        NodeProtocol::Hysteria2 => {
            value.insert("type".into(), Value::String("hysteria2".into()));
            insert_string(
                &mut value,
                "password",
                node.hy2_auth.as_deref().or(node.password.as_deref()),
            );
            tls_fields(&mut value, node);
            insert_string(&mut value, "obfs", node.hy2_obfs.as_deref());
        }
        NodeProtocol::Tuic => {
            value.insert("type".into(), Value::String("tuic".into()));
            insert_string(
                &mut value,
                "uuid",
                node.tuic_uuid.as_deref().or(node.username.as_deref()),
            );
            insert_string(
                &mut value,
                "password",
                node.tuic_password.as_deref().or(node.password.as_deref()),
            );
            insert_string(
                &mut value,
                "congestion-controller",
                node.tuic_congestion.as_deref(),
            );
            insert_string(&mut value, "alpn", node.tuic_alpn.as_deref());
            tls_fields(&mut value, node);
        }
        NodeProtocol::Juicity => {
            value.insert("type".into(), Value::String("juicity".into()));
            insert_string(
                &mut value,
                "uuid",
                node.juicity_uuid.as_deref().or(node.username.as_deref()),
            );
            insert_string(
                &mut value,
                "password",
                node.juicity_password
                    .as_deref()
                    .or(node.password.as_deref()),
            );
            tls_fields(&mut value, node);
        }
        NodeProtocol::AnyTLS => {
            value.insert("type".into(), Value::String("anytls".into()));
            insert_string(
                &mut value,
                "password",
                node.anytls_password.as_deref().or(node.password.as_deref()),
            );
            tls_fields(&mut value, node);
        }
        NodeProtocol::Direct | NodeProtocol::Block => {
            value.clear();
            value.insert("name".into(), Value::String(node.name.clone()));
            value.insert(
                "type".into(),
                Value::String(node.protocol.as_str().to_string()),
            );
        }
    }
    Value::Object(value)
}

fn sing_box_config(nodes: &[StoredNode]) -> Value {
    let outbounds = nodes
        .iter()
        .map(|stored| sing_box_node(&stored.node))
        .chain(std::iter::once(json!({
            "type": "selector",
            "tag": "auto",
            "outbounds": nodes.iter().map(|stored| stored.display_name.clone()).collect::<Vec<_>>()
        })))
        .collect::<Vec<_>>();
    json!({
        "log": { "level": "info" },
        "outbounds": outbounds,
        "route": { "final": "auto" }
    })
}

fn sing_box_node(node: &Node) -> Value {
    let mut value = Map::new();
    value.insert("tag".into(), Value::String(node.name.clone()));
    value.insert("server".into(), Value::String(node.host().to_string()));
    value.insert("server_port".into(), Value::from(node.port));
    let kind = match node.protocol {
        NodeProtocol::SS => "shadowsocks",
        NodeProtocol::Trojan => "trojan",
        NodeProtocol::VMess => "vmess",
        NodeProtocol::VLess => "vless",
        NodeProtocol::Socks5 => "socks",
        NodeProtocol::Hysteria2 => "hysteria2",
        NodeProtocol::Tuic => "tuic",
        NodeProtocol::Juicity => "juicity",
        NodeProtocol::AnyTLS => "anytls",
        NodeProtocol::Direct => "direct",
        NodeProtocol::Block => "block",
    };
    value.insert("type".into(), Value::String(kind.into()));
    match node.protocol {
        NodeProtocol::SS => {
            insert_string(&mut value, "method", node.encryption.as_deref());
            insert_string(&mut value, "password", node.password.as_deref());
        }
        NodeProtocol::Trojan => {
            insert_string(&mut value, "password", node.password.as_deref());
            tls_fields_sing_box(&mut value, node);
            transport_fields_sing_box(&mut value, node);
        }
        NodeProtocol::VMess => {
            insert_string(&mut value, "uuid", node.password.as_deref());
            insert_string(&mut value, "security", node.encryption.as_deref());
            insert_string(&mut value, "flow", node.flow.as_deref());
            tls_fields_sing_box(&mut value, node);
            transport_fields_sing_box(&mut value, node);
        }
        NodeProtocol::VLess => {
            insert_string(&mut value, "uuid", node.password.as_deref());
            insert_string(
                &mut value,
                "encryption",
                node.encryption.as_deref().filter(|value| *value != "none"),
            );
            insert_string(&mut value, "flow", node.flow.as_deref());
            sing_box_vless_mode_fields(&mut value, node);
            tls_fields_sing_box(&mut value, node);
            transport_fields_sing_box(&mut value, node);
        }
        NodeProtocol::Socks5 => {
            insert_string(&mut value, "username", node.username.as_deref());
            insert_string(&mut value, "password", node.password.as_deref());
        }
        NodeProtocol::Hysteria2 => {
            insert_string(
                &mut value,
                "password",
                node.hy2_auth.as_deref().or(node.password.as_deref()),
            );
            tls_fields_sing_box(&mut value, node);
            insert_string(&mut value, "obfs", node.hy2_obfs.as_deref());
        }
        NodeProtocol::Tuic => {
            insert_string(
                &mut value,
                "uuid",
                node.tuic_uuid.as_deref().or(node.username.as_deref()),
            );
            insert_string(
                &mut value,
                "password",
                node.tuic_password.as_deref().or(node.password.as_deref()),
            );
            insert_string(
                &mut value,
                "congestion_control",
                node.tuic_congestion.as_deref(),
            );
            tls_fields_sing_box(&mut value, node);
        }
        NodeProtocol::Juicity => {
            insert_string(
                &mut value,
                "uuid",
                node.juicity_uuid.as_deref().or(node.username.as_deref()),
            );
            insert_string(
                &mut value,
                "password",
                node.juicity_password
                    .as_deref()
                    .or(node.password.as_deref()),
            );
            tls_fields_sing_box(&mut value, node);
        }
        NodeProtocol::AnyTLS => {
            insert_string(
                &mut value,
                "password",
                node.anytls_password.as_deref().or(node.password.as_deref()),
            );
            tls_fields_sing_box(&mut value, node);
        }
        NodeProtocol::Direct | NodeProtocol::Block => {
            value.remove("server");
            value.remove("server_port");
        }
    }
    Value::Object(value)
}

fn dae_config(nodes: &[StoredNode]) -> String {
    let mut output = String::from("global {\n    dial_mode = \"domain\"\n}\n\nnode {\n");
    for stored in nodes {
        let link = share_link(&stored.node);
        let _ = writeln!(
            output,
            "    {}: '{}'",
            escape_key(&stored.display_name),
            escape_dae(&link)
        );
    }
    output.push_str("}\n\nrouting {\n    fallback = 'direct'\n}\n");
    output
}

fn share_link_config(nodes: &[StoredNode]) -> String {
    nodes
        .iter()
        .filter(|stored| {
            !matches!(
                stored.node.protocol,
                NodeProtocol::Direct | NodeProtocol::Block
            )
        })
        .map(|stored| share_link(&stored.node))
        .collect::<Vec<_>>()
        .join("\n")
}

fn share_link(node: &Node) -> String {
    let scheme = node.protocol.as_str();
    let host = format_host(node);
    let (user, password) = match node.protocol {
        NodeProtocol::SS => (
            node.encryption.as_deref().unwrap_or("aes-128-gcm"),
            node.password.as_deref().unwrap_or_default(),
        ),
        NodeProtocol::Hysteria2 => (
            "",
            node.hy2_auth
                .as_deref()
                .or(node.password.as_deref())
                .unwrap_or_default(),
        ),
        NodeProtocol::Tuic => (
            node.tuic_uuid
                .as_deref()
                .or(node.username.as_deref())
                .unwrap_or_default(),
            node.tuic_password
                .as_deref()
                .or(node.password.as_deref())
                .unwrap_or_default(),
        ),
        NodeProtocol::Juicity => (
            node.juicity_uuid
                .as_deref()
                .or(node.username.as_deref())
                .unwrap_or_default(),
            node.juicity_password
                .as_deref()
                .or(node.password.as_deref())
                .unwrap_or_default(),
        ),
        NodeProtocol::AnyTLS => (
            "",
            node.anytls_password
                .as_deref()
                .or(node.password.as_deref())
                .unwrap_or_default(),
        ),
        _ => ("", node.password.as_deref().unwrap_or_default()),
    };
    let auth = if user.is_empty() {
        percent_encode(password)
    } else {
        format!("{}:{}", percent_encode(user), percent_encode(password))
    };
    let mut link = format!("{scheme}://{auth}@{host}:{}", node.port);
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    if node.transport != "tcp" && !node.transport.is_empty() {
        query.append_pair("type", &node.transport);
    }
    if let Some(sni) = node.sni.as_deref() {
        query.append_pair("sni", sni);
    }
    if node.skip_cert_verify {
        query.append_pair("insecure", "1");
    }
    if node.protocol == NodeProtocol::AnyTLS {
        let timeout = node.anytls_idle_session_timeout.unwrap_or(30);
        query.append_pair("idle_session_timeout", &format!("{timeout}s"));
        if let Some(interval) = node.anytls_idle_session_check_interval {
            query.append_pair("idle_session_check_interval", &format!("{interval}s"));
        }
        if let Some(min_idle) = node.anytls_min_idle_session {
            query.append_pair("min_idle_session", &min_idle.to_string());
        }
    }
    if let Some(path) = node.ws_path.as_deref() {
        query.append_pair("path", path);
    }
    if let Some(host) = node.ws_host.as_deref() {
        query.append_pair("host", host);
    }
    if let Some(service) = node.grpc_service.as_deref() {
        query.append_pair("serviceName", service);
    }
    if let Some(flow) = node.flow.as_deref() {
        query.append_pair("flow", flow);
    }
    if let Some(pbk) = node
        .reality_public_key
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        query.append_pair("security", "reality");
        query.append_pair("pbk", pbk);
        if let Some(sid) = node.reality_short_id.as_deref() {
            query.append_pair("sid", sid);
        }
        if let Some(spider_x) = node.reality_spider_x.as_deref() {
            query.append_pair("spx", spider_x);
        }
    } else if node.protocol == NodeProtocol::VLess {
        query.append_pair("security", if node.tls { "tls" } else { "none" });
    }
    if node.protocol == NodeProtocol::VLess && node.vless_mode != WireMode::Legacy {
        query.append_pair("vless_mode", node.vless_mode.as_str());
    }
    let query = query.finish();
    if !query.is_empty() {
        link.push('?');
        link.push_str(&query);
    }
    if !node.name.is_empty() {
        link.push('#');
        link.push_str(&percent_encode(&node.name));
    }
    link
}

fn format_host(node: &Node) -> String {
    if node.host().contains(':') && !node.host().starts_with('[') {
        format!("[{}]", node.host())
    } else {
        node.host().to_string()
    }
}

fn tls_fields(value: &mut Map<String, Value>, node: &Node) {
    value.insert("tls".into(), Value::from(node.tls || reality_enabled(node)));
    let server_name = node
        .sni
        .as_deref()
        .or_else(|| reality_enabled(node).then_some(node.host()));
    insert_string(value, "servername", server_name);
    if node.skip_cert_verify {
        value.insert("skip-cert-verify".into(), Value::Bool(true));
    }
    if reality_enabled(node) {
        let mut reality = Map::new();
        insert_string(
            &mut reality,
            "public-key",
            node.reality_public_key.as_deref(),
        );
        insert_string(&mut reality, "short-id", node.reality_short_id.as_deref());
        insert_string(&mut reality, "spider-x", node.reality_spider_x.as_deref());
        value.insert("reality-opts".into(), Value::Object(reality));
    }
}

fn reality_enabled(node: &Node) -> bool {
    node.protocol == NodeProtocol::VLess
        && node
            .reality_public_key
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
}

fn clash_vless_mode_fields(value: &mut Map<String, Value>, node: &Node) {
    match node.vless_mode {
        // ponytail: MuxCool has no Clash wire representation; DAE/share links keep it intact.
        WireMode::Legacy | WireMode::MuxCool => {}
        WireMode::Xudp => {
            value.insert("udp".into(), Value::Bool(true));
            value.insert("packet-encoding".into(), Value::String("xudp".into()));
        }
        WireMode::UotV2 => {
            value.insert("udp".into(), Value::Bool(true));
            value.insert(
                "udp-over-tcp".into(),
                json!({"enabled": true, "version": 2}),
            );
        }
        WireMode::H2mux | WireMode::H2muxPadded => {
            value.insert("udp".into(), Value::Bool(true));
            value.insert(
                "multiplex".into(),
                json!({
                    "enabled": true,
                    "protocol": "h2mux",
                    "padding": node.vless_mode == WireMode::H2muxPadded
                }),
            );
        }
    }
}

fn transport_fields(value: &mut Map<String, Value>, node: &Node) {
    if node.transport != "tcp" && !node.transport.is_empty() {
        value.insert("network".into(), Value::String(node.transport.clone()));
    }
    if node.transport == "ws" {
        let mut ws = Map::new();
        insert_string(&mut ws, "path", node.ws_path.as_deref());
        if let Some(host) = node.ws_host.as_deref() {
            ws.insert("headers".into(), json!({"Host": host}));
        }
        value.insert("ws-opts".into(), Value::Object(ws));
    }
    if node.transport == "grpc" {
        let mut grpc = Map::new();
        insert_string(&mut grpc, "grpc-service-name", node.grpc_service.as_deref());
        value.insert("grpc-opts".into(), Value::Object(grpc));
    }
}

fn tls_fields_sing_box(value: &mut Map<String, Value>, node: &Node) {
    if !node.tls && node.sni.is_none() && !node.skip_cert_verify && !reality_enabled(node) {
        return;
    }
    let mut tls = Map::new();
    tls.insert(
        "enabled".into(),
        Value::from(node.tls || reality_enabled(node)),
    );
    let server_name = node
        .sni
        .as_deref()
        .or_else(|| reality_enabled(node).then_some(node.host()));
    insert_string(&mut tls, "server_name", server_name);
    if node.skip_cert_verify && !reality_enabled(node) {
        tls.insert("insecure".into(), Value::Bool(true));
    }
    if reality_enabled(node) {
        let mut reality = Map::new();
        reality.insert("enabled".into(), Value::Bool(true));
        insert_string(
            &mut reality,
            "public_key",
            node.reality_public_key.as_deref(),
        );
        insert_string(&mut reality, "short_id", node.reality_short_id.as_deref());
        tls.insert("reality".into(), Value::Object(reality));
    }
    value.insert("tls".into(), Value::Object(tls));
}

fn sing_box_vless_mode_fields(value: &mut Map<String, Value>, node: &Node) {
    match node.vless_mode {
        WireMode::Legacy => {
            value.insert("packet_encoding".into(), Value::String(String::new()));
        }
        WireMode::Xudp => {
            value.insert("packet_encoding".into(), Value::String("xudp".into()));
        }
        WireMode::H2mux | WireMode::H2muxPadded => {
            value.insert(
                "multiplex".into(),
                json!({
                    "enabled": true,
                    "protocol": "h2mux",
                    "padding": node.vless_mode == WireMode::H2muxPadded
                }),
            );
        }
        // ponytail: UoT and MuxCool have no VLESS outbound fields in sing-box;
        // DAE/share links remain the lossless export for those modes.
        WireMode::UotV2 | WireMode::MuxCool => {}
    }
}

fn transport_fields_sing_box(value: &mut Map<String, Value>, node: &Node) {
    let mut transport = Map::new();
    match node.transport.as_str() {
        "ws" => {
            transport.insert("type".into(), Value::String("ws".into()));
            insert_string(&mut transport, "path", node.ws_path.as_deref());
            if let Some(host) = node.ws_host.as_deref() {
                transport.insert("headers".into(), json!({"Host": host}));
            }
        }
        "grpc" => {
            transport.insert("type".into(), Value::String("grpc".into()));
            insert_string(&mut transport, "service_name", node.grpc_service.as_deref());
        }
        _ => return,
    }
    value.insert("transport".into(), Value::Object(transport));
}

fn insert_string(map: &mut Map<String, Value>, key: &str, value: Option<&str>) {
    if let Some(value) = value.filter(|value| !value.is_empty()) {
        map.insert(key.into(), Value::String(value.into()));
    }
}

fn escape_key(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' || character == '-' {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn escape_dae(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
}

fn percent_encode(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use honk_config::{node::Node, types::NodeProtocol};

    use super::{ExportFormat, ExportLimits, render, select_nodes};
    use crate::models::{ProbeRecord, StoredNode};

    fn nodes() -> Vec<StoredNode> {
        let node = Node::from_share_link("trojan://secret@example.com:443#edge")
            .expect("fixture share link is valid");
        vec![StoredNode {
            node,
            display_name: "edge".into(),
            region: None,
            sources: vec!["manual".into()],
            probes: Vec::new(),
        }]
    }

    #[test]
    fn format_defaults_to_clash() {
        assert_eq!(ExportFormat::parse(None), Ok(ExportFormat::Clash));
    }

    #[test]
    fn client_formats_parse_and_render() {
        assert_eq!(
            ExportFormat::parse(Some("SIP002")),
            Ok(ExportFormat::Sip002)
        );
        assert_eq!(
            ExportFormat::parse(Some("sip008")),
            Ok(ExportFormat::Sip008)
        );
        assert_eq!(
            ExportFormat::parse(Some("simple")),
            Ok(ExportFormat::ShareLinks)
        );
        assert_eq!(ExportFormat::parse(Some("surge")), Ok(ExportFormat::Surge));

        let nodes = nodes();
        assert!(
            render(ExportFormat::Sip002, &nodes)
                .expect("sip002 renders")
                .is_empty()
        );
        assert!(
            render(ExportFormat::Sip008, &nodes)
                .expect("sip008 renders")
                .contains("\"servers\"")
        );
        assert!(
            render(ExportFormat::ShareLinks, &nodes)
                .expect("share links render")
                .contains("trojan://")
        );
        assert!(
            render(ExportFormat::Surge, &nodes)
                .expect("surge renders")
                .contains("[Proxy]")
        );
    }

    #[test]
    fn each_format_has_its_wire_shape() {
        let nodes = nodes();
        let clash = render(ExportFormat::Clash, &nodes).expect("clash renders");
        let sing_box = render(ExportFormat::SingBox, &nodes).expect("sing-box renders");
        let dae = render(ExportFormat::Dae, &nodes).expect("dae renders");
        assert!(clash.contains("proxy-groups:"));
        assert!(sing_box.contains("\"outbounds\""));
        assert!(dae.contains("node {") && dae.contains("trojan://"));
        honk_config::parser::parse_dae_config(&dae).expect("dae output parses");
    }

    #[test]
    fn limits_choose_best_nodes_before_group_caps() {
        let selected = select_nodes(
            vec![
                ranked("slow-hk", "hk", "alpha", 300, true),
                ranked("fast-hk", "hk", "alpha", 50, true),
                ranked("fast-us", "us", "beta", 20, true),
                ranked("failed-us", "us", "beta", 1, false),
            ],
            ExportLimits {
                total: Some(2),
                per_region: Some(1),
                per_provider: Some(1),
                providers: Vec::new(),
                exclude_dead: false,
            },
        );
        assert_eq!(
            selected
                .iter()
                .map(|node| node.display_name.as_str())
                .collect::<Vec<_>>(),
            ["fast-us", "fast-hk"]
        );
    }

    #[test]
    fn dead_nodes_are_omitted_when_export_filter_enabled() {
        let mut untested = nodes();
        untested[0].display_name = "untested".into();
        let selected = select_nodes(
            vec![
                ranked("dead", "hk", "alpha", 1, false),
                ranked("alive", "hk", "alpha", 50, true),
                untested.remove(0),
            ],
            ExportLimits {
                exclude_dead: true,
                ..Default::default()
            },
        );
        assert_eq!(
            selected
                .iter()
                .map(|node| node.display_name.as_str())
                .collect::<Vec<_>>(),
            ["alive", "untested"]
        );
    }

    #[test]
    fn provider_filter_keeps_only_selected_sources() {
        let selected = select_nodes(
            vec![
                ranked("alpha-node", "hk", "alpha", 50, true),
                ranked("beta-node", "us", "beta", 20, true),
            ],
            ExportLimits {
                providers: vec!["ALPHA".into()],
                ..Default::default()
            },
        );
        assert_eq!(
            selected
                .iter()
                .map(|node| node.display_name.as_str())
                .collect::<Vec<_>>(),
            ["alpha-node"]
        );
    }

    #[test]
    fn vless_reality_fields_survive_client_exports() {
        let node = Node::from_share_link(
            "vless://b831381d-6324-4d53-ad4f-8cda48b30811@reality.example.com:443?security=reality&pbk=jHkr1EmJCyQxjU0HXJlNblVdXB4Z7yODHJhgJ5lqmzc&sid=a1b2c3d4e5f60718&spx=%2Freality&sni=mask.example&flow=xtls-rprx-vision&vless_mode=xudp#reality-node",
        )
        .expect("fixture share link is valid");
        let nodes = vec![StoredNode {
            node,
            display_name: "reality-node".into(),
            region: Some("hk".into()),
            sources: vec!["yss".into()],
            probes: Vec::new(),
        }];

        let clash: serde_yaml::Value =
            serde_yaml::from_str(&render(ExportFormat::Clash, &nodes).expect("clash renders"))
                .expect("clash output parses");
        let proxy = &clash["proxies"][0];
        assert_eq!(proxy["type"], "vless");
        assert_eq!(proxy["flow"], "xtls-rprx-vision");
        assert_eq!(proxy["servername"], "mask.example");
        assert_eq!(
            proxy["reality-opts"]["public-key"],
            "jHkr1EmJCyQxjU0HXJlNblVdXB4Z7yODHJhgJ5lqmzc"
        );
        assert_eq!(proxy["reality-opts"]["short-id"], "a1b2c3d4e5f60718");
        assert_eq!(proxy["reality-opts"]["spider-x"], "/reality");
        assert_eq!(proxy["packet-encoding"], "xudp");

        let sing_box: serde_json::Value =
            serde_json::from_str(&render(ExportFormat::SingBox, &nodes).expect("sing-box renders"))
                .expect("sing-box output parses");
        let outbound = &sing_box["outbounds"][0];
        assert!(outbound.get("security").is_none());
        assert_eq!(outbound["packet_encoding"], "xudp");
        assert_eq!(outbound["tls"]["reality"]["enabled"], true);
        assert_eq!(
            outbound["tls"]["reality"]["public_key"],
            "jHkr1EmJCyQxjU0HXJlNblVdXB4Z7yODHJhgJ5lqmzc"
        );
        assert_eq!(outbound["tls"]["reality"]["short_id"], "a1b2c3d4e5f60718");

        let dae = render(ExportFormat::Dae, &nodes).expect("dae renders");
        let share_links = render(ExportFormat::ShareLinks, &nodes).expect("share links render");
        let parsed_share_link =
            Node::from_share_link(share_links.trim()).expect("share-link output parses");
        assert!(dae.contains("security=reality"));
        assert!(dae.contains("spx=%2Freality"));
        assert!(dae.contains("vless_mode=xudp"));
        let parsed = honk_config::parser::parse_dae_config(&dae).expect("dae output parses");
        assert_eq!(
            parsed.nodes[0].reality_public_key,
            nodes[0].node.reality_public_key
        );
        assert_eq!(
            parsed.nodes[0].reality_spider_x,
            nodes[0].node.reality_spider_x
        );
        assert_eq!(parsed.nodes[0].vless_mode, nodes[0].node.vless_mode);
        assert_eq!(parsed_share_link.protocol, NodeProtocol::VLess);
        assert_eq!(
            parsed_share_link.reality_public_key,
            nodes[0].node.reality_public_key
        );
        assert_eq!(parsed_share_link.vless_mode, nodes[0].node.vless_mode);
    }

    #[test]
    fn vless_plain_link_keeps_tls_disabled() {
        let node = Node::from_share_link(
            "vless://11111111-1111-4111-8111-111111111111@example.com:80?security=none#plain",
        )
        .expect("fixture share link is valid");
        let output = render(
            ExportFormat::Dae,
            &[StoredNode {
                node,
                display_name: "plain".into(),
                region: None,
                sources: Vec::new(),
                probes: Vec::new(),
            }],
        )
        .expect("dae renders");
        assert!(output.contains("security=none"));
    }

    #[test]
    fn anytls_share_link_keeps_honk_pool_defaults() {
        let mut node = Node::from_share_link(
            "anytls://22222222-2222-4222-8222-222222222222@192.0.2.10:443?sni=anytls.example.com#fixture.fr.17",
        )
        .expect("fixture share link is valid");
        node.password = None;
        node.anytls_idle_session_timeout = Some(30);
        let output = render(
            ExportFormat::ShareLinks,
            &[StoredNode {
                node,
                display_name: "fixture.fr.17".into(),
                region: Some("fr".into()),
                sources: vec!["kad".into()],
                probes: Vec::new(),
            }],
        )
        .expect("share links render");

        assert!(output.contains("idle_session_timeout=30s"));
        let parsed = Node::from_share_link(output.trim()).expect("output parses");
        assert_eq!(
            parsed.anytls_password.as_deref(),
            Some("22222222-2222-4222-8222-222222222222")
        );
        assert_eq!(parsed.anytls_idle_session_timeout, Some(30));
    }

    fn ranked(name: &str, region: &str, provider: &str, latency: i64, success: bool) -> StoredNode {
        let mut node = Node::from_share_link("trojan://secret@example.com:443")
            .expect("fixture share link is valid");
        node.name = name.into();
        node.id = node.derive_id();
        StoredNode {
            node,
            display_name: name.into(),
            region: Some(region.into()),
            sources: vec![provider.into()],
            probes: vec![ProbeRecord {
                kind: "latency".into(),
                success,
                latency_ms: success.then_some(latency),
                status_code: None,
                error: None,
                probed_at: Utc::now(),
            }],
        }
    }
}
