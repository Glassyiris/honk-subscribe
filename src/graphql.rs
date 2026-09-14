use std::sync::Arc;

use async_graphql::{
    Context, EmptySubscription, InputObject, Object, Result, Schema, SimpleObject,
};
use honk_config::subscription::SubscriptionHeader;

use crate::{
    error::AppError,
    models::{
        NodeSnapshot, RefreshSummary, SubscriptionFilter, SubscriptionSettings,
        SubscriptionSnapshot,
    },
    service::{NodeFilter, Service},
};

pub type AppSchema = Schema<QueryRoot, MutationRoot, EmptySubscription>;

#[derive(InputObject, Default)]
pub struct NodeFilterInput {
    pub search: Option<String>,
    pub region: Option<String>,
    pub protocol: Option<String>,
    pub max_latency_ms: Option<i64>,
    pub min_stability_percent: Option<f64>,
    pub ai_unlocked: Option<bool>,
    pub netflix_unlocked: Option<bool>,
}

#[derive(InputObject)]
pub struct AddSubscriptionInput {
    pub name: String,
    pub url: String,
    #[graphql(default = "custom")]
    pub sub_type: String,
    #[graphql(default = 86400)]
    pub update_interval: i64,
    #[graphql(default = true)]
    pub enabled: bool,
    pub user_agent: Option<String>,
    pub headers: Option<Vec<SubscriptionHeaderInput>>,
    pub rename_template: Option<String>,
    pub filter: Option<SubscriptionFilterInput>,
    #[graphql(default)]
    pub probe_vless_modes: bool,
}

#[derive(InputObject)]
pub struct UpdateSubscriptionInput {
    pub id: String,
    pub name: String,
    pub url: String,
    #[graphql(default = "custom")]
    pub sub_type: String,
    #[graphql(default = 86400)]
    pub update_interval: i64,
    #[graphql(default = true)]
    pub enabled: bool,
    pub user_agent: Option<String>,
    pub headers: Option<Vec<SubscriptionHeaderInput>>,
    pub rename_template: Option<String>,
    pub filter: Option<SubscriptionFilterInput>,
    #[graphql(default)]
    pub probe_vless_modes: bool,
}

#[derive(InputObject)]
pub struct SubscriptionHeaderInput {
    pub key: String,
    pub value: String,
}

#[derive(InputObject, Default)]
pub struct SubscriptionFilterInput {
    pub include: Option<String>,
    pub exclude: Option<String>,
    pub protocol: Option<String>,
}

#[derive(InputObject, Default)]
pub struct AddNodeInput {
    pub share_link: Option<String>,
    pub config: Option<serde_json::Value>,
    pub name: Option<String>,
    pub region: Option<String>,
}

#[derive(InputObject)]
pub struct RenameNodesInput {
    pub ids: Vec<String>,
    pub template: String,
}

#[derive(InputObject)]
pub struct ProbeNodesInput {
    #[graphql(default)]
    pub ids: Vec<String>,
    #[graphql(default)]
    pub kinds: Vec<String>,
    #[graphql(default = 5000)]
    pub timeout_ms: i64,
}

#[derive(SimpleObject)]
pub struct SubscriptionView {
    pub id: String,
    pub name: String,
    pub url: String,
    pub sub_type: String,
    pub update_interval: i64,
    pub enabled: bool,
    pub user_agent: Option<String>,
    pub rename_template: Option<String>,
    pub filter: SubscriptionFilterView,
    pub probe_vless_modes: bool,
    pub last_updated: Option<String>,
    pub node_count: i32,
    pub node_kinds: Vec<String>,
    pub alive_nodes: i32,
    pub probed_nodes: i32,
    pub traffic_total_bytes: Option<i64>,
    pub traffic_used_bytes: Option<i64>,
    pub traffic_remaining_bytes: Option<i64>,
    pub traffic_expire_at: Option<String>,
}

#[derive(SimpleObject)]
pub struct SubscriptionFilterView {
    pub include: Option<String>,
    pub exclude: Option<String>,
    pub protocol: Option<String>,
}

#[derive(SimpleObject)]
pub struct NodeView {
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
    pub last_probe_at: Option<String>,
    pub last_probe_status_code: Option<i64>,
    pub last_probe_error: Option<String>,
}

#[derive(SimpleObject)]
pub struct RefreshView {
    pub subscription: SubscriptionView,
    pub imported_nodes: i32,
}

