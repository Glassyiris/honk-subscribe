use leptos::{ev::SubmitEvent, prelude::*, task::spawn_local};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::graphql;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Provider {
    pub id: String,
    pub name: String,
    pub url: String,
    pub sub_type: String,
    pub update_interval: i64,
    pub enabled: bool,
    pub user_agent: Option<String>,
    pub rename_template: Option<String>,
    pub filter: ProviderFilter,
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

#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct ProviderFilter {
    pub include: Option<String>,
    pub exclude: Option<String>,
    pub protocol: Option<String>,
}

#[component]
pub(crate) fn ProviderCard(
    provider: Provider,
    on_refresh: impl Fn(String) + Copy + Send + Sync + 'static,
    on_edit: impl Fn(Provider) + Copy + Send + Sync + 'static,
    on_rename: impl Fn(String) + Copy + Send + Sync + 'static,
    on_probe: impl Fn(String) + Copy + Send + Sync + 'static,
    on_delete: impl Fn(String) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let refresh_id = provider.id.clone();
    let edit_provider = StoredValue::new(provider.clone());
    let rename_id = StoredValue::new(provider.id.clone());
    let probe_name = StoredValue::new(provider.name.clone());
    let delete_id = StoredValue::new(provider.id.clone());
    let provider_url = StoredValue::new(provider.url.clone());
    let (url_visible, set_url_visible) = signal(false);
    let (confirming, set_confirming) = signal(false);
    let status = if provider.enabled { "status" } else { "badge" };
    let node_status = if provider.node_count == 0 {
        "—".into()
    } else if provider.probed_nodes == 0 {
        "未探测".into()
    } else {
        format!("{}/{}", provider.alive_nodes, provider.node_count)
    };
    let node_kinds = if provider.node_kinds.is_empty() {
        "未导入".into()
    } else {
        provider.node_kinds.join(" · ")
    };
    let traffic_status = format_bytes(provider.traffic_remaining_bytes);
    let traffic_detail = match (provider.traffic_used_bytes, provider.traffic_total_bytes) {
        (Some(used), Some(total)) => format!("已用 {} / {}", format_bytes(Some(used)), format_bytes(Some(total))),
        _ => "上游未提供".into(),
    };
    let rule_summary = format_rule_summary(&provider);
    view! {
        <article class="provider-card">
            <div class="provider-card-main">
                <div class="provider-card-head"><strong>{provider.name}</strong><span class="badge">{provider.sub_type}</span></div>
                <div class="provider-url-row">
                    <span class="provider-url" aria-live="polite">{move || if url_visible.get() { provider_url.with_value(Clone::clone) } else { "****".into() }}</span>
                    <button class="provider-url-toggle" type="button" on:click=move |_| set_url_visible.update(|visible| *visible = !*visible) aria-label=move || if url_visible.get() { "隐藏订阅链接" } else { "显示订阅链接" } aria-pressed=move || url_visible.get().to_string() title=move || if url_visible.get() { "隐藏订阅链接" } else { "显示订阅链接" }>
                        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M2.5 12s3.5-6 9.5-6 9.5 6 9.5 6-3.5 6-9.5 6-9.5-6-9.5-6Z"/><circle cx="12" cy="12" r="2.5"/></svg>
                    </button>
                </div>
                <div class="provider-meta"><span>{format!("类型 {node_kinds}")}</span><span>{format!("存活 {node_status}")}</span><span>{format!("剩余 {traffic_status}")}</span><span>{format!("每 {} 秒", provider.update_interval)}</span><span class=status>{if provider.enabled { "已启用" } else { "已停用" }}</span></div>
                <small class="muted">{format!("{} · {} · {}", provider.last_updated.map_or_else(|| "尚未刷新".into(), |value| format!("最后刷新 {value}")), traffic_detail, provider.traffic_expire_at.map_or_else(|| "无过期信息".into(), |value| format!("到期 {value}")))}</small>
                <small class="muted provider-rule">{rule_summary}</small>
            </div>
            <div class="provider-actions">
                <button class="ghost" type="button" on:click=move |_| on_refresh(refresh_id.clone())>"刷新"</button>
                <button class="ghost" type="button" on:click=move |_| on_probe(probe_name.with_value(Clone::clone))>"拨测"</button>
                <button class="ghost" type="button" on:click=move |_| on_edit(edit_provider.with_value(Clone::clone))>"编辑"</button>
                <button class="ghost" type="button" on:click=move |_| on_rename(rename_id.with_value(Clone::clone))>"重命名"</button>
                <Show when=move || confirming.get() fallback=move || view! { <button class="danger" type="button" on:click=move |_| set_confirming.set(true)>"删除"</button> }>
                    <button class="ghost" type="button" on:click=move |_| set_confirming.set(false)>"取消"</button>
                    <button class="danger" type="button" on:click=move |_| on_delete(delete_id.with_value(Clone::clone))>"确认删除"</button>
                </Show>
            </div>
        </article>
    }
}

