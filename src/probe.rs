use std::sync::Arc;
use std::time::Duration;

#[cfg(feature = "honk-probe")]
use std::net::SocketAddr;
#[cfg(feature = "honk-probe")]
use std::time::Instant;

use chrono::Utc;
use honk_config::node::{Node, WireMode};
#[cfg(feature = "honk-probe")]
use honk_config::types::NodeProtocol;
use tokio::{sync::Semaphore, task::JoinSet, time::timeout};

#[cfg(feature = "honk-probe")]
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::models::{ProbeOutcome, ProbeRecord};
#[cfg(feature = "honk-probe")]
use crate::subscription::UPSTREAM_USER_AGENT;

pub const VLESS_MODE_PROBE_TIMEOUT: Duration = Duration::from_secs(4);

#[cfg_attr(not(feature = "honk-probe"), allow(dead_code))]
const VLESS_UDP_MODES: [WireMode; 5] = [
    WireMode::MuxCool,
    WireMode::H2mux,
    WireMode::H2muxPadded,
    WireMode::Xudp,
    WireMode::UotV2,
];

#[cfg_attr(not(feature = "honk-probe"), allow(dead_code))]
fn vless_udp_modes(node: &Node) -> [WireMode; 5] {
    if node.flow.as_deref() == Some("xtls-rprx-vision") {
        [
            WireMode::Xudp,
            WireMode::MuxCool,
            WireMode::H2mux,
            WireMode::H2muxPadded,
            WireMode::UotV2,
        ]
    } else {
        VLESS_UDP_MODES
    }
}

#[cfg(feature = "honk-probe")]
const VLESS_QUIC_TARGET_HOST: &str = "cp.cloudflare.com";
#[cfg(feature = "honk-probe")]
const VLESS_QUIC_TARGET_PORT: u16 = 443;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeKind {
    Latency,
    Ai,
    Netflix,
}

impl ProbeKind {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "latency" | "tcp" => Some(Self::Latency),
            "ai" => Some(Self::Ai),
            "netflix" => Some(Self::Netflix),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Latency => "latency",
            Self::Ai => "ai",
            Self::Netflix => "netflix",
        }
    }
    #[cfg(feature = "honk-probe")]
    fn url(self) -> &'static str {
        match self {
            Self::Latency => "https://www.gstatic.com/generate_204",
            Self::Ai => "https://chatgpt.com/",
            Self::Netflix => "https://www.netflix.com/title/80018499",
        }
    }
}

#[derive(Clone, Default)]
pub struct ProbeEngine;

#[derive(Debug)]
struct ProbeMeasurement {
    latency: Duration,
    status_code: Option<u16>,
}

impl ProbeEngine {
    pub async fn detect_vless_modes(
        &self,
        nodes: Vec<Node>,
        timeout_duration: Duration,
    ) -> Vec<Node> {
        #[cfg(feature = "honk-probe")]
        {
            let mut nodes = nodes;
            reset_vless_modes(&mut nodes);
            let permits = Arc::new(Semaphore::new(10));
            let mut tasks = JoinSet::new();
            for node in nodes
                .iter()
                .filter(|node| node.protocol == NodeProtocol::VLess)
                .cloned()
            {
                let permits = Arc::clone(&permits);
                tasks.spawn(async move {
                    let Ok(_permit) = permits.acquire_owned().await else {
                        return None;
                    };
                    detect_vless_mode(node, timeout_duration).await
                });
            }
            while let Some(result) = tasks.join_next().await {
                if let Ok(Some((id, mode))) = result {
                    if let Some(node) = nodes.iter_mut().find(|node| node.id == id) {
                        node.vless_mode = mode;
                        node.id = node.derive_id();
                    }
                }
            }
            return nodes;
        }
        #[cfg(not(feature = "honk-probe"))]
        {
            let _ = timeout_duration;
            nodes
        }
    }

    pub async fn many(
        &self,
        items: Vec<(Node, ProbeKind)>,
        timeout_duration: Duration,
    ) -> Vec<ProbeOutcome> {
        let permits = Arc::new(Semaphore::new(10));
        let mut tasks = JoinSet::new();
        for (node, kind) in items {
            let permits = Arc::clone(&permits);
            tasks.spawn(async move {
                let Ok(_permit) = permits.acquire_owned().await else {
                    return None;
                };
                Some(probe_one(&node, kind, timeout_duration).await)
            });
        }
        let mut outcomes = Vec::new();
        while let Some(result) = tasks.join_next().await {
            if let Ok(Some(outcome)) = result {
                outcomes.push(outcome);
            }
        }
        outcomes
    }
}

#[cfg(feature = "honk-probe")]
fn reset_vless_modes(nodes: &mut [Node]) {
    for node in nodes {
        if node.protocol == NodeProtocol::VLess {
            node.vless_mode = WireMode::Legacy;
            node.id = node.derive_id();
        }
    }
}

