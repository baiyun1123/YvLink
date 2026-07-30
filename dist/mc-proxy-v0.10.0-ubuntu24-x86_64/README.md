# mc-proxy

面向 Minecraft Java 版的协议感知转发器和 Web 管理控制台。数据面解析 Handshake、Status 与 Login Start，用于域名选路、自定义 MOTD 和登录白名单；后续游戏与模组协议保持透明转发。控制面支持多后端负载均衡、故障转移、实时监控和 Geyser 基岩互通健康检查。

生产管理地址：<https://mc.lic6.top>

## 功能

- Tokio 多线程运行时，每个连接使用轻量异步任务。
- 单一 Minecraft Java 监听入口，所有路由共用同一个 IP 和端口。
- 按握手 virtual host 顺序匹配 `host → backend`，支持多个 host、`*`/`?` 通配和末尾 `host = "*"` 兜底。
- 每条路由支持最多 128 个后端，以及顺序、随机、轮询、最少连接和最低连接延迟策略；首选后端失败时自动尝试池内其余后端。
- 可选主动 TCP 健康检查，支持检查间隔、连接超时、连续失败/恢复阈值；健康节点优先，全部离线时仍保留真实连接降级尝试。
- 每条路由可选 PROXY Protocol v1/v2，在 Minecraft 首包前向明确兼容的受信任后端传递真实客户端与入口地址；默认关闭。
- 每条路由支持完全自定义状态，或读取后端 Status 后只覆盖指定字段；后端模式保留 Forge/NeoForge 扩展、favicon、玩家 sample 与未知 JSON 字段。
- 后端状态按“backend + 客户端协议号”缓存，并可在后端离线时返回可配置 fallback。
- 状态查询使用同一后端池与故障转移策略，状态缓存按具体 backend 和客户端协议号隔离。
- 每条路由可启用玩家名白名单，在 Login Start 后、连接后端前发送原版 Login Disconnect。
- 默认保留 Forge/FML 握手 Host 的 NUL 扩展；白名单只读取玩家名，允许后续 Fabric、Forge 和 NeoForge 插件消息透明通过。
- 按 Gate 兼容标记统计无标记、旧版 `FML`、现代 `FML2/FML3` 和 1.20.2+ `FORGE` 握手；Fabric 与原版因初始握手不可可靠区分而合并展示。
- 六组双向协议夹具覆盖 Fabric 1.20.1、Forge 1.12.2/1.16.5/1.20.1、NeoForge 1.21.1 与 FORGE NAT，验证 Host 改写和未知模组负载逐字节保真。
- 在线新建、修改、启停和删除规则，无需手工编辑 TOML。
- 实时统计活跃连接、累计连接、上下行字节、后端失败、转发异常、并发拒绝、白名单拒绝、本地状态响应、状态缓存命中、fallback 和模组握手分类。
- 前端每 2 秒采样状态，使用 SVG 绘制 60 秒实时吞吐曲线。
- 独立 Crossplay 页面配置 Bedrock UDP/Geyser 目标与认证方式，并通过真实 RakNet Pong 区分“已配置”和“翻译器在线”。
- 前端图标全部为内联 SVG，不使用 Emoji，不依赖 CDN。
- 管理 API 使用 Bearer Token，令牌只保存在浏览器 `sessionStorage`。
- 管理 HTTP 只监听 `127.0.0.1:18080`，公网由 Nginx HTTPS 反向代理。
- Nginx API 限速、安全响应头、Let’s Encrypt 自动续期。
- 后端连接超时、连接数限制、半关闭处理和 Ctrl+C/SIGTERM 优雅退出。
- Linux/Android 可选 `SO_REUSEPORT`。

mc-proxy 数据面本身只处理 Minecraft Java TCP；基岩版通过外部 Geyser Standalone 翻译层接入。白名单是后端认证前的快速筛选，不能替代在线模式身份认证。PROXY Protocol 只传递连接地址，不转换 Java 协议版本，也不替代 Velocity/Bungee 玩家信息转发。

