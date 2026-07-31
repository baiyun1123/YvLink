# 基岩版与 Java 版互通

更新时间：2026-07-31

## 架构

mc-proxy 本身处理 Minecraft Java TCP 协议。基岩版使用 UDP/RakNet，因此互通必须经过协议翻译层。mc-proxy 支持两种提供方：

- `provider = "external"`（默认）：独立的 Geyser Standalone 进程，需要 Java 21+。
- `provider = "geyserlite"`：Minekube GeyserLite 托管运行时，由 mc-proxy 直接启动，无需 JVM；支持进程内（embedded）与子进程（subprocess）两种模式。

内置 GeyserLite 只编译进 Linux 目标（geyserlite 0.3.x 上游暂不支持 Windows 编译）；Windows 部署请使用 `provider = "external"`。

external 架构：

```text
Bedrock 客户端
  └─ UDP 19132 → Geyser Standalone
                    └─ Java TCP 25565 → mc-proxy
                                          └─ Host 路由 → Java 后端
```

geyserlite 架构：

```text
Bedrock 客户端
  └─ UDP 19132 → mc-proxy 托管的 GeyserLite（embedded 同进程 / subprocess 子进程）
                    └─ Java TCP 25565 → Host 路由 → Java 后端
```

控制台的“基岩版互通”页面负责保存预期配置，并发送真实 RakNet Unconnected Ping 检查 UDP 入口。`enabled = true` 只代表要求监控；只有页面显示 `ONLINE` 才证明翻译器实际响应。geyserlite 模式下，页面还会展示托管运行状态（运行中/启动失败/退出原因）。

## 配置对应关系

mc-proxy：

```toml
[crossplay]
enabled = true
bedrock_listen = "0.0.0.0:19132"
java_address = "bedrock.example.com"
java_port = 25565
auth_type = "online"
```

geyserlite 模式：

```toml
[crossplay]
enabled = true
provider = "geyserlite"
bedrock_listen = "0.0.0.0:19132"
java_address = "bedrock.example.com"
java_port = 25565
auth_type = "online"

[crossplay.geyserlite]
mode = "embedded"        # embedded | subprocess
offline = false          # true 时禁止自动下载原生库
motd_line1 = "YvLink"
motd_line2 = "Bedrock via GeyserLite"
# floodgate_key = "00112233445566778899aabbccddeeff"   # 仅 floodgate 需要
```

Geyser Standalone 的生成配置需要使用相同参数：

```yaml
bedrock:
  address: 0.0.0.0
  port: 19132

java:
  address: bedrock.example.com
  port: 25565
  auth-type: online
```

`java.address` 应匹配一条 mc-proxy 路由的 Host，并解析到 mc-proxy 的 Java 监听地址。生产环境可通过内部 DNS或 `/etc/hosts` 将专用域名解析到 `127.0.0.1`，避免 Geyser 流量绕公网。

## 认证模式

- `online`：基岩玩家需要通过 Geyser 登录其 Java/Microsoft 账号，适合不能修改的在线模式后端。
- `floodgate`：允许基岩账号进入，但必须在你有权管理的 Java 后端安装对应 Floodgate 插件或模组，并安全分发 `key.pem`。
- `offline`：不验证身份，只适合隔离测试环境，不应直接用于公网生产。

Floodgate 私钥可允许绕过 Java 身份认证，不能提交到仓库或发给不受信任的人。geyserlite 模式使用 AES-128 密钥：在 `[crossplay.geyserlite]` 中提供 16 字节密钥的 32 位十六进制字符串（`floodgate_key`），与控制台保存的其他互通参数一样写入本地 TOML；管理页面以密码框展示并要求与后端 Floodgate 实例配套。若担心 TOML 泄露，也可改用 `online` 认证或限制管理端访问。

## 模组兼容边界

- 原版、Paper/Spigot 及纯服务端插件通常是最佳互通目标。
- Fabric/NeoForge 仅服务器侧功能可以尝试；旧版本通常还需要 ViaVersion/ViaProxy。
- 任何要求玩家安装同款客户端模组、资源加载器或自定义网络协议的内容，Geyser 无法自动翻译给基岩客户端。
- Forge 没有通用的“任意客户端模组转基岩”方案，应按具体整合包单独验收。

## 部署

### provider = "external"（Geyser Standalone）

1. 安装 Java 21 或更高版本。
2. 从 GeyserMC 官方 Downloads API 下载 `Geyser-Standalone.jar` 并校验官方 SHA-256。
3. 首次执行 `java -jar Geyser-Standalone.jar` 生成完整配置。
4. 按本页对应关系修改 `bedrock` 与 `java` 段。
5. 使用 `deploy/geyser/mc-proxy-geyser.service` 安装独立 systemd 服务。
6. 防火墙开放 Bedrock 监听端口的 UDP，不要误开成仅 TCP。
7. 在控制台启用互通监控，确认状态从 `OFFLINE` 变为 `ONLINE`。
8. 使用真实基岩客户端完成登录、移动、聊天、背包、实体、传送和重连验收。

### provider = "geyserlite"（内置托管）

1. 使用默认特性构建（`geyserlite` + `geyserlite-download`），首次启动时 mc-proxy 会从 GitHub Release 下载与目标平台匹配的 `libgeyserlite.so`（或子进程可执行文件）并校验 SHA-256，再开始监听 Bedrock UDP。
2. 离线或受限网络环境：预置 `GEYSERLITE_LIBRARY` 环境变量指向已下载的库，或在构建时使用 `--features geyserlite-embed` 把库内嵌进二进制；同时在配置里设置 `offline = true`。
3. 生产建议先以 `subprocess` 模式验收：原生库崩溃时只重启子进程，不会拖垮 Java 代理；确认稳定后可改用 `embedded` 降低开销。
4. 不需要 Java、不需要额外的 systemd 服务；`mc-proxy.service` 一个单元即可。
5. 防火墙开放 Bedrock 监听端口的 UDP，不要误开成仅 TCP。
6. 在控制台保存配置，确认“托管运行时”显示托管中、状态徽标从 `OFFLINE` 变为 `ONLINE`。
7. 使用真实基岩客户端完成登录、移动、聊天、背包、实体、传送和重连验收。

embedded 模式与 mc-proxy 共享地址空间：GeyserLite 原生代码发生段错误会终止整个进程，`catch_unwind` 无法挽救；需要严格崩溃隔离时必须使用 `subprocess`。

官方资料：

- Geyser Standalone：<https://geysermc.org/wiki/geyser/setup/self/standalone/>
- 当前支持版本：<https://geysermc.org/wiki/geyser/supported-versions/>
- Floodgate Standalone：<https://geysermc.org/wiki/floodgate/setup/standalone/>
- Geyser Downloads API：<https://geysermc.org/wiki/api/downloads.geysermc.org/download-latest/>
- GeyserLite（Minekube）：<https://github.com/minekube/geyserlite>
- Gate Bedrock 使用说明：<https://gate.minekube.com/guide/bedrock>