#[cfg(feature = "honk-probe")]
async fn detect_vless_mode(
    node: Node,
    timeout_duration: Duration,
) -> Option<(uuid::Uuid, WireMode)> {
    let registry = honk_outbound::ProxyRegistry::default_resolver().ok()?;
    let target = tokio::net::lookup_host((VLESS_QUIC_TARGET_HOST, VLESS_QUIC_TARGET_PORT))
        .await
        .ok()?
        .find(SocketAddr::is_ipv4)?;
    for mode in vless_udp_modes(&node) {
        let mut candidate = node.clone();
        candidate.vless_mode = mode;
        let Ok(Ok(())) = timeout(
            timeout_duration,
            probe_vless_quic(&registry, &candidate, target, timeout_duration),
        )
        .await
        else {
            continue;
        };
        return Some((node.id, mode));
    }
    None
}

#[cfg(feature = "honk-probe")]
async fn probe_vless_quic(
    registry: &honk_outbound::ProxyRegistry,
    node: &Node,
    target: SocketAddr,
    timeout_duration: Duration,
) -> anyhow::Result<()> {
    let transport = registry
        .dial_udp_transport(node, target, Some(VLESS_QUIC_TARGET_HOST), timeout_duration)
        .await?;
    let probe_node = Node {
        skip_cert_verify: true,
        sni: Some(VLESS_QUIC_TARGET_HOST.into()),
        ..Default::default()
    };
    let config = honk_outbound::quic::client_config(
        &probe_node,
        &[b"h3"],
        honk_outbound::quic::QuicClientOptions::default(),
    )
    .await?;
    honk_outbound::quic::quic_handshake_probe(
        transport,
        target,
        VLESS_QUIC_TARGET_HOST,
        &config,
        timeout_duration,
    )
    .await?;
    Ok(())
}

async fn probe_one(node: &Node, kind: ProbeKind, timeout_duration: Duration) -> ProbeOutcome {
    let result = timeout(timeout_duration, probe_inner(node, kind, timeout_duration)).await;
    match result {
        Ok(Ok(measurement)) => ProbeOutcome {
            node_id: node.id.to_string(),
            kind: kind.as_str().into(),
            success: true,
            latency_ms: duration_to_latency_ms(measurement.latency),
            status_code: measurement.status_code.map(i64::from),
            error: None,
            probed_at: Utc::now(),
        },
        Ok(Err(error)) => ProbeOutcome {
            node_id: node.id.to_string(),
            kind: kind.as_str().into(),
            success: false,
            latency_ms: None,
            status_code: None,
            error: Some(error),
            probed_at: Utc::now(),
        },
        Err(_) => ProbeOutcome {
            node_id: node.id.to_string(),
            kind: kind.as_str().into(),
            success: false,
            latency_ms: None,
            status_code: None,
            error: Some("probe timeout".into()),
            probed_at: Utc::now(),
        },
    }
}

fn duration_to_latency_ms(duration: Duration) -> Option<i64> {
    i64::try_from(duration.as_millis().max(1)).ok()
}

async fn probe_inner(
    node: &Node,
    kind: ProbeKind,
    timeout_duration: Duration,
) -> Result<ProbeMeasurement, String> {
    #[cfg(feature = "honk-probe")]
    {
        return match kind {
            ProbeKind::Latency => probe_latency_through_honk(node, kind, timeout_duration).await,
            ProbeKind::Ai | ProbeKind::Netflix => {
                probe_http_through_honk(node, kind, timeout_duration).await
            }
        };
    }
    #[cfg(not(feature = "honk-probe"))]
    {
        let _ = (node, kind, timeout_duration);
        Err("enable the honk-probe feature for proxy-routed probes; the default build does not report TCP reachability as latency".into())
    }
}

#[cfg(feature = "honk-probe")]
async fn probe_latency_through_honk(
    node: &Node,
    kind: ProbeKind,
    timeout_duration: Duration,
) -> Result<ProbeMeasurement, String> {
    let registry =
        honk_outbound::ProxyRegistry::default_resolver().map_err(|error| error.to_string())?;
    let entry = registry
        .find(node.protocol)
        .ok_or_else(|| format!("unsupported protocol {}", node.protocol.as_str()))?;
    let guard = honk_outbound::runtime::NodeRuntime::ephemeral_guarded(node);
    let runtime = guard.runtime();
    let result = honk_outbound::urltest::urltest_node(
        &runtime,
        entry.tcp.as_ref(),
        kind.url(),
        timeout_duration,
    )
    .await
    .map(|latency| ProbeMeasurement {
        latency,
        status_code: None,
    })
    .map_err(|error| error.to_string());
    guard.close().await;
    result
}

