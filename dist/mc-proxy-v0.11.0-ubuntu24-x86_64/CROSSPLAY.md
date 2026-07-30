# 基岩版与 Java 版互通

更新时间：2026-07-30

## 架构

mc-proxy 本身处理 Minecraft Java TCP 协议。基岩版使用 UDP/RakNet，因此互通模式采用独立的 Geyser Standalone 翻译层：

```text
Bedrock 客户端
  └─ UDP 19132 → Geyser Standalone
                    └─ Java TCP 25565 → mc-proxy
                                          └─ Host 路由 → Java 后端
```

控制台的“基岩版互通”页面负责保存预期配置，并发送真实 RakNet Unconnected Ping 检查 Geyser UDP 入口。`enabled = true` 只代表要求监控；只有页面显示 `ONLINE` 才证明翻译器实际响应。

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

Floodgate 私钥可允许绕过 Java 身份认证，不能提交到仓库、放进网页或发给不受信任的人。

## 模组兼容边界

- 原版、Paper/Spigot 及纯服务端插件通常是最佳互通目标。
- Fabric/NeoForge 仅服务器侧功能可以尝试；旧版本通常还需要 ViaVersion/ViaProxy。
- 任何要求玩家安装同款客户端模组、资源加载器或自定义网络协议的内容，Geyser 无法自动翻译给基岩客户端。
- Forge 没有通用的“任意客户端模组转基岩”方案，应按具体整合包单独验收。

## 部署

1. 安装 Java 21 或更高版本。
2. 从 GeyserMC 官方 Downloads API 下载 `Geyser-Standalone.jar` 并校验官方 SHA-256。
3. 首次执行 `java -jar Geyser-Standalone.jar` 生成完整配置。
4. 按本页对应关系修改 `bedrock` 与 `java` 段。
5. 使用 `deploy/geyser/mc-proxy-geyser.service` 安装独立 systemd 服务。
6. 防火墙开放 Bedrock 监听端口的 UDP，不要误开成仅 TCP。
7. 在控制台启用互通监控，确认状态从 `OFFLINE` 变为 `ONLINE`。
8. 使用真实基岩客户端完成登录、移动、聊天、背包、实体、传送和重连验收。

官方资料：

- Geyser Standalone：<https://geysermc.org/wiki/geyser/setup/self/standalone/>
- 当前支持版本：<https://geysermc.org/wiki/geyser/supported-versions/>
- Floodgate Standalone：<https://geysermc.org/wiki/floodgate/setup/standalone/>
- Geyser Downloads API：<https://geysermc.org/wiki/api/downloads.geysermc.org/download-latest/>

