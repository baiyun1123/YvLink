# Minecraft 协议与模组兼容矩阵

更新时间：2026-08-08

## 当前工作模式

v0.11.0 是“协议感知的薄代理”：解析 Handshake、Status Request/Ping 和 Login Start，完成域名选路、后端状态覆盖、自定义状态响应与玩家名白名单；连接后端后不终止 Minecraft 会话，后续 Login、Configuration、Play 与插件消息按原始字节透明转发。路由还可在 Minecraft 数据前写入 PROXY Protocol v1/v2，并通过可选 TCP 或 Minecraft Status 主动健康检查提前降低异常节点的选路优先级。

这与 Gate Lite 的边界相近，但额外提供了代理白名单。它不是完整的 Velocity/BungeeCord 类会话代理，目前不负责 Mojang 身份认证、服务器切换或玩家信息转发。

## 兼容矩阵

| 服务端 | 当前等级 | 已有证据 | 约束 |
|---|---|---|---|
| 原版 Java | 基础可用 | Status 覆盖/缓存/fallback、Ping/Pong、白名单拒绝、白名单放行集成测试；生产协议探针通过 | 玩家身份最终由 `online-mode=true` 后端认证 |
| Fabric 1.21.1 | 真实服务端前段矩阵通过 | Fabric Loader 0.19.3：直连/代理 Status 相等；Login Success 与首个 Configuration 包摘要相等 | 尚未用真实游戏客户端完成进服/游玩验收；不支持 Velocity modern forwarding |
| Forge 1.8–1.12.2 | 薄代理协议矩阵通过 | `\0FML\0` 路由与改写保留；双向 `FML\|HS` 类负载逐字节保真；状态保留旧版 `modinfo` | 仍需真实旧版服务端/客户端验收 |
| Forge 1.13–1.20.1 | 薄代理协议矩阵通过 | `\0FML2\0`/`\0FML3\0` 路由与改写保留；双向 `fml:loginwrapper` 类负载逐字节保真 | 不提供终止会话代理所需的 FML 中继、缓存重放与服务器切换 |
| Forge/NeoForge 1.20.2+ | 真实服务端前段矩阵通过 | Forge 52.1.16 与 NeoForge 21.1.244：`\0FORGE` Login、Configuration 首包和 `forgeData`/`isModded` 状态扩展经代理保真 | 尚未执行完整 Configuration 状态机、进服/游玩及复杂模组包矩阵；不支持 Velocity modern forwarding |
| Paper/Velocity 后端 | 仅普通直连后端模式 | 基本 TCP/Minecraft 透传可用 | 后端不能要求 Velocity/Bungee 玩家信息转发 |
| Bedrock → Java | Geyser 外部翻译层或托管 GeyserLite | mc-proxy 提供 Crossplay 配置与真实 RakNet UDP 健康探测；托管 GeyserLite 支持 Floodgate 密钥 | 客户端模组无法翻译；Floodgate 仅适用于可安装配套组件的后端 |
| Java → 不同版本 Java 后端 | 可选 ViaLite 托管 subprocess | 已选路后经仅回环 ViaLite 入口连接后端，控制台可查看运行状态 | 不解决客户端前端握手兼容；不能与 PROXY Protocol 共用；尚不提供 Velocity/Bungee 身份转发 |
| Fabric + NotEnoughBandwidth（NEB） | 后端/客户端模组配套 | YvLink 后续流量保持字节透明，可与已验证的 NEB 链路共存 | NEB 不是代理模块，不能嵌入；必须在真实服务端与客户端组合中验收兼容模式和黑名单 |

## 重要限制

