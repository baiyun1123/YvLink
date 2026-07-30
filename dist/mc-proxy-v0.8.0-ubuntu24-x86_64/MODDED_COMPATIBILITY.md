# Minecraft 协议与模组兼容矩阵

更新时间：2026-07-30

## 当前工作模式

v0.8.0 是“协议感知的薄代理”：解析 Handshake、Status Request/Ping 和 Login Start，完成域名选路、后端状态覆盖、自定义状态响应与玩家名白名单；连接后端后不终止 Minecraft 会话，后续 Login、Configuration、Play 与插件消息按原始字节透明转发。

这与 Gate Lite 的边界相近，但额外提供了代理白名单。它不是完整的 Velocity/BungeeCord 类会话代理，目前不负责 Mojang 身份认证、服务器切换或玩家信息转发。

## 兼容矩阵

| 服务端 | 当前等级 | 已有证据 | 约束 |
|---|---|---|---|
| 原版 Java | 基础可用 | Status 覆盖/缓存/fallback、Ping/Pong、白名单拒绝、白名单放行集成测试；生产协议探针通过 | 玩家身份最终由 `online-mode=true` 后端认证 |
| Fabric | 薄代理协议矩阵通过 | 无标记 Handshake、Login 后双向自定义负载逐字节保真 | 尚未用真实 Fabric 服务端/客户端验收；不支持 Velocity modern forwarding |
| Forge 1.8–1.12.2 | 薄代理协议矩阵通过 | `\0FML\0` 路由与改写保留；双向 `FML\|HS` 类负载逐字节保真；状态保留旧版 `modinfo` | 仍需真实旧版服务端/客户端验收 |
| Forge 1.13–1.20.1 | 薄代理协议矩阵通过 | `\0FML2\0`/`\0FML3\0` 路由与改写保留；双向 `fml:loginwrapper` 类负载逐字节保真 | 不提供终止会话代理所需的 FML 中继、缓存重放与服务器切换 |
| Forge/NeoForge 1.20.2+ | 薄代理协议矩阵通过 | `\0FORGE`/`\0FORGE2` 路由与改写保留；双向 Configuration 自定义负载逐字节保真；状态保留 `forgeData` | 仍需真实 Configuration 会话验收；不支持 Velocity modern forwarding |
| Paper/Velocity 后端 | 仅普通直连后端模式 | 基本 TCP/Minecraft 透传可用 | 后端不能要求 Velocity/Bungee 玩家信息转发 |
| Bedrock → Java | Geyser 外部翻译层 | mc-proxy 提供 Crossplay 配置与真实 RakNet UDP 健康探测 | 客户端模组无法翻译；Floodgate 仅适用于可安装配套组件的后端 |

## 重要限制

1. 模组路由应选择 `status.mode = "backend"`：代理会保留 `forgeData`、`modinfo`、favicon 与未知扩展，只覆盖明确配置的字段。`custom` 模式仍会完全生成 Status JSON，适合不依赖模组状态扩展的路由。
2. 玩家名白名单发生在 Mojang 身份认证之前，只是提前筛选。攻击者可以在 Login Start 中声明任意玩家名；安全边界仍是无法从公网直连、且使用在线模式认证的后端。
3. 薄代理无法安全生成 Velocity modern forwarding 数据。若后端安装 FabricProxy-Lite 或 Proxy-Compatible-Forge 并强制 modern forwarding，当前版本会被后端拒绝。
4. v0.8.0 的握手观察指标只按 Host 中的 `FML`、`FML2/FML3`、`FORGE` 标记分类。Fabric 与原版没有可在初始 Handshake 中可靠区分的统一标记，所以合并为“原版 / Fabric”；指标不代表玩家已完成认证或模组协商。

## 后续实现顺序

### P0：模组安全的状态响应（v0.6.0 已完成）

- 已获取后端 Status JSON 并保留所有未知字段。
- 已实现仅覆盖配置指定的 MOTD、版本、协议号和展示人数。
- 已按“后端 + 客户端协议号”缓存并支持 TTL。
- 已实现后端离线可配置 fallback。
- 已用集成测试验证 Forge/NeoForge 状态扩展、favicon 与未知字段不丢失。

### P1：薄代理生产能力

- 多后端、顺序故障转移与原子健康状态（v0.7.0 已完成）。
- Round-robin、随机、最少连接与最低连接延迟策略（v0.7.0 已完成）。
- PROXY Protocol v1/v2，用于可信内网后端真实 IP 传递。
- 每后端活动连接、成功连接、尝试失败和连接延迟指标（v0.7.0 已完成）；PROXY Protocol 与主动健康检查仍待实现。

### P2：完整会话代理

- 在线模式认证与加密会话终止。
- Velocity modern forwarding（HMAC 密钥）。
- BungeeCord/BungeeGuard 转发。
- Forge 1.13–1.20.1 的 LoginPluginMessage/FML 中继。
- Forge/NeoForge 1.20.2+ Configuration 阶段状态机。
- Fabric、Forge、NeoForge 的真实服务端版本矩阵和可重复容器化验收。

### P1.5：模组薄代理证据（v0.8.0 已完成协议夹具）

- 对 Gate 使用的 `FML`、`FML2`、`FML3`、`FORGE` 与 `FORGE2` Host 标记进行路由分类和统计。
- `modifyVirtualHost` 改写基础域名时完整保留所有加载器标记。
- Fabric、Forge 1.12.2、Forge 1.16.5、Forge 1.20.1、NeoForge 1.21.1 和 FORGE NAT 六组双向负载夹具逐字节通过。
- 这些测试证明薄代理不修改模拟协议流；真实客户端、真实加载器和具体模组组合仍属于待执行的外部矩阵。

## 参考边界

- Gate Lite 文档：<https://gate.minekube.com/guide/lite>
- Gate 模组服文档：<https://gate.minekube.com/guide/modded-servers>
- Gate 兼容性文档：<https://gate.minekube.com/guide/compatibility>
