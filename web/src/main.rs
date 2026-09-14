use gloo_net::http::Request;
use leptos::{
    ev::SubmitEvent,
    leptos_dom::helpers::queue_microtask,
    mount::mount_to_body,
    prelude::*,
    task::spawn_local,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{cmp::Ordering, collections::{BTreeMap, BTreeSet}};
use wasm_bindgen::JsCast;

mod providers;

use providers::{Provider, ProviderCard, ProviderCardProps, ProviderDialog};

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Node {
    id: String,
    name: String,
    protocol: String,
    address: String,
    region: Option<String>,
    sources: Vec<String>,
    latency_ms: Option<i64>,
    last_probe_ok: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct GraphqlError {
    message: String,
}

#[derive(Debug, Deserialize)]
struct GraphqlResponse<T> {
    data: Option<T>,
    errors: Option<Vec<GraphqlError>>,
}

#[derive(Debug, Deserialize)]
struct Dashboard {
    nodes: Vec<Node>,
    subscriptions: Vec<Provider>,
}

#[derive(Debug, Serialize)]
struct GraphqlRequest {
    query: String,
    variables: Value,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Drawer {
    ManualNode,
    Provider,
    Export,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    NodeInventory,
    Subscriptions,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Theme {
    Dark,
    Light,
}

impl Theme {
    fn toggle(self) -> Self {
        match self {
            Self::Dark => Self::Light,
            Self::Light => Self::Dark,
        }
    }

    fn storage_value(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Density {
    Comfortable,
    Compact,
}

impl Density {
    fn toggle(self) -> Self {
        match self {
            Self::Comfortable => Self::Compact,
            Self::Compact => Self::Comfortable,
        }
    }

    fn storage_value(self) -> &'static str {
        match self {
            Self::Comfortable => "comfortable",
            Self::Compact => "compact",
        }
    }
}

#[derive(Clone, Copy)]
struct ExportOption {
    format: &'static str,
    label: &'static str,
    description: &'static str,
}

const EXPORT_OPTIONS: &[ExportOption] = &[
    ExportOption {
        format: "sip002",
        label: "SIP002 + SIP003",
        description: "Shadowsocks URI，包含插件参数",
    },
    ExportOption {
        format: "sip008",
        label: "SIP008",
        description: "Shadowsocks JSON 订阅",
    },
    ExportOption {
        format: "simple",
        label: "通用分享链接",
        description: "VLESS / VMess / Trojan 等节点链接",
    },
    ExportOption {
        format: "clash",
        label: "YAML（Clash）",
        description: "Clash / Clash Meta 订阅",
    },
    ExportOption {
        format: "sing-box",
        label: "JSON（sing-box）",
        description: "sing-box JSON 订阅",
    },
    ExportOption {
        format: "surge",
        label: "Surge",
        description: "Surge 订阅链接",
    },
    ExportOption {
        format: "dae",
        label: "DAE",
        description: "dae 订阅链接",
    },
];

fn stored_setting(key: &str) -> Option<String> {
    web_sys::window()
        .and_then(|window| window.local_storage().ok().flatten())
        .and_then(|storage| storage.get_item(key).ok().flatten())
}

fn save_setting(key: &str, value: &str) {
    if let Some(storage) = web_sys::window().and_then(|window| window.local_storage().ok().flatten()) {
        let _ = storage.set_item(key, value);
    }
}

fn load_theme() -> Theme {
    match stored_setting("honk.theme").as_deref() {
        Some("light") => Theme::Light,
        _ => Theme::Dark,
    }
}

fn load_density() -> Density {
    match stored_setting("honk.density").as_deref() {
        Some("compact") => Density::Compact,
        _ => Density::Comfortable,
    }
}

pub(crate) async fn graphql<T: for<'de> Deserialize<'de>>(
    query: &str,
    variables: Value,
) -> Result<T, String> {
    let request = GraphqlRequest {
        query: query.into(),
        variables,
    };
    let body = serde_json::to_string(&request).map_err(|error| error.to_string())?;
    let response = Request::post("/graphql")
        .header("content-type", "application/json")
        .body(body)
        .map_err(|error| error.to_string())?
        .send()
        .await
        .map_err(|error| error.to_string())?;
    let response: GraphqlResponse<T> = response.json().await.map_err(|error| error.to_string())?;
    if let Some(errors) = response.errors.filter(|errors| !errors.is_empty()) {
        return Err(errors
            .into_iter()
            .map(|error| error.message)
            .collect::<Vec<_>>()
            .join(", "));
    }
    response.data.ok_or_else(|| "API returned no data".into())
}

#[component]
fn App() -> impl IntoView {
    let (nodes, set_nodes) = signal(Vec::<Node>::new());
    let (providers, set_providers) = signal(Vec::<Provider>::new());
    let (page, set_page) = signal(Page::NodeInventory);
    let (search, set_search) = signal(String::new());
    let (region_filter, set_region_filter) = signal(String::new());
    let (provider_filter, set_provider_filter) = signal(String::new());
    let (protocol_filter, set_protocol_filter) = signal(String::new());
    let (sort_by, set_sort_by) = signal(String::from("default"));
    let (selected_node, set_selected_node) = signal(None::<String>);
    let (probing_node, set_probing_node) = signal(None::<String>);
    let (theme, set_theme) = signal(load_theme());
    let (density, set_density) = signal(load_density());
    let (drawer, set_drawer) = signal(None::<Drawer>);
    let (editing_provider, set_editing_provider) = signal(None::<Provider>);
    let background_hidden = Signal::derive(move || drawer.get().is_some().then_some("true"));
    let background_inert = Signal::derive(move || drawer.get().is_some());
    let (restore_sidebar, set_restore_sidebar) = signal(false);
    let header_trigger = NodeRef::<leptos::html::Button>::new();
    let provider_sidebar_trigger = NodeRef::<leptos::html::Button>::new();
    let provider_header_trigger = NodeRef::<leptos::html::Button>::new();
    let export_trigger = NodeRef::<leptos::html::Button>::new();
    let (loading, set_loading) = signal(true);
    let (message, set_message) = signal(String::new());
    let (message_error, set_message_error) = signal(false);

    let toggle_theme = move |_| {
        let next = theme.get().toggle();
        save_setting("honk.theme", next.storage_value());
        set_theme.set(next);
    };
    let toggle_density = move |_| {
        let next = density.get().toggle();
        save_setting("honk.density", next.storage_value());
        set_density.set(next);
    };

    let reload = move || {
        set_loading.set(true);
        spawn_local(async move {
            let result = graphql::<Dashboard>(
                "query Dashboard { nodes { id name protocol address region sources latencyMs lastProbeOk } subscriptions { id name url subType updateInterval enabled userAgent renameTemplate probeVlessModes filter { include exclude protocol } lastUpdated nodeCount nodeKinds aliveNodes probedNodes trafficTotalBytes trafficUsedBytes trafficRemainingBytes trafficExpireAt } }",
                Value::Null,
            )
            .await;
            match result {
                Ok(value) => {
                    set_nodes.set(value.nodes);
                    set_providers.set(value.subscriptions);
                }
                Err(error) => {
                    set_message_error.set(true);
                    set_message.set(error);
                }
            }
            set_loading.set(false);
        });
    };
    reload();

    let visible_nodes = move || {
        let query = search.get().to_ascii_lowercase();
        let region = region_filter.get();
        let provider = provider_filter.get();
        let protocol = protocol_filter.get();
        let sort = sort_by.get();
        let mut visible = nodes
            .get()
            .into_iter()
            .filter(|node| {
                (query.is_empty()
                    || node.name.to_ascii_lowercase().contains(&query)
                    || node.address.to_ascii_lowercase().contains(&query))
                    && (region.is_empty() || node_region(node) == region)
                    && (provider.is_empty()
                        || node.sources.iter().any(|source| source == &provider))
                    && (protocol.is_empty() || node.protocol == protocol)
            })
            .collect::<Vec<_>>();
        visible.sort_by(|left, right| compare_nodes(left, right, &sort));
        visible
    };
    let has_filters = move || {
        !search.get().is_empty()
            || !region_filter.get().is_empty()
            || !provider_filter.get().is_empty()
            || !protocol_filter.get().is_empty()
            || sort_by.get() != "default"
    };
    let clear_filters = move |_| {
        set_search.set(String::new());
        set_region_filter.set(String::new());
        set_provider_filter.set(String::new());
        set_protocol_filter.set(String::new());
        set_sort_by.set("default".into());
    };
    let export_provider_names = Signal::derive(move || {
        let mut names = BTreeSet::new();
        for node in nodes.get() {
            if node.sources.is_empty() {
                names.insert("手动节点".to_string());
            } else {
                names.extend(node.sources);
            }
        }
        names.into_iter().collect::<Vec<_>>()
    });
    let open_manual_header = move |_| {
        set_page.set(Page::NodeInventory);
        set_restore_sidebar.set(false);
        set_drawer.set(Some(Drawer::ManualNode));
    };
    let open_provider_sidebar = move |_| {
        set_page.set(Page::Subscriptions);
        set_drawer.set(None);
    };
    let open_provider_header = move |_| {
        set_page.set(Page::Subscriptions);
        set_restore_sidebar.set(false);
        set_editing_provider.set(None);
        set_drawer.set(Some(Drawer::Provider));
    };
    let open_export = move |_| {
        set_page.set(Page::NodeInventory);
        set_restore_sidebar.set(false);
        set_drawer.set(Some(Drawer::Export));
    };
    let edit_provider = move |provider: Provider| {
        set_page.set(Page::Subscriptions);
        set_restore_sidebar.set(false);
        set_editing_provider.set(Some(provider));
        set_drawer.set(Some(Drawer::Provider));
    };
    let close_drawer = move || {
        let target = match (drawer.get(), restore_sidebar.get()) {
            (Some(Drawer::Provider), true) => provider_sidebar_trigger.get(),
            (Some(Drawer::Provider), false) => provider_header_trigger.get(),
            (Some(Drawer::Export), _) => export_trigger.get(),
            (Some(Drawer::ManualNode), _) | (None, _) => header_trigger.get(),
        };
        set_drawer.set(None);
        queue_microtask(move || {
            if let Some(target) = target {
                let _ = target.focus();
            }
        });
    };
    let refresh_provider = move |id: String| {
        set_loading.set(true);
        spawn_local(async move {
            let result = graphql::<Value>(
                "mutation RefreshProvider($id: String!) { refreshSubscription(id: $id) { importedNodes } }",
                json!({"id": id}),
            )
            .await;
            match result {
                Ok(_) => {
                    set_message_error.set(false);
                    set_message.set("provider 已刷新".into());
                    reload();
                }
                Err(error) => {
                    set_message_error.set(true);
                    set_message.set(error);
                    set_loading.set(false);
                }
            }
        });
    };
    let delete_provider = move |id: String| {
        spawn_local(async move {
            let result = graphql::<Value>(
                "mutation DeleteProvider($id: String!) { deleteSubscription(id: $id) }",
                json!({"id": id}),
            )
            .await;
            match result {
                Ok(_) => {
                    set_message_error.set(false);
                    set_message.set("订阅已删除".into());
                    reload();
                }
                Err(error) => {
                    set_message_error.set(true);
                    set_message.set(error);
                }
            }
        });
    };
    let rename_provider = move |id: String| {
        set_loading.set(true);
        spawn_local(async move {
            let result = graphql::<Value>(
                "mutation RenameProvider($id: String!) { renameSubscriptionNodes(id: $id) { id } }",
                json!({"id": id}),
            )
            .await;
            match result {
                Ok(_) => {
                    set_message_error.set(false);
                    set_message.set("节点已按 provider 规则重命名".into());
                    reload();
                }
                Err(error) => {
                    set_message_error.set(true);
                    set_message.set(error);
                    set_loading.set(false);
                }
            }
        });
    };
    let probe_provider = move |provider_name: String| {
        let ids = nodes
            .get()
            .into_iter()
            .filter(|node| node.sources.iter().any(|source| source == &provider_name))
            .map(|node| node.id)
            .collect::<Vec<_>>();
        if ids.is_empty() {
            set_message_error.set(true);
            set_message.set("provider 没有可拨测节点".into());
            return;
        }
        set_loading.set(true);
        spawn_local(async move {
            let result = graphql::<Value>(
                "mutation ProbeNodes($ids: [String!]!, $kinds: [String!]!, $timeoutMs: Int!) { probeNodes(input: { ids: $ids, kinds: $kinds, timeoutMs: $timeoutMs }) { id } }",
                json!({"ids": ids, "kinds": ["latency"], "timeoutMs": 5000}),
            )
            .await;
            match result {
                Ok(_) => {
                    set_message_error.set(false);
                    set_message.set("provider 延迟拨测完成".into());
                    reload();
                }
                Err(error) => {
                    set_message_error.set(true);
                    set_message.set(error);
                    set_loading.set(false);
                }
            }
        });
    };
    let probe_selected_node = move |_| {
        let Some(id) = selected_node.get() else {
            set_message_error.set(true);
            set_message.set("先选择一个节点".into());
            return;
        };
        set_probing_node.set(Some(id.clone()));
        spawn_local(async move {
            let result = graphql::<Value>(
                "mutation ProbeNodes($ids: [String!]!, $kinds: [String!]!, $timeoutMs: Int!) { probeNodes(input: { ids: $ids, kinds: $kinds, timeoutMs: $timeoutMs }) { id } }",
                json!({"ids": [id], "kinds": ["latency", "ai", "netflix"], "timeoutMs": 5000}),
            )
            .await;
            match result {
                Ok(_) => {
                    set_message_error.set(false);
                    set_message.set("节点拨测完成：延迟 / AI / Netflix".into());
                    set_probing_node.set(None);
                    reload();
                }
                Err(error) => {
                    set_message_error.set(true);
                    set_message.set(error);
                    set_probing_node.set(None);
                }
            }
        });
    };

    view! {
        <div class="shell" class:theme-light=move || theme.get() == Theme::Light class:density-compact=move || density.get() == Density::Compact aria-hidden=background_hidden inert=background_inert>
            <aside class="sidebar" aria-label="主导航">
                <div class="brand"><span class="brand-mark">"H"</span><span>"HONK / CONTROL"</span></div>
                <nav class="nav">
                    <span class="nav-label">"Workspace"</span>
                    <button class:active=move || page.get() == Page::NodeInventory type="button" on:click=move |_| { set_page.set(Page::NodeInventory); set_drawer.set(None); }>"节点库存"</button>
                    <button node_ref=provider_sidebar_trigger class:active=move || page.get() == Page::Subscriptions type="button" on:click=open_provider_sidebar>"订阅管理"</button>
                </nav>
                <div class="sidebar-foot">"API ONLINE"<br/><small>"SQLite · honk-config"</small></div>
            </aside>
            <main class="main">
                <header class="topline">
                    <div><p class="eyebrow">{move || if page.get() == Page::Subscriptions { "Subscription operations / 02" } else { "Node operations / 01" }}</p><h1>{move || if page.get() == Page::Subscriptions { "订阅管理" } else { "节点库存" }}</h1><p class="lede">{move || if page.get() == Page::Subscriptions { "管理 provider，查看节点种类、存活和剩余流量状态。" } else { "聚合订阅与手动节点，按可用性和速度输出到目标客户端。" }}</p></div>
                    <div class="actions">
                        <div class="view-controls" aria-label="视图设置">
                            <button class="ghost compact" type="button" on:click=toggle_theme aria-pressed=move || theme.get() == Theme::Light title=move || if theme.get() == Theme::Light { "切换深色主题" } else { "切换浅色主题" }>{move || if theme.get() == Theme::Light { "深色" } else { "浅色" }}</button>
                            <button class="ghost compact" type="button" on:click=toggle_density aria-pressed=move || density.get() == Density::Compact title=move || if density.get() == Density::Compact { "切换舒展布局" } else { "切换紧凑布局" }>{move || if density.get() == Density::Compact { "舒展" } else { "紧凑" }}</button>
                        </div>
                        <Show when=move || page.get() == Page::Subscriptions fallback=move || ()>
                            <button node_ref=provider_header_trigger class="primary" type="button" on:click=open_provider_header>"＋ 添加订阅"</button>
                        </Show>
                        <Show when=move || page.get() == Page::NodeInventory fallback=move || ()>
                            <button node_ref=provider_header_trigger class="primary" type="button" on:click=open_provider_header>"＋ 添加 provider"</button><button node_ref=header_trigger class="ghost" type="button" on:click=open_manual_header>"＋ 手动添加节点"</button><button node_ref=export_trigger class="ghost" type="button" on:click=open_export>"订阅链接"</button>
                        </Show>
                    </div>
                </header>
                <Show when=move || page.get() == Page::NodeInventory fallback=|| ()>
                    <section class="metrics" aria-label="节点概览">
                        <Metric label="Total nodes" value=move || nodes.get().len().to_string() />
                        <Metric label="Healthy now" value=move || nodes.get().iter().filter(|node| node.last_probe_ok == Some(true)).count().to_string() />
                        <Metric label="Median latency" value=move || median_latency(&nodes.get()) />
                    </section>
                    <section class="toolbar" aria-label="节点操作">
                        <div class="toolbar-head"><div><strong>"Node inventory"</strong><span class="muted">{move || format!("{} records", visible_nodes().len())}</span></div><div class="toolbar-actions"><button class="primary compact" type="button" disabled=move || selected_node.get().is_none() || probing_node.get().is_some() on:click=probe_selected_node title="选择节点后执行完整拨测">{move || if probing_node.get().is_some() { "拨测中…" } else { "手动拨测" }}</button><Show when=has_filters fallback=|| ()><button class="ghost compact" type="button" on:click=clear_filters>"清除筛选"</button></Show></div></div>
                        <div class="node-filters">
                            <input class="control" type="search" placeholder="搜索名称或地址" aria-label="搜索节点" prop:value=search on:input=move |event| set_search.set(event_target_value(&event)) />
                            <select class="control" aria-label="按地区过滤" prop:value=region_filter on:change=move |event| set_region_filter.set(event_target_value(&event))><option value="">"全部地区"</option>{move || node_regions(&nodes.get()).into_iter().map(|region| view! { <option value=region.clone()>{region.clone()}</option> }).collect_view()}</select>
                            <select class="control" aria-label="按 provider 过滤" prop:value=provider_filter on:change=move |event| set_provider_filter.set(event_target_value(&event))><option value="">"全部 provider"</option>{move || node_providers(&nodes.get()).into_iter().map(|provider| view! { <option value=provider.clone()>{provider.clone()}</option> }).collect_view()}</select>
                            <select class="control" aria-label="按支持协议过滤" prop:value=protocol_filter on:change=move |event| set_protocol_filter.set(event_target_value(&event))><option value="">"全部协议"</option>{move || node_protocols(&nodes.get()).into_iter().map(|protocol| view! { <option value=protocol.clone()>{protocol.clone()}</option> }).collect_view()}</select>
                            <select class="control" aria-label="节点排序" prop:value=sort_by on:change=move |event| set_sort_by.set(event_target_value(&event))><option value="default">"默认顺序"</option><option value="region">"地区"</option><option value="provider">"provider"</option><option value="latency">"延迟"</option><option value="protocol">"支持协议"</option></select>
                        </div>
                    </section>
                    <section class="inventory" aria-live="polite">
                        {move || if loading.get() && nodes.get().is_empty() { view! { <div class="empty"><span class="empty-mark" aria-hidden="true">"…"</span><div><strong>"正在连接控制面板"</strong>"读取 SQLite 节点库存…"</div></div> }.into_any() } else if visible_nodes().is_empty() { view! { <div class="empty"><span class="empty-mark" aria-hidden="true">"＋"</span><div><strong>"还没有节点"</strong>"使用手动输入弹窗或订阅 API 开始聚合。"</div></div> }.into_any() } else { view! { <div class="node-list">{move || node_groups(&visible_nodes(), &sort_by.get()).into_iter().map(|group| NodeGroupCard(NodeGroupCardProps { group, selected_node, set_selected_node, probing_node })).collect_view()}</div> }.into_any() }}
                    </section>
                </Show>
                <Show when=move || page.get() == Page::Subscriptions fallback=|| ()>
                    <section class="provider-management" aria-label="订阅管理">
                        <div class="section-head"><div><p class="eyebrow">"Providers"</p><h2>"订阅源"</h2></div><button class="ghost" type="button" on:click=open_provider_header>"＋ 添加订阅"</button></div>
                        <p class="management-hint">"每个 provider 的节点类型、探测存活数和订阅流量在刷新后更新。未提供标准流量头时显示“"<span class="nowrap">"未提供"</span>"”。"</p>
                        {move || if loading.get() && providers.get().is_empty() { view! { <div class="empty provider-empty"><span class="empty-mark" aria-hidden="true">"…"</span><div><strong>"正在读取订阅"</strong>"连接 SQLite provider 列表…"</div></div> }.into_any() } else if providers.get().is_empty() { view! { <div class="empty provider-empty"><span class="empty-mark" aria-hidden="true">"＋"</span><div><strong>"还没有订阅"</strong>"添加订阅地址后，从这里管理 provider。"</div></div> }.into_any() } else { view! { <div class="provider-list">{move || providers.get().into_iter().map(|provider| ProviderCard(ProviderCardProps { provider, on_refresh: refresh_provider, on_edit: edit_provider, on_rename: rename_provider, on_probe: probe_provider, on_delete: delete_provider })).collect_view()}</div> }.into_any() }}
                    </section>
                </Show>
                <Show when=move || !message.get().is_empty() fallback=|| ()><div class="toast" class:toast-error=move || message_error.get() role=move || if message_error.get() { "alert" } else { "status" }>{message}</div></Show>
            </main>
        </div>
        <Show when=move || drawer.get() == Some(Drawer::ManualNode) fallback=|| ()>
            <ManualNodeDialog light=Signal::derive(move || theme.get() == Theme::Light) close=close_drawer reload=reload set_message=set_message set_message_error=set_message_error />
        </Show>
        <Show when=move || drawer.get() == Some(Drawer::Provider) fallback=|| ()>
            <ProviderDialog provider=Signal::derive(move || editing_provider.get()) light=Signal::derive(move || theme.get() == Theme::Light) close=close_drawer reload=reload set_message=set_message set_message_error=set_message_error />
        </Show>
        <Show when=move || drawer.get() == Some(Drawer::Export) fallback=|| ()>
            <ExportDialog close=close_drawer light=Signal::derive(move || theme.get() == Theme::Light) provider_names=export_provider_names />
        </Show>
    }
}

#[component]
fn Metric(label: &'static str, value: impl IntoView) -> impl IntoView {
    view! { <div class="metric"><span class="metric-label">{label}</span><strong class="metric-value">{value}</strong></div> }
}

#[component]
fn ExportDialog(
    close: impl Fn() + Copy + Send + Sync + 'static,
    light: Signal<bool>,
    provider_names: Signal<Vec<String>>,
) -> impl IntoView {
    let (total_limit, set_total_limit) = signal(String::new());
    let (region_limit, set_region_limit) = signal(String::new());
    let (exclude_dead, set_exclude_dead) = signal(false);
    let (selected_providers, set_selected_providers) = signal(Vec::<String>::new());
    let (copy_status, set_copy_status) = signal(None::<(&'static str, bool)>);
    let close_button = NodeRef::<leptos::html::Button>::new();
    close_button.on_load(|button| {
        let _ = button.focus();
    });
    view! {
        <div class="modal-backdrop" class:theme-light=light role="presentation" on:click=move |_| close()>
            <section class="modal export-modal" role="dialog" aria-modal="true" aria-labelledby="export-title" on:keydown=move |event| { if event.key() == "Escape" { event.prevent_default(); close(); } } on:click=|event| event.stop_propagation()>
                <div class="modal-head"><div><p class="eyebrow">"Subscription URLs"</p><h2 id="export-title">"复制订阅链接"</h2></div><button node_ref=close_button class="close" type="button" on:click=move |_| close() aria-label="关闭">"×"</button></div>
                <p class="export-hint">"复制本站生成的客户端订阅链接。客户端请求本服务的 /subscription 接口，由服务端聚合节点后返回；通用分享链接支持 VLESS 等协议，SIP002 / SIP008 仅包含 Shadowsocks 节点。"</p>
                <div class="export-limits" aria-label="订阅链接筛选">
                    <label>"总数 limit"<input class="control" type="number" min="1" inputmode="numeric" placeholder="不限" prop:value=total_limit on:input=move |event| set_total_limit.set(event_target_value(&event)) /></label>
                    <label>"按地区 limit"<input class="control" type="number" min="1" inputmode="numeric" placeholder="不限" prop:value=region_limit on:input=move |event| set_region_limit.set(event_target_value(&event)) /></label>
                    <label class="check-row"><input type="checkbox" prop:checked=exclude_dead on:change=move |event| set_exclude_dead.set(event_target_checked(&event)) />"过滤死节点"</label>
                    <div class="export-provider-field">
                        <span>"provider"</span>
                        <details class="export-provider-select">
                            <summary><span>{move || if selected_providers.get().is_empty() { "全部 provider".into() } else { format!("已选 {} 个", selected_providers.get().len()) }}</span><span aria-hidden="true">"⌄"</span></summary>
                            <div class="export-provider-menu">
                                {move || {
                                    let names = provider_names.get();
                                    if names.is_empty() {
                                        view! { <span class="muted export-provider-empty">"暂无 provider"</span> }.into_any()
                                    } else {
                                        names.into_iter().map(|provider| {
                                            let checked_name = provider.clone();
                                            let event_name = provider.clone();
                                            view! {
                                                <label class="export-provider-option">
                                                    <input type="checkbox" prop:checked=move || selected_providers.get().contains(&checked_name) on:change=move |event| {
                                                        let name = event_name.clone();
                                                        if event_target_checked(&event) {
                                                            set_selected_providers.update(|values| if !values.contains(&name) { values.push(name); });
                                                        } else {
                                                            set_selected_providers.update(|values| values.retain(|value| value != &name));
                                                        }
                                                    } />
                                                    <span>{provider}</span>
                                                </label>
                                            }
                                        }).collect_view().into_any()
                                    }
                                }}
                            </div>
                        </details>
                    </div>
                </div>
                <p class="export-ranking-hint">"填写任一 limit 后，优先选择拨测成功、稳定率高、延迟低且解锁状态更好的节点；启用“过滤死节点”时仅排除最近一次 "<span class="nowrap">"latency 检测失败"</span>"的节点；未拨测节点会保留。"</p>
                <div class="export-options">
                    {EXPORT_OPTIONS.iter().map(|option| {
                        let format = option.format;
                        let link = Signal::derive(move || {
                            subscription_url(&export_href(
                                format,
                                &total_limit.get(),
                                &region_limit.get(),
                                exclude_dead.get(),
                                &selected_providers.get(),
                            ))
                        });
                        view! {
                            <button class="export-option" type="button" on:click=move |_| {
                                let link = link.get();
                                spawn_local(async move {
                                    set_copy_status.set(Some((format, copy_text(&link).await)));
                                });
                            } aria-label=format!("复制 {} 订阅链接", option.label)>
                                <span class="export-option-main"><strong>{option.label}</strong><small>{option.description}</small></span>
                                <span class="export-option-action">{move || match copy_status.get() {
                                    Some((copied_format, true)) if copied_format == format => "已复制",
                                    Some((copied_format, false)) if copied_format == format => "复制失败",
                                    _ => "复制链接",
                                }}</span>
                            </button>
                        }
                    }).collect_view()}
                </div>
                <div class="modal-actions"><button class="ghost" type="button" on:click=move |_| close()>"取消"</button></div>
            </section>
        </div>
    }
}

fn subscription_url(path: &str) -> String {
    web_sys::window()
        .and_then(|window| window.location().origin().ok())
        .filter(|origin| !origin.is_empty())
        .map_or_else(|| path.to_string(), |origin| format!("{origin}{path}"))
}

async fn copy_text(value: &str) -> bool {
    let Some(window) = web_sys::window() else {
        return false;
    };
    if window.is_secure_context()
        && wasm_bindgen_futures::JsFuture::from(window.navigator().clipboard().write_text(value))
            .await
            .is_ok()
    {
        return true;
    }

    let Some(document) = window.document() else {
        return false;
    };
    let Ok(textarea) = document
        .create_element("textarea")
        .and_then(|element| element.dyn_into::<web_sys::HtmlTextAreaElement>().map_err(Into::into))
    else {
        return false;
    };
    textarea.set_value(value);
    let _ = textarea.set_attribute("readonly", "true");
    let _ = textarea.set_attribute("aria-hidden", "true");
    let _ = textarea.set_attribute(
        "style",
        "position:fixed;top:-9999px;left:-9999px;opacity:0;",
    );
    let Some(body) = document.body() else {
        return false;
    };
    if body.append_child(&textarea).is_err() {
        return false;
    }
    textarea.select();
    let copied = document
        .dyn_into::<web_sys::HtmlDocument>()
        .ok()
        .and_then(|document| document.exec_command("copy").ok())
        .unwrap_or(false);
    textarea.remove();
    copied
}

fn export_href(
    format: &str,
    total: &str,
    region: &str,
    exclude_dead: bool,
    providers: &[String],
) -> String {
    let mut query = vec![format!("format={format}")];
    for (name, value) in [("limit", total), ("region_limit", region)] {
        if value.trim().parse::<usize>().is_ok_and(|limit| limit > 0) {
            query.push(format!("{name}={}", value.trim()));
        }
    }
    if exclude_dead {
        query.push("exclude_dead=1".into());
    }
    query.extend(
        providers
            .iter()
            .filter_map(|provider| {
                (!provider.trim().is_empty())
                    .then(|| format!("provider={}", encode_query_value(provider.trim())))
            }),
    );
    format!("/subscription?{}", query.join("&"))
}

fn encode_query_value(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push('%');
            encoded.push(HEX[(byte >> 4) as usize] as char);
            encoded.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    encoded
}

#[derive(Clone)]
struct NodeGroup {
    provider: String,
    region: String,
    nodes: Vec<Node>,
    healthy: usize,
    unhealthy: usize,
    unprobed: usize,
    protocols: Vec<String>,
}

#[component]
fn NodeGroupCard(
    group: NodeGroup,
    selected_node: ReadSignal<Option<String>>,
    set_selected_node: WriteSignal<Option<String>>,
    probing_node: ReadSignal<Option<String>>,
) -> impl IntoView {
    let flag = region_flag(&group.region);
    let provider = group.provider.clone();
    let region = group.region.clone();
    let total = group.nodes.len();
    let alive = group.healthy;
    let protocols = group.protocols.join(" / ");
    let latency = median_latency(&group.nodes);
    let health_label = format!(
        "存活 {alive}/{total}；不可用 {}；未探测 {}",
        group.unhealthy, group.unprobed
    );
    let nodes = group.nodes.into_iter().map(|node| {
        let (status, status_label) = match node.last_probe_ok {
            Some(true) => ("status-good", "健康"),
            Some(false) => ("status-bad", "不可用"),
            None => ("status-pending", "未探测"),
        };
        let latency = node
            .latency_ms
            .map_or_else(|| "—".into(), |value| format!("{value} ms"));
        let selected_id = node.id.clone();
        let selected_for_class = {
            let selected_id = selected_id.clone();
            move || selected_node.get().as_deref() == Some(selected_id.as_str())
        };
        let selected_for_aria = {
            let selected_id = selected_id.clone();
            move || selected_node.get().as_deref() == Some(selected_id.as_str())
        };
        let node_id = selected_id.clone();
        let probing_for_class = {
            let selected_id = selected_id.clone();
            move || probing_node.get().as_deref() == Some(selected_id.as_str())
        };
        view! {
            <button class="node-chip" class:node-chip-selected=selected_for_class class:node-chip-probing=probing_for_class type="button" aria-pressed=selected_for_aria on:click=move |_| set_selected_node.set(Some(node_id.clone()))>
                <span class="node-chip-main"><span class=format!("status-dot {status}") role="img" aria-label=status_label title=status_label></span><strong>{node.name}</strong></span>
                <span class="node-chip-latency">{latency}</span>
                <small>{node.protocol}</small>
            </button>
        }
    }).collect_view();
    view! {
        <article class="node-group">
            <header class="node-group-head">
                <div class="node-group-title"><span class="group-flag" aria-hidden="true">{flag}</span><div><strong>{provider}{" · "}{region}</strong><span class="group-meta">{protocols}{" · "}{alive}/{total}</span></div></div>
                <div class="node-group-status"><span class="group-health" aria-label=health_label.clone() title=health_label>{alive}/{total}</span><span class="group-latency">{latency}</span></div>
            </header>
            <div class="node-chip-list">{nodes}</div>
        </article>
    }
}

fn node_groups(nodes: &[Node], sort_by: &str) -> Vec<NodeGroup> {
    let mut grouped = BTreeMap::<(String, String), Vec<Node>>::new();
    for node in nodes {
        grouped
            .entry((node_provider(node).to_owned(), node_region(node).to_owned()))
            .or_default()
            .push(node.clone());
    }
    let mut groups = grouped
        .into_iter()
        .map(|((provider, region), nodes)| {
            let healthy = nodes
                .iter()
                .filter(|node| node.last_probe_ok == Some(true))
                .count();
            let unhealthy = nodes
                .iter()
                .filter(|node| node.last_probe_ok == Some(false))
                .count();
            let unprobed = nodes.iter().filter(|node| node.last_probe_ok.is_none()).count();
            let protocols = nodes
                .iter()
                .map(|node| node.protocol.as_str())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .map(str::to_owned)
                .collect();
            NodeGroup {
                provider,
                region,
                nodes,
                healthy,
                unhealthy,
                unprobed,
                protocols,
            }
        })
        .collect::<Vec<_>>();
    groups.sort_by(|left, right| {
        compare_nodes(&left.nodes[0], &right.nodes[0], sort_by)
            .then_with(|| left.provider.cmp(&right.provider))
            .then_with(|| left.region.cmp(&right.region))
    });
    groups
}

fn node_provider(node: &Node) -> &str {
    node.sources.first().map_or("手动节点", String::as_str)
}

fn region_flag(region: &str) -> String {
    let region = region.to_ascii_lowercase();
    let bytes = region.as_bytes();
    if bytes.len() != 2 || !bytes.iter().all(u8::is_ascii_lowercase) {
        return "•".into();
    }
    bytes
        .iter()
        .map(|byte| char::from_u32(0x1f1e6 + u32::from(*byte - b'a')).map_or('•', |value| value))
        .collect()
}

fn node_region(node: &Node) -> &str {
    node.region.as_deref().unwrap_or("未分区")
}

fn node_regions(nodes: &[Node]) -> Vec<String> {
    nodes
        .iter()
        .map(node_region)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(str::to_owned)
        .collect()
}

fn node_providers(nodes: &[Node]) -> Vec<String> {
    nodes
        .iter()
        .flat_map(|node| node.sources.iter().map(String::as_str))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(str::to_owned)
        .collect()
}

fn node_protocols(nodes: &[Node]) -> Vec<String> {
    nodes
        .iter()
        .map(|node| node.protocol.as_str())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(str::to_owned)
        .collect()
}

fn compare_nodes(left: &Node, right: &Node, sort_by: &str) -> Ordering {
    let ordering = match sort_by {
        "region" => node_region(left).cmp(node_region(right)),
        "provider" => left.sources.first().cmp(&right.sources.first()),
        "latency" => match (left.latency_ms, right.latency_ms) {
            (Some(left), Some(right)) => left.cmp(&right),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        },
        "protocol" => left.protocol.cmp(&right.protocol),
        _ => Ordering::Equal,
    };
    ordering.then_with(|| left.name.cmp(&right.name))
}

#[component]
fn ManualNodeDialog(
    light: Signal<bool>,
    close: impl Fn() + Send + Sync + 'static + Copy,
    reload: impl Fn() + Send + Sync + 'static + Copy,
    set_message: WriteSignal<String>,
    set_message_error: WriteSignal<bool>,
) -> impl IntoView {
    let (share_link, set_share_link) = signal(String::new());
    let (protocol, set_protocol) = signal("ss".to_string());
    let (name, set_name) = signal(String::new());
    let (region, set_region) = signal(String::new());
    let (host, set_host) = signal(String::new());
    let (port, set_port) = signal("443".to_string());
    let (username, set_username) = signal(String::new());
    let (password, set_password) = signal(String::new());
    let (encryption, set_encryption) = signal("aes-128-gcm".to_string());
    let (transport, set_transport) = signal("tcp".to_string());
    let (tls, set_tls) = signal(false);
    let (first_focused, set_first_focused) = signal(true);
    let first_field = NodeRef::<leptos::html::Textarea>::new();
    let close_button = NodeRef::<leptos::html::Button>::new();
    let last_field = NodeRef::<leptos::html::Button>::new();
    first_field.on_load(|field| {
        let _ = field.focus();
    });
    let focus_first = move || {
        if let Some(field) = first_field.get() {
            let _ = field.focus();
        }
    };
    let focus_close = move || {
        if let Some(field) = close_button.get() {
            let _ = field.focus();
        }
    };
    let focus_last = move || {
        if let Some(field) = last_field.get() {
            let _ = field.focus();
        }
    };

    let submit = move |event: SubmitEvent| {
        event.prevent_default();
        let share_link_value = share_link.get();
        let config = json!({
            "protocol": protocol.get(), "name": name.get(), "host": host.get(),
            "address": host.get(), "port": port.get().parse::<u16>().unwrap_or(443),
            "username": username.get(), "password": password.get(),
            "encryption": encryption.get(), "transport": transport.get(), "tls": tls.get()
        });
        let variables = json!({"input": {
            "shareLink": if share_link_value.trim().is_empty() { Value::Null } else { Value::String(share_link_value) },
            "config": config, "name": if name.get().trim().is_empty() { Value::Null } else { Value::String(name.get()) },
            "region": if region.get().trim().is_empty() { Value::Null } else { Value::String(region.get()) }
        }});
        let close = close;
        let reload = reload;
        spawn_local(async move {
            let result = graphql::<Value>("mutation Add($input: AddNodeInput!) { addNode(input: $input) { id } }", variables).await;
            match result {
                Ok(_) => {
                    set_message_error.set(false);
                    set_message.set("手动节点已保存".into());
                    close();
                    reload();
                }
                Err(error) => {
                    set_message_error.set(true);
                    set_message.set(error);
                }
            }
        });
    };

    let change_protocol = move |event| {
        let protocol_value = event_target_value(&event);
        let (encryption_value, tls_value) = manual_protocol_defaults(&protocol_value);
        set_protocol.set(protocol_value);
        set_encryption.set(encryption_value.into());
        set_tls.set(tls_value);
    };

    view! {
        <div class="modal-backdrop" class:theme-light=light role="presentation" on:click=move |_| close()>
            <section class="modal" role="dialog" aria-modal="true" aria-labelledby="manual-node-title" on:keydown=move |event| { if event.key() == "Escape" { event.prevent_default(); close(); } } on:click=|event| event.stop_propagation()>
                <div class="modal-head"><div><p class="eyebrow">"Manual node"</p><h2 id="manual-node-title">"手动输入节点"</h2></div><button node_ref=close_button class="close" type="button" on:keydown=move |event| { if event.key() == "Tab" { event.prevent_default(); if event.shift_key() { focus_last(); } else { focus_first(); } } } on:click=move |_| close() aria-label="关闭">"×"</button></div>
                <form class="form" on:submit=submit>
                    <label>"分享链接（可选）"<textarea class:focus-ring=first_focused on:focus=move |_| set_first_focused.set(true) on:blur=move |_| set_first_focused.set(false) node_ref=first_field on:keydown=move |event| { if event.key() == "Tab" && event.shift_key() { event.prevent_default(); focus_close(); } } rows="3" placeholder="ss://、vmess://、vless://、trojan://…" prop:value=share_link on:input=move |event| set_share_link.set(event_target_value(&event))></textarea></label>
                    <div class="hint">"填写分享链接时，结构化字段只作为默认值；不填链接即可使用下方参数创建节点。"</div>
                    <div class="field-grid"><label>"协议"<select prop:value=protocol on:change=change_protocol><option value="ss">"ss"</option><option value="trojan">"trojan"</option><option value="vmess">"vmess"</option><option value="vless">"vless"</option><option value="hysteria2">"hysteria2"</option><option value="tuic">"tuic"</option><option value="juicity">"juicity"</option><option value="anytls">"anytls"</option><option value="socks5">"socks5"</option></select></label><label>"端口"<input type="number" min="1" max="65535" prop:value=port on:input=move |event| set_port.set(event_target_value(&event)) /></label></div>
                    <div class="field-grid"><label>"地址 / 主机"<input prop:value=host on:input=move |event| set_host.set(event_target_value(&event)) placeholder="example.com" /></label><label>"地区（可选）"<input prop:value=region on:input=move |event| set_region.set(event_target_value(&event)) placeholder="JP / US / HK" /></label></div>
                    <div class="field-grid"><label>"用户名（可选）"<input prop:value=username on:input=move |event| set_username.set(event_target_value(&event)) /></label><label>"密码 / UUID（可选）"<input type="password" prop:value=password on:input=move |event| set_password.set(event_target_value(&event)) /></label></div>
                    <div class="field-grid"><label>"加密（协议默认）"<input prop:value=encryption on:input=move |event| set_encryption.set(event_target_value(&event)) /></label><label>"传输（默认 tcp）"<select prop:value=transport on:change=move |event| set_transport.set(event_target_value(&event))><option value="tcp">"tcp"</option><option value="ws">"ws"</option><option value="grpc">"grpc"</option></select></label></div>
                    <label class="check-row"><input type="checkbox" prop:checked=tls on:change=move |event| set_tls.set(event_target_checked(&event)) />"启用 TLS"</label>
                    <label>"显示名称（可选）"<input prop:value=name on:input=move |event| set_name.set(event_target_value(&event)) placeholder="留空使用协议和地址" /></label>
                    <div class="modal-actions"><button class="ghost" type="button" on:click=move |_| close()>"取消"</button><button node_ref=last_field class="primary" type="submit" on:keydown=move |event| { if event.key() == "Tab" && !event.shift_key() { event.prevent_default(); focus_close(); } }>"保存节点"</button></div>
                </form>
            </section>
        </div>
    }
}

fn manual_protocol_defaults(protocol: &str) -> (&'static str, bool) {
    match protocol {
        "ss" => ("aes-128-gcm", false),
        "trojan" | "vless" | "anytls" => ("", true),
        _ => ("", false),
    }
}

fn median_latency(nodes: &[Node]) -> String {
    let mut values = nodes.iter().filter_map(|node| node.latency_ms).collect::<Vec<_>>();
    values.sort_unstable();
    values
        .get(values.len() / 2)
        .map_or_else(|| "—".into(), |value| format!("{value} ms"))
}

fn main() {
    mount_to_body(App);
}

#[cfg(test)]
mod tests {
    use super::{Node, export_href, node_groups};

    fn node(id: &str, provider: &str, region: &str) -> Node {
        Node {
            id: id.into(),
            name: id.into(),
            protocol: "anytls".into(),
            address: "example.com:443".into(),
            region: Some(region.into()),
            sources: vec![provider.into()],
            latency_ms: Some(50),
            last_probe_ok: Some(true),
        }
    }

    #[test]
    fn dashboard_nodes_decode_probe_fields() {
        let node: Node = serde_json::from_value(serde_json::json!({
            "id": "node-1",
            "name": "🇭🇰 yss.hk.01",
            "protocol": "anytls",
            "address": "example.com:443",
            "region": null,
            "sources": ["yss"],
            "latencyMs": 72,
            "lastProbeOk": true
        }))
        .expect("dashboard node should decode");
        assert_eq!(node.latency_ms, Some(72));
        assert_eq!(node.last_probe_ok, Some(true));
    }

    #[test]
    fn node_groups_by_provider_and_region() {
        let groups = node_groups(
            &[
                node("hk-1", "yss", "hk"),
                node("hk-2", "yss", "hk"),
                node("de-1", "kad", "de"),
            ],
            "default",
        );
        assert_eq!(groups.len(), 2);
        let hk = groups
            .iter()
            .find(|group| group.provider == "yss" && group.region == "hk")
            .expect("hk group should exist");
        assert_eq!(hk.nodes.len(), 2);
        assert_eq!(hk.healthy, 2);
    }

    #[test]
    fn export_href_includes_only_positive_limits() {
        assert_eq!(
            export_href("clash", "10", "2", false, &["香港 / yss".into()]),
            "/subscription?format=clash&limit=10&region_limit=2&provider=%E9%A6%99%E6%B8%AF%20%2F%20yss"
        );
    }

    #[test]
    fn export_href_can_request_dead_node_filter() {
        assert_eq!(
            export_href("clash", "", "", true, &[]),
            "/subscription?format=clash&exclude_dead=1"
        );
    }
}
