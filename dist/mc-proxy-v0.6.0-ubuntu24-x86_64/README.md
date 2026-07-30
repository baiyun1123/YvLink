# mc-proxy

面向 Minecraft Java 版的协议感知转发器和 Web 管理控制台。数据面解析 Handshake、Status 与 Login Start，用于域名选路、自定义 MOTD 和登录白名单；后续游戏与模组协议保持透明转发。控制面支持多规则、实时流量与并发监控、在线配置和原子持久化。

生产管理地址：<https://mc.lic6.top>

## 功能

- Tokio 多线程运行时，每个连接使用轻量异步任务。
- 单一 Minecraft Java 监听入口，所有路由共用同一个 IP 和端口。
- 按握手 virtual host 顺序匹配 `host → backend`，支持多个 host、`*`/`?` 通配和末尾 `host = "*"` 兜底。
- 每条路由支持完全自定义状态，或读取后端 Status 后只覆盖指定字段；后端模式保留 Forge/NeoForge 扩展、favicon、玩家 sample 与未知 JSON 字段。
- 后端状态按“backend + 客户端协议号”缓存，并可在后端离线时返回可配置 fallback。
- 每条路由可启用玩家名白名单，在 Login Start 后、连接后端前发送原版 Login Disconnect。
- 默认保留 Forge/FML 握手 Host 的 NUL 扩展；白名单只读取玩家名，允许后续 Fabric、Forge 和 NeoForge 插件消息透明通过。
- 在线新建、修改、启停和删除规则，无需手工编辑 TOML。
- 实时统计活跃连接、累计连接、上下行字节、后端失败、转发异常、并发拒绝、白名单拒绝、本地状态响应、状态缓存命中和 fallback。
- 前端每 2 秒采样状态，使用 SVG 绘制 60 秒实时吞吐曲线。
- 前端图标全部为内联 SVG，不使用 Emoji，不依赖 CDN。
- 管理 API 使用 Bearer Token，令牌只保存在浏览器 `sessionStorage`。
- 管理 HTTP 只监听 `127.0.0.1:18080`，公网由 Nginx HTTPS 反向代理。
- Nginx API 限速、安全响应头、Let’s Encrypt 自动续期。
- 后端连接超时、连接数限制、半关闭处理和 Ctrl+C/SIGTERM 优雅退出。
- Linux/Android 可选 `SO_REUSEPORT`。

当前只支持 Minecraft Java TCP。白名单是后端认证前的快速筛选，不能替代在线模式身份认证。基岩版 UDP/RakNet、协议级真实 IP 和版本转换尚不在当前版本范围。

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

## 配置

示例结构：

```toml
[admin]
listen = "127.0.0.1:18080"

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
modify_virtual_host = false
whitelist_enabled = true
whitelist = ["Alice", "Bob"]
whitelist_message = "§c你不在此服务器的白名单中。"
enabled = true

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
backend = "backend.example.com:25565"
modify_virtual_host = true
whitelist_enabled = false
whitelist = []
enabled = true

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
modify_virtual_host = false
whitelist_enabled = false
whitelist = []
enabled = true
```

客户端无论使用哪个域名，都连接同一个 `settings.listen`。代理读取 Minecraft Java 握手里的域名并按 `rules` 出现顺序选择首个匹配后端；新增规则会自动插到已启用的 `host = "*"` 兜底规则之前。默认将握手原文写入后端；若后端要求使用它自己的域名，设置 `modify_virtual_host = true`，代理会把握手 Host 改为 `backend` 主机名。存在 `[rules.status]` 时代理管理 Status Request 和 Ping：`custom` 完全生成响应，`backend` 拉取并覆盖后端响应；`whitelist_enabled = true` 时代理额外预读并原样保留 Login Start，只有白名单玩家才会继续连接后端。

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

`dist/` 包含在 Ubuntu 24.04 x86_64 原生编译并通过测试的 v0.6.0：

```sh
tar -xzf dist/mc-proxy-v0.6.0-ubuntu24-x86_64.tar.gz
chmod +x mc-proxy
./mc-proxy --help
```

Android 共享存储不能可靠保留 Linux 执行权限，因此推荐传输 `.tar.gz`。归档内的 ELF 权限为 `0755`。

SHA-256 以 `dist/SHA256SUMS` 为准。

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
```

systemd 服务以无登录权限的 `mc-proxy` 用户运行，并使用文件系统、内核、设备、能力和地址族限制。管理令牌文件权限为 `0600`，配置为 `0640`。

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
