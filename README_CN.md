# honk-subsribe

一个以 honk crate 为协议与节点模型基础的订阅聚合控制面板。项目本身不复制 honk 源码，而是通过 Cargo Git 依赖固定到同一个 revision：

- `honk-config`：分享链接、节点身份、订阅模型和配置解析。
- `honk-core` / `honk-outbound`：可选的 Linux 出站拨测能力。

## 当前能力

- 聚合 Clash YAML、Base64 分享链接、SIP008 和自定义自动识别订阅。
- 手动录入 `ss://`、`vmess://`、`vless://`、`trojan://` 等 honk 支持的分享链接。
- 基于稳定节点 ID 的批量重命名，默认输出 `🇭🇰 yss.hk.01`，支持 `{flag}`、`{provider}`、`{region}`、`{index}`、`{name}` 模板变量。
- SQLite 保存订阅、节点来源、节点配置和拨测历史。
- GraphQL 查询节点库存、延迟、稳定性、AI/Netflix 探测状态；支持添加订阅、刷新、手动添加、批量重命名、批量拨测。
- 独立订阅管理页：添加/删除订阅，查看每个 provider 的节点协议种类、探测存活数、刷新时间和剩余流量。
- provider 支持编辑订阅参数、刷新前过滤（包含/排除关键字、协议）和自定义重命名模板；模板支持 `{flag}`、`{provider}`、`{region}`、`{index}`、`{name}`。
- 暗色运维面板：节点表格、地区/协议/延迟/稳定性/解锁状态过滤器、批量操作。
- Leptos CSR 前端，提供独立的手动节点输入弹窗；分享链接和结构化节点参数均可选。
- 订阅输出可按客户端选择：`/subscription?format=sip002`（包含 SIP003 插件参数）、`/subscription?format=sip008`、`/subscription?format=clash`、`/subscription?format=sing-box`、`/subscription?format=surge`、`/subscription?format=dae`，省略格式时默认 Clash；这些是本站聚合接口，客户端请求后由服务端统一筛选、排序并返回节点。节点页的“订阅链接”按钮会复制本站生成的完整订阅 URL。链接支持总数 `limit`、地区 `region_limit`、重复的 `provider` 参数和 `exclude_dead=1`；选中 provider 后只输出对应节点，启用数值限制时按拨测成功、稳定率、延迟和解锁状态综合排序再截断，启用 `exclude_dead=1` 时排除最近一次 latency 拨测失败的节点，未拨测节点保留。
- 启用 provider 的 VLESS mode 探测后，使用 `cp.cloudflare.com:443` 的 QUIC 探测按 `mux-cool`、`h2mux`、`h2mux-padded`、`xudp`、`uot-v2` 顺序选择首个成功能力；全部 UDP mode 失败时保留 `legacy` 作为 TCP/兼容回退，不把它当作 UDP 能力证明。

Clash 和 sing-box 导出会为 Trojan、AnyTLS、Hysteria2、TUIC、Juicity 保留协议必需的 TLS，即使历史节点模型的可选 `tls` 标记为 `false`。VMess/VLESS 的可选 TLS 和 REALITY 设置、现有认证及证书校验选项保持不变；无需重写数据库中的节点或 ID。

## 运行

```bash
(cd web && trunk build --release)
cargo run
```

`AddNodeInput` 支持 `shareLink` 或部分 `config` JSON。结构化输入默认协议为 `ss`、端口为 `443`、名称为 `manual-node`；订阅的更新间隔默认 `86400` 秒，启用状态默认开启。

页面中的“订阅管理”对应现有订阅模型，更新间隔默认 86400 秒，User-Agent 可选；复杂请求头可通过 GraphQL 的 AddSubscriptionInput.headers 传入。编辑 provider 可以保存刷新前过滤和 `{flag}`、`{provider}`、`{region}`、`{index}`、`{name}` 重命名模板；刷新时先过滤再重命名，`{index}` 为两位序号。刷新时读取常见的 subscription-userinfo 响应头显示上传、下载、总量和到期时间；上游未提供时显示“未提供”。provider 卡片也可按节点原名识别地区，并以稳定的 `🇭🇰 yss.hk.01` 格式批量重命名节点；该操作不改变节点 ID 或订阅地址，重复执行结果一致。

打开 <http://127.0.0.1:8090/>。GraphiQL 在 <http://127.0.0.1:8090/graphiql>，健康检查在 <http://127.0.0.1:8090/health>。

环境变量：

```bash
HONK_SUBSCRIBE_ADDR=127.0.0.1:8090
HONK_SUBSCRIBE_DB=honk-subsribe.db
```

默认构建适合 macOS 等环境，但不包含代理出站拨测能力；拨测不会把节点 TCP 可达性伪装成延迟。Linux 上使用下面的 feature 启用完整拨测：

```bash
cargo run --features honk-probe
```

该 feature 会引入同一 Git revision 的 `honk-core` 和 `honk-outbound`，通过节点协议出站访问 `generate_204`、ChatGPT 与 Netflix 探测地址，延迟包含节点握手、TLS 和目标响应头往返。浏览器 HTTP 状态只能作为地区/解锁的可达性启发式，不能替代供应商或平台官方认证。

## GraphQL 示例

```graphql
query {
  nodes(filter: { maxLatencyMs: 300, minStabilityPercent: 90, aiUnlocked: true }) {
    id name protocol address latencyMs stabilityPercent aiUnlocked netflixUnlocked
  }
}
```

## 设计记录

界面研究、设计 token、响应式布局、无障碍和动效约束见 [`DESIGN.md`](DESIGN.md)。