pub struct QueryRoot;

#[Object]
impl QueryRoot {
    async fn health(&self, ctx: &Context<'_>) -> Result<bool> {
        ctx.data::<Arc<Service>>()?
            .health()
            .map(|()| true)
            .map_err(Into::into)
    }

    async fn subscriptions(&self, ctx: &Context<'_>) -> Result<Vec<SubscriptionView>> {
        ctx.data::<Arc<Service>>()?
            .subscriptions()
            .map(|values| values.into_iter().map(SubscriptionView::from).collect())
            .map_err(Into::into)
    }

    async fn nodes(
        &self,
        ctx: &Context<'_>,
        filter: Option<NodeFilterInput>,
    ) -> Result<Vec<NodeView>> {
        let filter = filter.unwrap_or_default();
        ctx.data::<Arc<Service>>()?
            .nodes(NodeFilter::from(filter))
            .map(|values| values.into_iter().map(NodeView::from).collect())
            .map_err(Into::into)
    }
}

pub struct MutationRoot;

#[Object]
impl MutationRoot {
    async fn add_subscription(
        &self,
        ctx: &Context<'_>,
        input: AddSubscriptionInput,
    ) -> Result<SubscriptionView> {
        ctx.data::<Arc<Service>>()?
            .add_subscription(
                input.name,
                input.url,
                input.sub_type,
                crate::service::SubscriptionOptions {
                    update_interval: input.update_interval,
                    enabled: input.enabled,
                    user_agent: input.user_agent,
                    headers: input
                        .headers
                        .unwrap_or_default()
                        .into_iter()
                        .map(|header| SubscriptionHeader {
                            key: header.key,
                            value: header.value,
                        })
                        .collect(),
                    settings: subscription_settings(
                        input.rename_template,
                        input.filter,
                        input.probe_vless_modes,
                    ),
                },
            )
            .map(SubscriptionView::from)
            .map_err(Into::into)
    }

    async fn update_subscription(
        &self,
        ctx: &Context<'_>,
        input: UpdateSubscriptionInput,
    ) -> Result<SubscriptionView> {
        ctx.data::<Arc<Service>>()?
            .update_subscription(
                input.id,
                input.name,
                input.url,
                input.sub_type,
                crate::service::SubscriptionOptions {
                    update_interval: input.update_interval,
                    enabled: input.enabled,
                    user_agent: input.user_agent,
                    headers: input
                        .headers
                        .unwrap_or_default()
                        .into_iter()
                        .map(|header| SubscriptionHeader {
                            key: header.key,
                            value: header.value,
                        })
                        .collect(),
                    settings: subscription_settings(
                        input.rename_template,
                        input.filter,
                        input.probe_vless_modes,
                    ),
                },
            )
            .map(SubscriptionView::from)
            .map_err(Into::into)
    }

    async fn refresh_subscription(&self, ctx: &Context<'_>, id: String) -> Result<RefreshView> {
        let result = ctx.data::<Arc<Service>>()?.refresh_subscription(id).await?;
        Ok(RefreshView::from(result))
    }

    async fn delete_subscription(&self, ctx: &Context<'_>, id: String) -> Result<bool> {
        ctx.data::<Arc<Service>>()?.delete_subscription(id)?;
        Ok(true)
    }

