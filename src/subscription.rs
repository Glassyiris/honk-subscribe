use std::collections::HashMap;

use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use chrono::{DateTime, Utc};
use honk_config::{
    node::Node,
    subscription::Subscription,
    types::{NodeProtocol, SubscriptionType},
};
use serde::Deserialize;

use crate::models::SubscriptionTraffic;

pub(crate) const UPSTREAM_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/151.0.0.0 Safari/537.36";

pub struct SubscriptionFetcher {
    client: reqwest::Client,
}

pub struct FetchResult {
    pub nodes: Vec<Node>,
    pub traffic: Option<SubscriptionTraffic>,
}

pub fn parse_share_link(link: &str) -> Result<Node, honk_config::ConfigError> {
    let normalized = normalize_vless_packet_encoding(link);
    Node::from_share_link(&normalized)
}

fn normalize_vless_packet_encoding(link: &str) -> String {
    let Ok(mut url) = url::Url::parse(link) else {
        return link.to_owned();
    };
    let is_vless = url.scheme() == "vless";
    let is_anytls = url.scheme() == "anytls";
    if !is_vless && !is_anytls {
        return link.to_owned();
    }
    let pairs = url
        .query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    if !pairs
        .iter()
        .any(|(key, value)| key == "packetEncoding" && (is_anytls || value == "none"))
    {
        return link.to_owned();
    }
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in pairs {
        if key == "packetEncoding" && (is_anytls || value == "none") {
            continue;
        }
        query.append_pair(&key, &value);
    }
    let query = query.finish();
    url.set_query((!query.is_empty()).then_some(query.as_str()));
    url.to_string()
}

impl SubscriptionFetcher {
    pub fn new() -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()?;
        Ok(Self { client })
    }

    pub async fn fetch(&self, subscription: &Subscription) -> anyhow::Result<FetchResult> {
        let mut request = self.client.get(&subscription.url);
        for header in &subscription.headers {
            if header.key.eq_ignore_ascii_case("user-agent") {
                continue;
            }
            request = request.header(&header.key, &header.value);
        }
        let request = request.header("user-agent", UPSTREAM_USER_AGENT);
        let response = request.send().await?.error_for_status()?;
        let traffic = response
            .headers()
            .get("subscription-userinfo")
            .and_then(|value| value.to_str().ok())
            .and_then(parse_subscription_userinfo);
        let content = response.text().await?;
        Ok(FetchResult {
            nodes: parse_content(subscription, &content)?,
            traffic,
        })
    }
}

fn parse_subscription_userinfo(value: &str) -> Option<SubscriptionTraffic> {
    let mut upload = None;
    let mut download = None;
    let mut total = None;
    let mut expire = None;
    for part in value.split(';') {
        let (key, raw_value) = part.trim().split_once('=')?;
        let parsed = raw_value.trim().parse::<u64>().ok()?;
        match key {
            "upload" => upload = Some(parsed),
            "download" => download = Some(parsed),
            "total" => total = Some(parsed),
            "expire" => {
                expire = i64::try_from(parsed)
                    .ok()
                    .and_then(|seconds| DateTime::<Utc>::from_timestamp(seconds, 0));
            }
            _ => {}
        }
    }
    (upload.is_some() || download.is_some() || total.is_some() || expire.is_some()).then_some(
        SubscriptionTraffic {
            upload: upload.unwrap_or_default(),
            download: download.unwrap_or_default(),
            total,
            expire,
        },
    )
}

pub fn parse_content(subscription: &Subscription, content: &str) -> anyhow::Result<Vec<Node>> {
    match subscription.sub_type {
        SubscriptionType::Clash => parse_clash(subscription.id, content),
        SubscriptionType::Simple | SubscriptionType::Sip008 => {
            parse_links(subscription.id, content)
        }
        SubscriptionType::Custom => {
            parse_clash(subscription.id, content).or_else(|_| parse_links(subscription.id, content))
        }
    }
}

fn parse_links(subscription_id: uuid::Uuid, content: &str) -> anyhow::Result<Vec<Node>> {
    let decoded = decode_base64(content).unwrap_or_else(|| content.trim().to_string());
    let nodes = decoded
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let mut node = parse_share_link(line).ok()?;
            node.subscription_id = Some(subscription_id);
            node.id = node.derive_id();
            Some(node)
        })
        .collect::<Vec<_>>();
    if nodes.is_empty() {
        anyhow::bail!("subscription contains no supported share links");
    }
    Ok(nodes)
}

fn decode_base64(content: &str) -> Option<String> {
    let compact = content.trim().replace(['\n', '\r', ' ', '\t'], "");
    let bytes = STANDARD
        .decode(&compact)
        .or_else(|_| URL_SAFE_NO_PAD.decode(&compact))
        .ok()?;
    String::from_utf8(bytes).ok()
}