fn format_rule_summary(provider: &Provider) -> String {
    let rename = provider.rename_template.as_deref().unwrap_or("不改名");
    let filter = [
        provider.filter.include.as_deref().map(|value| format!("含 {value}")),
        provider.filter.exclude.as_deref().map(|value| format!("排除 {value}")),
        provider.filter.protocol.as_deref().map(|value| format!("协议 {value}")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    let probe = if provider.probe_vless_modes {
        "VLESS UDP 已探测"
    } else {
        "VLESS UDP 未探测"
    };
    format!(
        "重命名 {rename} · 过滤 {} · {probe}",
        if filter.is_empty() {
            "无".into()
        } else {
            filter.join(" / ")
        }
    )
}

fn format_bytes(value: Option<i64>) -> String {
    let Some(value) = value.filter(|value| *value >= 0) else {
        return "未提供".into();
    };
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut size = value as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < units.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{value} {}", units[unit])
    } else {
        format!("{size:.1} {}", units[unit])
    }
}

#[component]
pub(crate) fn ProviderDialog(
    provider: Signal<Option<Provider>>,
    light: Signal<bool>,
    close: impl Fn() + Copy + Send + Sync + 'static,
    reload: impl Fn() + Copy + Send + Sync + 'static,
    set_message: WriteSignal<String>,
    set_message_error: WriteSignal<bool>,
) -> impl IntoView {
    let initial = provider.get_untracked();
    let editing_id = initial.as_ref().map(|value| value.id.clone());
    let editing = editing_id.is_some();
    let (name, set_name) = signal(initial.as_ref().map_or_else(String::new, |value| value.name.clone()));
    let (url, set_url) = signal(initial.as_ref().map_or_else(String::new, |value| value.url.clone()));
    let (sub_type, set_sub_type) = signal(initial.as_ref().map_or_else(|| "custom".into(), |value| value.sub_type.clone()));
    let (update_interval, set_update_interval) = signal(initial.as_ref().map_or_else(|| "86400".into(), |value| value.update_interval.to_string()));
    let (user_agent, set_user_agent) = signal(initial.as_ref().and_then(|value| value.user_agent.clone()).unwrap_or_default());
    let (enabled, set_enabled) = signal(initial.as_ref().is_none_or(|value| value.enabled));
    let (rename_template, set_rename_template) = signal(initial.as_ref().and_then(|value| value.rename_template.clone()).unwrap_or_default());
    let (filter_include, set_filter_include) = signal(initial.as_ref().and_then(|value| value.filter.include.clone()).unwrap_or_default());
    let (filter_exclude, set_filter_exclude) = signal(initial.as_ref().and_then(|value| value.filter.exclude.clone()).unwrap_or_default());
    let (filter_protocol, set_filter_protocol) = signal(initial.as_ref().and_then(|value| value.filter.protocol.clone()).unwrap_or_default());
    let (probe_vless_modes, set_probe_vless_modes) = signal(initial.as_ref().is_some_and(|value| value.probe_vless_modes));
    let first_field = NodeRef::<leptos::html::Input>::new();
    let close_button = NodeRef::<leptos::html::Button>::new();
    first_field.on_load(|field| {
        let _ = field.focus();
    });

    let submit = move |event: SubmitEvent| {
        event.prevent_default();
        let edit_id = editing_id.clone();
        let is_editing = edit_id.is_some();
        let variables = json!({"input": {
            "name": name.get(),
            "url": url.get(),
            "subType": sub_type.get(),
            "updateInterval": update_interval.get().parse::<i64>().unwrap_or(86400),
            "enabled": enabled.get(),
            "userAgent": if user_agent.get().trim().is_empty() { Value::Null } else { Value::String(user_agent.get()) },
            "headers": Value::Null,
            "renameTemplate": if rename_template.get().trim().is_empty() { Value::Null } else { Value::String(rename_template.get()) },
            "probeVlessModes": probe_vless_modes.get(),
            "filter": {"include": optional_value(filter_include.get()), "exclude": optional_value(filter_exclude.get()), "protocol": optional_value(filter_protocol.get())}
        }});
        let close = close;
        let reload = reload;
        let mutation = if is_editing {
            "mutation UpdateProvider($input: UpdateSubscriptionInput!) { updateSubscription(input: $input) { id } }"
        } else {
            "mutation AddProvider($input: AddSubscriptionInput!) { addSubscription(input: $input) { id } }"
        };
        let variables = if let Some(id) = edit_id {
            let mut variables = variables;
            variables["input"]["id"] = Value::String(id);
            variables
        } else {
            variables
        };
        spawn_local(async move {
            let result = graphql::<Value>(mutation, variables).await;
            match result {
                Ok(_) => {
                    set_message_error.set(false);
                    set_message.set(if is_editing { "provider 配置已保存" } else { "provider 已添加" }.into());
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

    view! {
        <div class="modal-backdrop" class:theme-light=light role="presentation" on:click=move |_| close()>
            <section class="modal" role="dialog" aria-modal="true" aria-labelledby="provider-title" on:keydown=move |event| { if event.key() == "Escape" { event.prevent_default(); close(); } } on:click=|event| event.stop_propagation()>
                <div class="modal-head"><div><p class="eyebrow">"Provider"</p><h2 id="provider-title">{if editing { "编辑订阅源" } else { "添加订阅源" }}</h2></div><button node_ref=close_button class="close" type="button" on:click=move |_| close() aria-label="关闭">"×"</button></div>
                <form class="form" on:submit=submit>
                    <label>"名称"<input node_ref=first_field required prop:value=name on:input=move |event| set_name.set(event_target_value(&event)) placeholder="例如：主力线路" /></label>
                    <label>"订阅地址"<input required type="url" prop:value=url on:input=move |event| set_url.set(event_target_value(&event)) placeholder="https://example.com/subscribe" /></label>
                    <div class="field-grid"><label>"类型"<select prop:value=sub_type on:change=move |event| set_sub_type.set(event_target_value(&event))><option value="custom">"custom（自动识别）"</option><option value="clash">"clash"</option><option value="simple">"simple"</option><option value="sip008">"sip008"</option></select></label><label>"更新间隔（秒）"<input type="number" min="0" prop:value=update_interval on:input=move |event| set_update_interval.set(event_target_value(&event)) /></label></div>
                    <label>"User-Agent（可选）"<input prop:value=user_agent on:input=move |event| set_user_agent.set(event_target_value(&event)) placeholder="留空使用默认请求头" /></label>
                    <label class="check-row"><input type="checkbox" prop:checked=enabled on:change=move |event| set_enabled.set(event_target_checked(&event)) />"启用此 provider"</label>
                    <label class="check-row"><input type="checkbox" prop:checked=probe_vless_modes on:change=move |event| set_probe_vless_modes.set(event_target_checked(&event)) />"刷新时探测 VLESS UDP mode"</label>
                    <label>"重命名模板（可选）"<input prop:value=rename_template on:input=move |event| set_rename_template.set(event_target_value(&event)) placeholder="例如：{flag} {provider}.{region}.{index}" /></label>
                    <div class="field-grid"><label>"包含关键字（可选）"<input prop:value=filter_include on:input=move |event| set_filter_include.set(event_target_value(&event)) placeholder="例如：香港 / Tokyo" /></label><label>"排除关键字（可选）"<input prop:value=filter_exclude on:input=move |event| set_filter_exclude.set(event_target_value(&event)) placeholder="例如：过期 / 低倍率" /></label></div>
                    <label>"协议过滤（可选）"<select prop:value=filter_protocol on:change=move |event| set_filter_protocol.set(event_target_value(&event))><option value="">"全部协议"</option><option value="ss">"ss"</option><option value="trojan">"trojan"</option><option value="vmess">"vmess"</option><option value="vless">"vless"</option><option value="hysteria2">"hysteria2"</option><option value="tuic">"tuic"</option><option value="socks5">"socks5"</option></select></label>
                    <div class="hint">"模板支持 {flag}、{provider}、{region}、{index}、{name}；index 为两位序号。过滤在刷新导入前执行；开启后所有 VLESS 按 XUDP 优先，通过 cp.cloudflare.com:443 的 QUIC 握手探测可用 UDP mode。"</div>
                    <div class="modal-actions"><button class="ghost" type="button" on:click=move |_| close()>"取消"</button><button class="primary" type="submit">{if editing { "保存修改" } else { "保存订阅" }}</button></div>
                </form>
            </section>
        </div>
    }
}

fn optional_value(value: String) -> Value {
    if value.trim().is_empty() {
        Value::Null
    } else {
        Value::String(value)
    }
}