1. 模组路由应选择 `status.mode = "backend"`：代理会保留 `forgeData`、`modinfo`、favicon 与未知扩展，只覆盖明确配置的字段。`custom` 模式仍会完全生成 Status JSON，适合不依赖模组状态扩展的路由。
2. 玩家名白名单发生在 Mojang 身份认证之前，只是提前筛选。攻击者可以在 Login Start 中声明任意玩家名；安全边界仍是无法从公网直连、且使用在线模式认证的后端。
3. 薄代理无法安全生成 Velocity modern forwarding 数据。若后端安装 FabricProxy-Lite 或 Proxy-Compatible-Forge 并强制 modern forwarding，当前版本会被后端拒绝。
4. v0.8.0 的握手观察指标只按 Host 中的 `FML`、`FML2/FML3`、`FORGE` 标记分类。Fabric 与原版没有可在初始 Handshake 中可靠区分的统一标记，所以合并为“原版 / Fabric”；指标不代表玩家已完成认证或模组协商。
5. PROXY Protocol 默认关闭。启用时后端必须在该端口明确支持相同版本，并应通过防火墙只接受代理连接；它不会生成 Velocity/Bungee 转发数据。
6. ViaLite 放在 YvLink 与后端之间，不能像 Gate Lite 那样在原始字节管道中凭空重写协议。启用它时必须关闭路由的 PROXY Protocol；当前项目会对 ViaLite 使用 `forwarding = none`。
7. [NotEnoughBandwidth](https://github.com/USS-Shenzhou/NotEnoughBandwidth) 的紧凑包头、聚合压缩和延迟区块缓存需要 Fabric 生态中的服务端/客户端实现协作。它不是一个可嵌入 YvLink 的 Rust 网络库；请在生产启用前执行目标模组组合的完整登录与游玩回归。

## 真实服务端矩阵结果

2026-07-30 已在隔离目录安装并实测 Minecraft 1.21.1 的 Fabric Loader 0.19.3、Forge 52.1.16 和 NeoForge 21.1.244；三份官方安装器的 Maven SHA-1 均已校验。用户明确接受 Minecraft EULA 后，三份测试实例已设为 `eula=true`。测试采用“每次只启动一套、最大堆 512 MiB、仅监听回环地址”，没有修改或重启生产服务。

三套连续矩阵最终均为 `passed=true`：

- Fabric、Forge、NeoForge 的直连、透明转发与 `status.mode=backend` Status JSON 完整相等。
- Forge 的 `forgeData`、NeoForge 的 `isModded` 等扩展字段原样保留。
- 三套服务端的 Login Success 和发送 Login Acknowledged 后首个 Configuration 包，其包 ID、长度与 SHA-256 在直连和代理链路完全相同。
- `minecraft-status` 主动探测均能从 `healthy` 切换到服务端停止后的 `unhealthy`。

合并报告为 `tests/modded-matrix-summary.json`，SHA-256 为 `8b20399dd6319a2550f61d7c6ed11326ae52cf67c234274426e2f67541c50dbf`。执行器在等待后端时使用真实 Minecraft Status 响应作为就绪条件，避免 Forge/NeoForge 已监听 TCP 但仍处于初始化阶段时产生假失败。具体安全边界和复现命令见 `tests/MODDED_MATRIX_RUNBOOK.md`。

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
- PROXY Protocol v1/v2，用于可信内网后端真实 IP 传递（v0.9.0 已完成，含 IPv4/IPv6、普通转发与后端状态查询）。
- 每后端活动连接、成功连接、尝试失败和连接延迟指标（v0.7.0 已完成）。
- 主动 TCP 健康检查、连续失败/恢复阈值、健康感知候选顺序与无玩家空闲探测（v0.10.0 已完成）。
- Minecraft Status 协议健康检查：验证 Handshake、Status JSON 和 Ping/Pong，支持探测 Host、协议号及 PROXY Protocol（v0.11.0 已完成）。

### P2：完整会话代理

- 在线模式认证与加密会话终止。
- Velocity modern forwarding（HMAC 密钥）。
- BungeeCord/BungeeGuard 转发。
- Forge 1.13–1.20.1 的 LoginPluginMessage/FML 中继。
- Forge/NeoForge 1.20.2+ Configuration 阶段状态机。
- 扩展 Fabric、Forge、NeoForge 的真实客户端、完整 Configuration 状态机与代表性复杂模组包版本矩阵；1.21.1 三加载器服务端前段矩阵已通过。
- 可重复容器化验收；当前执行器使用隔离目录与回环端口，尚未容器化。

### P1.5：模组薄代理证据（v0.8.0 已完成协议夹具）

- 对 Gate 使用的 `FML`、`FML2`、`FML3`、`FORGE` 与 `FORGE2` Host 标记进行路由分类和统计。
- `modifyVirtualHost` 改写基础域名时完整保留所有加载器标记。
- Fabric、Forge 1.12.2、Forge 1.16.5、Forge 1.20.1、NeoForge 1.21.1 和 FORGE NAT 六组双向负载夹具逐字节通过。
- 这些夹具证明薄代理不修改模拟协议流；1.21.1 三加载器真实服务端前段矩阵也已通过，但真实游戏客户端和具体复杂模组组合仍属于后续矩阵。

## 参考边界

- Gate Lite 文档：<https://gate.minekube.com/guide/lite>
- Gate 模组服文档：<https://gate.minekube.com/guide/modded-servers>
- Gate 兼容性文档：<https://gate.minekube.com/guide/compatibility>