## 管理页面

访问：

```text
https://mc.lic6.top
```

服务器上的管理令牌位于：

```text
/etc/mc-proxy/admin.env
```

使用 root 查看：

```sh
sed -n 's/^MC_PROXY_ADMIN_TOKEN=//p' /etc/mc-proxy/admin.env
```

登录页面不会把令牌写入长期本地存储，关闭当前浏览器标签后需要重新输入。

API 文档：

```text
https://mc.lic6.top/docs/api
```

对应源码文档为 `docs/api.html`，支持手机和电脑布局以及关键词检索。

当前原版、Fabric、Forge 与 NeoForge 能力边界及后续路线见 `MODDED_COMPATIBILITY.md`。

基岩版互通架构、Geyser/Floodgate 部署与模组限制见 `CROSSPLAY.md`。

## 配置

示例结构：

```toml
[admin]
listen = "127.0.0.1:18080"

[crossplay]
enabled = false
bedrock_listen = "0.0.0.0:19132"
java_address = "bedrock.example.com"
java_port = 25565
auth_type = "online"

[settings]
listen = "0.0.0.0:25565"
proxy_enabled = true
max_connections = 10000
connect_timeout_ms = 5000
handshake_timeout_ms = 5000
shutdown_grace_secs = 30
copy_buffer_bytes = 32768
socket_buffer_bytes = 1048576
listen_backlog = 4096
tcp_nodelay = true
reuse_port = false
stats_interval_secs = 10

[[rules]]
id = "play"
name = "主服"
host = "play.example.com"
backend = "10.0.0.2:25565"
proxy_protocol = "off"
modify_virtual_host = false
whitelist_enabled = true
whitelist = ["Alice", "Bob"]
whitelist_message = "§c你不在此服务器的白名单中。"
enabled = true

[rules.health_check]
enabled = true
interval_secs = 30
timeout_ms = 2000
unhealthy_threshold = 3
healthy_threshold = 2

[rules.status]
mode = "custom"
cache_ttl_secs = 10
motd = "§a欢迎来到主服"
version_name = "原版 1.21"
# protocol 留空时跟随客户端查询协议
online = 0
max = 100

[[rules]]
id = "mod"
name = "模组服"
host = ["*.mod.example.com", "mod.example.com"]
backend = ["backend-a.example.com:25565", "backend-b.example.com:25565"]
strategy = "least-connections"
proxy_protocol = "off"
modify_virtual_host = true
whitelist_enabled = false
whitelist = []
enabled = true

[rules.health_check]
enabled = true
interval_secs = 30
timeout_ms = 2000
unhealthy_threshold = 3
healthy_threshold = 2

[rules.status]
mode = "backend"
cache_ttl_secs = 60
# 以下字段可省略；后端模式只覆盖明确填写的字段。
motd = "§b模组服在线"

[rules.status.fallback]
motd = "§c模组服暂时离线"
version_name = "后端不可用"
protocol = -1
online = 0
max = 100

[[rules]]
id = "default"
name = "默认线路"
host = "*"
backend = "10.0.0.10:25565"
proxy_protocol = "off"
modify_virtual_host = false
whitelist_enabled = false
whitelist = []
enabled = true

[rules.health_check]
enabled = false
interval_secs = 30
timeout_ms = 2000
unhealthy_threshold = 3
healthy_threshold = 2
```