    async fn rename_subscription_nodes(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> Result<SubscriptionView> {
        ctx.data::<Arc<Service>>()?
            .rename_subscription_nodes(id)
            .map(SubscriptionView::from)
            .map_err(Into::into)
    }

    async fn add_node(&self, ctx: &Context<'_>, input: AddNodeInput) -> Result<NodeView> {
        let node = ctx.data::<Arc<Service>>()?.add_manual_node(
            input.share_link,
            input.config,
            input.name,
            input.region,
        )?;
        let snapshot = ctx
            .data::<Arc<Service>>()?
            .nodes(NodeFilter::default())?
            .into_iter()
            .find(|value| value.id == node.id.to_string())
            .ok_or(AppError::NotFound)?;
        Ok(NodeView::from(snapshot))
    }

    async fn rename_nodes(
        &self,
        ctx: &Context<'_>,
        input: RenameNodesInput,
    ) -> Result<Vec<NodeView>> {
        ctx.data::<Arc<Service>>()?
            .rename_nodes(input.ids, input.template)
            .map(|values| values.into_iter().map(NodeView::from).collect())
            .map_err(Into::into)
    }

    async fn probe_nodes(
        &self,
        ctx: &Context<'_>,
        input: ProbeNodesInput,
    ) -> Result<Vec<NodeView>> {
        ctx.data::<Arc<Service>>()?
            .probe_nodes(input.ids, input.kinds, input.timeout_ms)
            .await
            .map(|values| values.into_iter().map(NodeView::from).collect())
            .map_err(Into::into)
    }
}

impl From<NodeFilterInput> for NodeFilter {
    fn from(value: NodeFilterInput) -> Self {
        Self {
            search: value.search,
            region: value.region,
            protocol: value.protocol,
            max_latency_ms: value.max_latency_ms,
            min_stability_percent: value.min_stability_percent,
            ai_unlocked: value.ai_unlocked,
            netflix_unlocked: value.netflix_unlocked,
        }
    }
}

impl From<NodeSnapshot> for NodeView {
    fn from(value: NodeSnapshot) -> Self {
        Self {
            id: value.id,
            name: value.name,
            protocol: value.protocol,
            address: value.address,
            region: value.region,
            sources: value.sources,
            latency_ms: value.latency_ms,
            stability_percent: value.stability_percent,
            ai_unlocked: value.ai_unlocked,
            netflix_unlocked: value.netflix_unlocked,
            last_probe_ok: value.last_probe_ok,
            last_probe_at: value.last_probe_at.map(|time| time.to_rfc3339()),
            last_probe_status_code: value.last_probe_status_code,
            last_probe_error: value.last_probe_error,
        }
    }
}

impl From<SubscriptionSnapshot> for SubscriptionView {
    fn from(value: SubscriptionSnapshot) -> Self {
        let traffic = value.traffic.as_ref();
        Self {
            id: value.subscription.id.to_string(),
            name: value.subscription.name,
            url: value.subscription.url,
            sub_type: format!("{:?}", value.subscription.sub_type).to_ascii_lowercase(),
            update_interval: i64::try_from(value.subscription.update_interval).unwrap_or(i64::MAX),
            enabled: value.subscription.enabled,
            user_agent: value.subscription.user_agent,
            rename_template: value.settings.rename_template,
            filter: SubscriptionFilterView {
                include: value.settings.filter.include,
                exclude: value.settings.filter.exclude,
                protocol: value.settings.filter.protocol,
            },
            probe_vless_modes: value.settings.probe_vless_modes,
            last_updated: value
                .subscription
                .last_updated
                .map(|time| time.to_rfc3339()),
            node_count: i32::try_from(value.subscription.node_count).unwrap_or(i32::MAX),
            node_kinds: value.node_kinds,
            alive_nodes: i32::try_from(value.alive_nodes).unwrap_or(i32::MAX),
            probed_nodes: i32::try_from(value.probed_nodes).unwrap_or(i32::MAX),
            traffic_total_bytes: traffic.and_then(|item| bytes(item.total)),
            traffic_used_bytes: traffic.and_then(|item| bytes(Some(item.used()))),
            traffic_remaining_bytes: traffic.and_then(|item| bytes(item.remaining())),
            traffic_expire_at: traffic.and_then(|item| item.expire.map(|time| time.to_rfc3339())),
        }
    }
}

fn subscription_settings(
    rename_template: Option<String>,
    filter: Option<SubscriptionFilterInput>,
    probe_vless_modes: bool,
) -> SubscriptionSettings {
    SubscriptionSettings {
        rename_template,
        probe_vless_modes,
        filter: filter.map_or_else(SubscriptionFilter::default, |filter| SubscriptionFilter {
            include: filter.include,
            exclude: filter.exclude,
            protocol: filter.protocol,
        }),
    }
}

impl From<RefreshSummary> for RefreshView {
    fn from(value: RefreshSummary) -> Self {
        Self {
            subscription: SubscriptionView::from(value.subscription),
            imported_nodes: i32::try_from(value.imported_nodes).unwrap_or(i32::MAX),
        }
    }
}

fn bytes(value: Option<u64>) -> Option<i64> {
    value.and_then(|value| i64::try_from(value).ok())
}

pub fn schema(service: Arc<Service>) -> AppSchema {
    Schema::build(QueryRoot, MutationRoot, EmptySubscription)
        .data(service)
        .finish()
}