#[derive(Debug, Deserialize)]
struct ClashDocument {
    proxies: Vec<ClashProxy>,
}

#[derive(Debug, Deserialize)]
struct ClashProxy {
    name: String,
    #[serde(rename = "type")]
    protocol: String,
    server: String,
    port: u16,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    uuid: Option<String>,
    #[serde(default)]
    cipher: Option<String>,
    #[serde(default)]
    tls: Option<bool>,
    #[serde(default)]
    sni: Option<String>,
    #[serde(default, rename = "servername")]
    server_name: Option<String>,
    #[serde(default, rename = "skip-cert-verify")]
    skip_cert_verify: bool,
    #[serde(default)]
    network: Option<String>,
    #[serde(default, rename = "ws-opts")]
    ws: Option<ClashWsOptions>,
    #[serde(default, rename = "grpc-opts")]
    grpc: Option<ClashGrpcOptions>,
}

#[derive(Debug, Deserialize)]
struct ClashWsOptions {
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    headers: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct ClashGrpcOptions {
    #[serde(default, rename = "grpc-service-name")]
    service_name: Option<String>,
}

fn parse_clash(subscription_id: uuid::Uuid, content: &str) -> anyhow::Result<Vec<Node>> {
    let document: ClashDocument = serde_yaml::from_str(content)?;
    let nodes = document
        .proxies
        .into_iter()
        .filter_map(|proxy| clash_node(subscription_id, proxy))
        .collect::<Vec<_>>();
    if nodes.is_empty() {
        anyhow::bail!("clash subscription contains no supported proxies");
    }
    Ok(nodes)
}

fn clash_node(subscription_id: uuid::Uuid, proxy: ClashProxy) -> Option<Node> {
    let protocol = proxy.protocol.parse::<NodeProtocol>().ok()?;
    let mut node = Node {
        id: uuid::Uuid::nil(),
        name: proxy.name,
        protocol,
        address: format!("{}:{}", proxy.server, proxy.port),
        host: proxy.server,
        port: proxy.port,
        username: proxy.username,
        password: proxy.password.or(proxy.uuid),
        encryption: proxy.cipher,
        tls: proxy.tls.unwrap_or(matches!(
            protocol,
            NodeProtocol::Trojan | NodeProtocol::VLess | NodeProtocol::AnyTLS
        )),
        sni: proxy.sni.or(proxy.server_name),
        skip_cert_verify: proxy.skip_cert_verify,
        network: proxy.network,
        subscription_id: Some(subscription_id),
        ..Default::default()
    };
    if let Some(ws) = proxy.ws {
        node.transport = "ws".into();
        node.ws_path = ws.path;
        node.ws_host = ws
            .headers
            .get("Host")
            .cloned()
            .or_else(|| ws.headers.get("host").cloned());
    }
    if let Some(grpc) = proxy.grpc {
        node.transport = "grpc".into();
        node.grpc_service = grpc.service_name;
    }
    node.id = node.derive_id();
    Some(node)
}

#[cfg(test)]
mod tests {

    use honk_config::types::NodeProtocol;

    use super::{UPSTREAM_USER_AGENT, parse_share_link, parse_subscription_userinfo};

    #[test]
    fn parses_subscription_traffic_header() {
        let traffic = parse_subscription_userinfo(
            "upload=1024; download=2048; total=10000; expire=4102444800",
        )
        .expect("traffic header");
        assert_eq!(traffic.used(), 3072);
        assert_eq!(traffic.remaining(), Some(6928));
        assert!(traffic.expire.is_some());
    }

    #[test]
    fn uses_the_requested_chrome_user_agent_for_upstream_requests() {
        assert_eq!(
            UPSTREAM_USER_AGENT,
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/151.0.0.0 Safari/537.36"
        );
    }

    #[test]
    fn parses_legacy_vless_packet_encoding_none() {
        let node = parse_share_link(
            "vless://11111111-1111-4111-8111-111111111111@vless.example.com:443?security=tls&type=ws&path=%2Ffixture%2Fdata%2Fjp&host=ws.example.com&packetEncoding=none&sni=ws.example.com&fp=safari#jp01",
        )
        .expect("legacy VLESS share link should parse");
        assert_eq!(node.protocol, NodeProtocol::VLess);
        assert_eq!(node.transport, "ws");
        assert!(node.tls);
    }

    #[test]
    fn parses_legacy_anytls_packet_encoding_none() {
        let node = parse_share_link(
            "anytls://secret@anytls.example.com:35355?security=tls&type=tcp&packetEncoding=none&alpn=h2&allowInsecure=1&sni=127.0.0.1&fp=chrome&udp=1&insecure=1&disable_sni=true&tfo=true&fast_open=true&reuse=true#fixture",
        )
        .expect("legacy AnyTLS share link should parse");
        assert_eq!(node.protocol, NodeProtocol::AnyTLS);
        assert!(node.tls);
    }
}