客户端无论使用哪个域名，都连接同一个 `settings.listen`。代理读取 Minecraft Java 握手里的域名并按 `rules` 出现顺序选择首个匹配路由，再按该路由的 `strategy` 选择后端；连接失败会尝试池内其余地址。启用 `[rules.health_check]` 后，即使没有玩家也会定时进行 TCP 建连检查；达到失败阈值的节点排到健康/未知节点之后，全部节点离线时仍会逐个真实尝试，避免短暂抖动或探测误判造成硬中断。探测并发全局限制为 64，超出部分在后续调度周期继续，避免大量路由同时探测耗尽文件描述符。TCP 检查只证明端口可达，不证明 Minecraft 登录、版本或模组协商正常。默认将握手原文写入后端；若后端要求使用它自己的域名，设置 `modify_virtual_host = true`，代理会按最终选中的 backend 改写握手 Host。`proxy_protocol = "v1"` 或 `"v2"` 会在任何 Minecraft 字节之前写入对应头；只有后端监听器明确启用相同版本时才能使用，否则普通 Minecraft 服务端会把它当作非法握手。生产上还必须用防火墙限制后端端口仅允许代理访问，避免客户端伪造来源地址。存在 `[rules.status]` 时代理管理 Status Request 和 Ping：`custom` 完全生成响应，`backend` 拉取并覆盖后端响应；`whitelist_enabled = true` 时代理额外预读并原样保留 Login Start，只有白名单玩家才会继续连接后端。

生产配置位于 `/etc/mc-proxy/config.toml`，管理页面的变更会先校验并应用，再通过临时文件加 rename 原子持久化。若新端口无法绑定，运行时会回滚旧规则。

生产模板的 `proxy_enabled` 保持关闭，因为尚未提供真实 Minecraft 后端地址。请先在管理页面配置 host 路由，再启用全局入口。

## 本地编译

需要 Rust 1.85 或更高版本：

```sh
cargo build --release
```

启动管理端时必须提供至少 32 个字符的令牌：

```sh
cp config.example.toml config.toml
export MC_PROXY_ADMIN_TOKEN='替换为高强度随机令牌'
RUST_LOG=mc_proxy=info ./target/release/mc-proxy --config config.toml
```

## Ubuntu 24 产物

`dist/` 包含在 Ubuntu 24.04 x86_64 原生编译并通过测试的 v0.10.0：

```sh
tar -xzf dist/mc-proxy-v0.10.0-ubuntu24-x86_64.tar.gz
chmod +x mc-proxy
./mc-proxy --help
```

Android 共享存储不能可靠保留 Linux 执行权限，因此推荐传输 `.tar.gz`。归档内的 ELF 权限为 `0755`。

SHA-256 以 `dist/SHA256SUMS` 为准；本版二进制为
`b29b85b40e6960cc51d8fe63e478607452b25fd324056a24723105d4c4718713`，
归档为 `ec7115cca883d123dc64ef912f32c0f077a49c94f1862ea31119ce71522b7e77`。

## 服务器运维

```sh
systemctl status mc-proxy
journalctl -u mc-proxy -f
systemctl restart mc-proxy
nginx -t
certbot certificates
```

已部署文件：

```text
/opt/mc-proxy/mc-proxy
/etc/mc-proxy/config.toml
/etc/mc-proxy/admin.env
/etc/systemd/system/mc-proxy.service
/etc/nginx/sites-available/mc.lic6.top
/etc/nginx/conf.d/mc-proxy-rate.conf
/opt/mc-proxy/geyser/Geyser-Standalone.jar
/etc/systemd/system/mc-proxy-geyser.service
```

systemd 服务以无登录权限的 `mc-proxy` 用户运行，并使用文件系统、内核、设备、能力和地址族限制。管理令牌文件权限为 `0600`，配置为 `0640`。

生产服务器已安装 Java 21 和经官方 SHA-256 校验的 Geyser Standalone 2.11.0。由于当前只有不可控的 Hypixel 示例后端，`mc-proxy-geyser.service` 保持 `disabled/inactive`；配置自有 Java 后端并完成认证方案后再启用 UDP 入口。

## 验证

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
node --check web/app.js
```

详细 Ubuntu 构建、API、TCP、HTTPS 和证书验收结果见 `BUILD_UBUNTU24.md`。

## 性能说明

默认每个连接的每个方向使用 32 KiB 用户态缓冲区。流量指标在数据成功写入另一端后按块原子累加，因此长连接不需要等到断开才更新面板。生产调优仍应使用真实 Minecraft 协议客户端测试 16、32、64 和 128 KiB，而不是使用 HTTP 压测工具。
