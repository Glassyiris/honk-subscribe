use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use honk_config::{node::Node, subscription::Subscription, types::SubscriptionType};
use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use crate::{
    error::AppResult,
    models::{
        ProbeOutcome, ProbeRecord, StoredNode, StoredSubscription, SubscriptionSettings,
        SubscriptionTraffic,
    },
    naming,
};

pub struct Store {
    connection: Mutex<Connection>,
}

impl Store {
    pub fn open(path: &str) -> AppResult<Self> {
        let connection = Connection::open(path)?;
        connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS subscriptions (
               id TEXT PRIMARY KEY, name TEXT NOT NULL, url TEXT NOT NULL,
               sub_type TEXT NOT NULL, update_interval INTEGER NOT NULL,
               enabled INTEGER NOT NULL, user_agent TEXT, headers_json TEXT NOT NULL,
               rename_template TEXT, filter_json TEXT NOT NULL DEFAULT '{}',
               last_updated TEXT, created_at TEXT NOT NULL,
               traffic_upload INTEGER, traffic_download INTEGER,
               traffic_total INTEGER, traffic_expire INTEGER
             );
             CREATE TABLE IF NOT EXISTS nodes (
               id TEXT PRIMARY KEY, name TEXT NOT NULL, node_json TEXT NOT NULL,
               region TEXT, created_at TEXT NOT NULL, updated_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS node_sources (
               node_id TEXT NOT NULL, source_kind TEXT NOT NULL,
               source_key TEXT NOT NULL, label TEXT NOT NULL,
               PRIMARY KEY (node_id, source_kind, source_key),
               FOREIGN KEY (node_id) REFERENCES nodes(id) ON DELETE CASCADE
             );
             CREATE TABLE IF NOT EXISTS probes (
               id INTEGER PRIMARY KEY AUTOINCREMENT, node_id TEXT NOT NULL,
               kind TEXT NOT NULL, success INTEGER NOT NULL, latency_ms INTEGER,
               status_code INTEGER, error TEXT, probed_at TEXT NOT NULL,
               FOREIGN KEY (node_id) REFERENCES nodes(id) ON DELETE CASCADE
             );
             CREATE INDEX IF NOT EXISTS probes_node_time ON probes(node_id, probed_at DESC);",
        )?;
        ensure_subscription_columns(&connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn health(&self) -> AppResult<()> {
        self.connection
            .lock()
            .query_row("SELECT 1", [], |_| Ok(()))?;
        Ok(())
    }

    pub fn save_subscription(
        &self,
        value: &Subscription,
        settings: &SubscriptionSettings,
    ) -> AppResult<()> {
        let headers_json = serde_json::to_string(&value.headers)?;
        let filter_json = serde_json::to_string(settings)?;
        let connection = self.connection.lock();
        connection.execute(
            "INSERT INTO subscriptions
             (id, name, url, sub_type, update_interval, enabled, user_agent, headers_json,
              rename_template, filter_json, last_updated, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(id) DO UPDATE SET name=excluded.name, url=excluded.url,
             sub_type=excluded.sub_type, update_interval=excluded.update_interval,
             enabled=excluded.enabled, user_agent=excluded.user_agent,
             headers_json=excluded.headers_json, rename_template=excluded.rename_template,
             filter_json=excluded.filter_json",
            params![
                value.id.to_string(),
                value.name,
                value.url,
                sub_type_name(value.sub_type),
                i64::try_from(value.update_interval).unwrap_or(i64::MAX),
                i64::from(value.enabled),
                value.user_agent,
                headers_json,
                settings.rename_template,
                filter_json,
                value.last_updated.map(|time| time.to_rfc3339()),
                value.created_at.to_rfc3339()
            ],
        )?;
        Ok(())
    }

    pub fn subscription_settings(&self, id: Uuid) -> AppResult<SubscriptionSettings> {
        let connection = self.connection.lock();
        let (rename_template, filter_json) = connection
            .query_row(
                "SELECT rename_template, filter_json FROM subscriptions WHERE id = ?1",
                [id.to_string()],
                |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?
            .ok_or(crate::error::AppError::NotFound)?;
        let stored = serde_json::from_str::<serde_json::Value>(&filter_json)?;
        if stored.get("filter").is_some() {
            Ok(serde_json::from_value(stored)?)
        } else {
            Ok(SubscriptionSettings {
                rename_template,
                filter: serde_json::from_value(stored)?,
                probe_vless_modes: false,
            })
        }
    }

    pub fn list_subscriptions(&self) -> AppResult<Vec<StoredSubscription>> {
        let connection = self.connection.lock();
        let mut statement = connection.prepare(
            "SELECT id, name, url, sub_type, update_interval, enabled, user_agent,
                    headers_json, last_updated, created_at
             FROM subscriptions ORDER BY created_at DESC",
        )?;
        let rows = statement
            .query_map([], subscription_row)?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|row| subscription_from_row(row).map(|value| StoredSubscription { value }))
            .collect()
    }

    pub fn get_subscription(&self, id: &str) -> AppResult<Subscription> {
        let connection = self.connection.lock();
        let row = connection
            .query_row(
                "SELECT id, name, url, sub_type, update_interval, enabled, user_agent,
                    headers_json, last_updated, created_at FROM subscriptions WHERE id = ?1",
                [id],
                subscription_row,
            )
            .optional()?
            .ok_or(crate::error::AppError::NotFound)?;
        subscription_from_row(row)
    }

    pub fn subscription_traffic(&self, id: Uuid) -> AppResult<Option<SubscriptionTraffic>> {
        let connection = self.connection.lock();
        let row = connection
            .query_row(
                "SELECT traffic_upload, traffic_download, traffic_total, traffic_expire
                 FROM subscriptions WHERE id = ?1",
                [id.to_string()],
                |row| {
                    Ok((
                        row.get::<_, Option<i64>>(0)?,
                        row.get::<_, Option<i64>>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                    ))
                },
            )
            .optional()?
            .ok_or(crate::error::AppError::NotFound)?;
        let (upload, download, total, expire) = row;
        if upload.is_none() && download.is_none() && total.is_none() && expire.is_none() {
            return Ok(None);
        }
        Ok(Some(SubscriptionTraffic {
            upload: upload
                .and_then(|value| u64::try_from(value).ok())
                .unwrap_or_default(),
            download: download
                .and_then(|value| u64::try_from(value).ok())
                .unwrap_or_default(),
            total: total.and_then(|value| u64::try_from(value).ok()),
            expire: expire
                .and_then(|value| u64::try_from(value).ok())
                .and_then(|value| chrono::DateTime::from_timestamp(value as i64, 0)),
        }))
    }

    pub fn subscription_node_ids(&self, id: Uuid) -> AppResult<Vec<String>> {
        let connection = self.connection.lock();
        let mut statement = connection.prepare(
            "SELECT node_id FROM node_sources
             WHERE source_kind = 'subscription' AND source_key = ?1",
        )?;
        Ok(statement
            .query_map([id.to_string()], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?)
    }

    pub fn update_subscription_after_refresh(
        &self,
        id: Uuid,
        count: usize,
        at: DateTime<Utc>,
        traffic: Option<&SubscriptionTraffic>,
    ) -> AppResult<Subscription> {
        let connection = self.connection.lock();
        if let Some(traffic) = traffic {
            connection.execute(
                "UPDATE subscriptions SET last_updated = ?2, traffic_upload = ?3,
                 traffic_download = ?4, traffic_total = ?5, traffic_expire = ?6
                 WHERE id = ?1",
                params![
                    id.to_string(),
                    at.to_rfc3339(),
                    i64::try_from(traffic.upload).unwrap_or(i64::MAX),
                    i64::try_from(traffic.download).unwrap_or(i64::MAX),
                    traffic.total.and_then(|value| i64::try_from(value).ok()),
                    traffic.expire.map(|value| value.timestamp()),
                ],
            )?;
        } else {
            connection.execute(
                "UPDATE subscriptions SET last_updated = ?2 WHERE id = ?1",
                params![id.to_string(), at.to_rfc3339()],
            )?;
        }
        drop(connection);
        let mut subscription = self.get_subscription(&id.to_string())?;
        subscription.node_count = u32::try_from(count).unwrap_or(u32::MAX);
        Ok(subscription)
    }

    pub fn delete_subscription(&self, id: Uuid) -> AppResult<()> {
        let mut connection = self.connection.lock();
        let transaction = connection.transaction()?;
        let deleted =
            transaction.execute("DELETE FROM subscriptions WHERE id = ?1", [id.to_string()])?;
        if deleted == 0 {
            return Err(crate::error::AppError::NotFound);
        }
        transaction.execute(
            "DELETE FROM node_sources WHERE source_kind = 'subscription' AND source_key = ?1",
            [id.to_string()],
        )?;
        transaction.execute(
            "DELETE FROM nodes WHERE id NOT IN (SELECT node_id FROM node_sources)",
            [],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn replace_subscription_nodes(
        &self,
        subscription: &Subscription,
        nodes: &[Node],
    ) -> AppResult<()> {
        let mut connection = self.connection.lock();
        let transaction = connection.transaction()?;
        transaction.execute(
            "DELETE FROM node_sources WHERE source_kind = 'subscription' AND source_key = ?1",
            [subscription.id.to_string()],
        )?;
        for node in nodes {
            let name = naming::normalize_proxy_name(&node.name);
            let region = naming::region(&name);
            let node_json = serde_json::to_string(node)?;
            transaction.execute(
                "INSERT INTO nodes (id, name, node_json, region, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5)
                 ON CONFLICT(id) DO UPDATE SET node_json=excluded.node_json,
                 name=excluded.name, region=COALESCE(excluded.region, nodes.region),
                 updated_at=excluded.updated_at",
                params![
                    node.id.to_string(),
                    name,
                    node_json,
                    region,
                    Utc::now().to_rfc3339()
                ],
            )?;
            transaction.execute(
                "INSERT OR REPLACE INTO node_sources (node_id, source_kind, source_key, label)
                 VALUES (?1, 'subscription', ?2, ?3)",
                params![
                    node.id.to_string(),
                    subscription.id.to_string(),
                    subscription.name
                ],
            )?;
        }
        transaction.execute(
            "DELETE FROM nodes WHERE id NOT IN (SELECT node_id FROM node_sources)",
            [],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn save_manual_node(&self, node: &Node, region: Option<&str>) -> AppResult<()> {
        let mut connection = self.connection.lock();
        let transaction = connection.transaction()?;
        let node_json = serde_json::to_string(node)?;
        let name = naming::normalize_proxy_name(&node.name);
        let region = region
            .map(naming::normalize_region)
            .or_else(|| naming::region(&name));
        transaction.execute(
            "INSERT INTO nodes (id, name, node_json, region, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)
             ON CONFLICT(id) DO UPDATE SET node_json=excluded.node_json,
             name=excluded.name, region=COALESCE(excluded.region, nodes.region), updated_at=excluded.updated_at",
            params![node.id.to_string(), name, node_json, region, Utc::now().to_rfc3339()],
        )?;
        transaction.execute(
            "INSERT OR REPLACE INTO node_sources (node_id, source_kind, source_key, label)
             VALUES (?1, 'manual', 'manual', '手动节点')",
            [node.id.to_string()],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn rename_nodes(&self, names: &[(String, String)]) -> AppResult<()> {
        let mut connection = self.connection.lock();
        let transaction = connection.transaction()?;
        for (id, name) in names {
            let json: String = transaction.query_row(
                "SELECT node_json FROM nodes WHERE id = ?1",
                [id],
                |row| row.get(0),
            )?;
            let mut node: Node = serde_json::from_str(&json)?;
            node.name = name.clone();
            transaction.execute(
                "UPDATE nodes SET name = ?2, node_json = ?3, updated_at = ?4 WHERE id = ?1",
                params![
                    id,
                    name,
                    serde_json::to_string(&node)?,
                    Utc::now().to_rfc3339()
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    #[cfg_attr(not(feature = "honk-probe"), allow(dead_code))]
    pub fn save_probe(&self, outcome: &ProbeOutcome) -> AppResult<()> {
        self.connection.lock().execute(
            "INSERT INTO probes (node_id, kind, success, latency_ms, status_code, error, probed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                outcome.node_id,
                outcome.kind,
                i64::from(outcome.success),
                outcome.latency_ms,
                outcome.status_code,
                outcome.error,
                outcome.probed_at.to_rfc3339()
            ],
        )?;
        Ok(())
    }

    pub fn list_nodes(&self) -> AppResult<Vec<StoredNode>> {
        let connection = self.connection.lock();
        let mut statement = connection.prepare(
            "SELECT id, name, node_json, region FROM nodes ORDER BY name COLLATE NOCASE",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut sources: HashMap<String, Vec<String>> = HashMap::new();
        let mut source_statement =
            connection.prepare("SELECT node_id, label FROM node_sources ORDER BY label")?;
        for row in source_statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })? {
            let (id, label) = row?;
            sources.entry(id).or_default().push(label);
        }
        let mut probes: HashMap<String, Vec<ProbeRecord>> = HashMap::new();
        let mut probe_statement = connection.prepare("SELECT node_id, kind, success, latency_ms, status_code, error, probed_at FROM probes ORDER BY probed_at DESC LIMIT 100000")?;
        for row in probe_statement.query_map([], probe_row)? {
            let (id, probe) = row?;
            probes.entry(id).or_default().push(probe);
        }
        rows.into_iter()
            .map(|(id, display_name, json, region)| {
                let mut node: Node = serde_json::from_str(&json)?;
                if node.id.to_string() != id {
                    return Err(crate::error::AppError::InvalidInput(
                        "node identity mismatch".into(),
                    ));
                }
                node.name = naming::normalize_proxy_name(&display_name);
                Ok(StoredNode {
                    node,
                    display_name: naming::normalize_proxy_name(&display_name),
                    region: region
                        .map(|value| naming::normalize_region(&value))
                        .or_else(|| naming::region(&display_name)),
                    sources: sources.remove(&id).unwrap_or_default(),
                    probes: probes.remove(&id).unwrap_or_default(),
                })
            })
            .collect()
    }
}
fn ensure_subscription_columns(connection: &Connection) -> rusqlite::Result<()> {
    let mut statement = connection.prepare("PRAGMA table_info(subscriptions)")?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<HashSet<_>, _>>()?;
    for (name, kind) in [
        ("rename_template", "TEXT"),
        ("filter_json", "TEXT NOT NULL DEFAULT '{}'"),
        ("traffic_upload", "INTEGER"),
        ("traffic_download", "INTEGER"),
        ("traffic_total", "INTEGER"),
        ("traffic_expire", "INTEGER"),
    ] {
        if !columns.contains(name) {
            connection.execute(
                &format!("ALTER TABLE subscriptions ADD COLUMN {name} {kind}"),
                [],
            )?;
        }
    }
    Ok(())
}

type SubscriptionRow = (
    String,
    String,
    String,
    String,
    i64,
    bool,
    Option<String>,
    String,
    Option<String>,
    String,
);

fn subscription_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SubscriptionRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get::<_, i64>(5)? != 0,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
    ))
}

fn subscription_from_row(row: SubscriptionRow) -> AppResult<Subscription> {
    let id = Uuid::parse_str(&row.0)
        .map_err(|_| crate::error::AppError::InvalidInput("invalid subscription id".into()))?;
    let last_updated = row.8.as_deref().map(parse_time).transpose()?;
    let created_at = parse_time(&row.9)?;
    Ok(Subscription {
        id,
        name: row.1,
        url: row.2,
        sub_type: parse_type(&row.3)?,
        update_interval: u64::try_from(row.4).unwrap_or_default(),
        user_agent: row.6,
        headers: serde_json::from_str(&row.7)?,
        enabled: row.5,
        last_updated,
        node_count: 0,
        created_at,
    })
}

fn parse_time(value: &str) -> AppResult<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|time| time.with_timezone(&Utc))
        .map_err(|_| crate::error::AppError::InvalidInput("invalid stored timestamp".into()))
}

fn parse_type(value: &str) -> AppResult<SubscriptionType> {
    match value {
        "simple" => Ok(SubscriptionType::Simple),
        "clash" => Ok(SubscriptionType::Clash),
        "sip008" => Ok(SubscriptionType::Sip008),
        "custom" => Ok(SubscriptionType::Custom),
        _ => Err(crate::error::AppError::InvalidInput(
            "invalid subscription type".into(),
        )),
    }
}

fn sub_type_name(value: SubscriptionType) -> &'static str {
    match value {
        SubscriptionType::Simple => "simple",
        SubscriptionType::Clash => "clash",
        SubscriptionType::Sip008 => "sip008",
        SubscriptionType::Custom => "custom",
    }
}

fn probe_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<(String, ProbeRecord)> {
    let timestamp: String = row.get(6)?;
    let probed_at = DateTime::parse_from_rfc3339(&timestamp)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|_| rusqlite::Error::InvalidQuery)?;
    Ok((
        row.get(0)?,
        ProbeRecord {
            kind: row.get(1)?,
            success: row.get::<_, i64>(2)? != 0,
            latency_ms: row.get(3)?,
            status_code: row.get(4)?,
            error: row.get(5)?,
            probed_at,
        },
    ))
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use honk_config::{node::Node, subscription::Subscription};
    use rusqlite::params;
    use uuid::Uuid;

    use crate::models::{SubscriptionFilter, SubscriptionSettings};

    use super::Store;

    #[test]
    fn legacy_nodes_infer_region_when_listed() {
        let store = Store::open(":memory:").expect("in-memory store");
        let mut node = Node::default();
        node.name = "🇭🇰 yss.hk.01".into();
        node.id = node.derive_id();
        let subscription = Subscription {
            id: Uuid::new_v4(),
            ..Default::default()
        };

        store
            .replace_subscription_nodes(&subscription, &[node])
            .expect("store node");
        store
            .connection
            .lock()
            .execute("UPDATE nodes SET region = NULL", [])
            .expect("simulate legacy row");

        let nodes = store.list_nodes().expect("list nodes");
        assert_eq!(nodes[0].region.as_deref(), Some("hk"));
    }

    #[test]
    fn legacy_proxy_cn_is_normalized_to_tw_when_listed() {
        let store = Store::open(":memory:").expect("in-memory store");
        let mut node = Node::default();
        node.name = "🇨🇳 nexi.cn.01".into();
        node.id = node.derive_id();
        let node_id = node.id.to_string();
        let subscription_id = Uuid::new_v4();
        let now = Utc::now().to_rfc3339();
        store
            .connection
            .lock()
            .execute(
                "INSERT INTO nodes (id, name, node_json, region, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                params![
                    node_id,
                    "🇨🇳 nexi.cn.01",
                    serde_json::to_string(&node).expect("node JSON"),
                    "cn",
                    now
                ],
            )
            .expect("legacy node");
        store
            .connection
            .lock()
            .execute(
                "INSERT INTO node_sources (node_id, source_kind, source_key, label)
                 VALUES (?1, 'subscription', ?2, 'nexi')",
                params![node.id.to_string(), subscription_id.to_string()],
            )
            .expect("legacy source");

        let nodes = store.list_nodes().expect("list nodes");
        assert_eq!(nodes[0].display_name, "🇹🇼 nexi.tw.01");
        assert_eq!(nodes[0].region.as_deref(), Some("tw"));
        assert_eq!(nodes[0].node.name, "🇹🇼 nexi.tw.01");
    }

    #[test]
    fn subscription_settings_round_trip_vless_probe_toggle() {
        let store = Store::open(":memory:").expect("in-memory store");
        let subscription = Subscription {
            id: Uuid::new_v4(),
            ..Default::default()
        };
        let settings = SubscriptionSettings {
            rename_template: Some("{provider}.{index}".into()),
            filter: SubscriptionFilter {
                protocol: Some("vless".into()),
                ..Default::default()
            },
            probe_vless_modes: true,
        };

        store
            .save_subscription(&subscription, &settings)
            .expect("save settings");
        assert_eq!(
            store
                .subscription_settings(subscription.id)
                .expect("load settings"),
            settings
        );
    }
}