#[cfg(feature = "honk-probe")]
async fn probe_http_through_honk(
    node: &Node,
    kind: ProbeKind,
    timeout_duration: Duration,
) -> Result<ProbeMeasurement, String> {
    let registry =
        honk_outbound::ProxyRegistry::default_resolver().map_err(|error| error.to_string())?;
    let entry = registry
        .find(node.protocol)
        .ok_or_else(|| format!("unsupported protocol {}", node.protocol.as_str()))?;
    let guard = honk_outbound::runtime::NodeRuntime::ephemeral_guarded(node);
    let runtime = guard.runtime();
    let target = url::Url::parse(kind.url()).map_err(|error| error.to_string())?;
    let host = target
        .host_str()
        .ok_or_else(|| "probe URL has no host".to_string())?;
    let port = target
        .port_or_known_default()
        .ok_or_else(|| "probe URL has no port".to_string())?;
    let result = async {
        let address = tokio::net::lookup_host((host, port))
            .await
            .map_err(|error| error.to_string())?
            .next()
            .ok_or_else(|| "probe target did not resolve".to_string())?;
        let started = Instant::now();
        let proxy = entry
            .tcp
            .dial_runtime(runtime, address, Some(host), timeout_duration)
            .await
            .map_err(|error| error.to_string())?;
        let (path, secure) = (target.path().to_string(), target.scheme() == "https");
        let status = if secure {
            let connector = honk_outbound::tls::build_dns_connector(false, b"\x08http/1.1")
                .map_err(|error| error.to_string())?;
            let mut tls = connector
                .connect(host, proxy.stream)
                .await
                .map_err(|error| error.to_string())?;
            http_status(&mut tls, host, &path).await?
        } else {
            let mut stream = proxy.stream;
            http_status(stream.as_mut(), host, &path).await?
        };
        Ok(ProbeMeasurement {
            latency: started.elapsed(),
            status_code: Some(status),
        })
    }
    .await;
    guard.close().await;
    result
}

#[cfg(feature = "honk-probe")]
async fn http_status<S>(stream: &mut S, host: &str, path: &str) -> Result<u16, String>
where
    S: AsyncRead + AsyncWrite + Unpin + ?Sized,
{
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: {UPSTREAM_USER_AGENT}\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|error| error.to_string())?;
    let mut bytes = [0_u8; 1024];
    let count = stream
        .read(&mut bytes)
        .await
        .map_err(|error| error.to_string())?;
    let line = String::from_utf8_lossy(&bytes[..count]);
    let status = line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| "malformed HTTP status".into())
        .and_then(|value| value.parse::<u16>().map_err(|error| error.to_string()))?;
    if !(200..500).contains(&status) {
        return Err(format!("bad status code: {status}"));
    }
    Ok(status)
}

pub fn latest_probe(probes: &[ProbeRecord], kind: ProbeKind) -> Option<&ProbeRecord> {
    probes.iter().find(|probe| probe.kind == kind.as_str())
}

#[cfg(test)]
mod tests {
    use super::VLESS_UDP_MODES;
    use super::duration_to_latency_ms;
    #[cfg(feature = "honk-probe")]
    use super::reset_vless_modes;
    use super::vless_udp_modes;
    use honk_config::node::Node;
    #[cfg(feature = "honk-probe")]
    use honk_config::types::NodeProtocol;
    use std::time::Duration;

    #[test]
    fn successful_submillisecond_probe_is_reported_as_one_millisecond() {
        assert_eq!(duration_to_latency_ms(Duration::from_micros(400)), Some(1));
    }

    #[test]
    fn vless_mode_probe_candidates_follow_capability_priority() {
        assert_eq!(
            VLESS_UDP_MODES,
            [
                honk_config::node::WireMode::MuxCool,
                honk_config::node::WireMode::H2mux,
                honk_config::node::WireMode::H2muxPadded,
                honk_config::node::WireMode::Xudp,
                honk_config::node::WireMode::UotV2,
            ]
        );
    }

    #[test]
    fn vision_nodes_probe_xudp_before_other_udp_modes() {
        let node = Node {
            flow: Some("xtls-rprx-vision".into()),
            ..Default::default()
        };
        assert_eq!(
            vless_udp_modes(&node),
            [
                honk_config::node::WireMode::Xudp,
                honk_config::node::WireMode::MuxCool,
                honk_config::node::WireMode::H2mux,
                honk_config::node::WireMode::H2muxPadded,
                honk_config::node::WireMode::UotV2,
            ]
        );
    }

    #[cfg(feature = "honk-probe")]
    #[test]
    fn every_vless_mode_is_reset_before_reprobing() {
        let mut nodes = vec![Node {
            protocol: NodeProtocol::VLess,
            vless_mode: honk_config::node::WireMode::Xudp,
            ..Default::default()
        }];
        reset_vless_modes(&mut nodes);
        assert_eq!(nodes[0].vless_mode, honk_config::node::WireMode::Legacy);
        assert_eq!(nodes[0].id, nodes[0].derive_id());
    }

    #[cfg(not(feature = "honk-probe"))]
    #[tokio::test]
    async fn default_build_does_not_report_tcp_connect_as_node_latency() {
        let node =
            honk_config::node::Node::from_share_link("trojan://secret@example.com:443#probe")
                .expect("probe fixture should parse");
        let error = super::probe_inner(&node, super::ProbeKind::Latency, Duration::from_secs(1))
            .await
            .expect_err("default build must require proxy-routed probe support");
        assert!(error.contains("proxy-routed probes"));

        let outcome =
            super::probe_one(&node, super::ProbeKind::Latency, Duration::from_secs(1)).await;
        assert!(!outcome.success);
        assert_eq!(outcome.latency_ms, None);
    }
}
