# Ubuntu 24 编译、部署与测试报告

## v0.13.0 GeyserLite 托管互通增量报告

- 构建时间：2026-07-31 08:40–09:20（Asia/Shanghai）
- 目标环境：Ubuntu 24.04.4 LTS x86_64，Rust/Cargo 1.97.1
- rustfmt：通过
- Clippy `-D warnings`：通过，0 个警告
- 单元测试：38 个通过，0 个失败
- 集成测试：18 个通过，0 个失败
- release 构建：通过；`cargo check --no-default-features`：通过
- JavaScript 语法检查（`node --check web/app.js`）：通过

新增覆盖：

- `crossplay.provider = "external" | "geyserlite"`；旧配置默认 `external`，保持兼容。
- `[crossplay.geyserlite]`：embedded/subprocess 模式、library/binary 路径、offline、MOTD 两行、Floodgate 16 字节十六进制密钥。
- 序列化往返测试确认 provider 以产品名 `geyserlite` 输出，而不是 kebab-case 的 `geyser-lite`。
- 配置校验覆盖 Floodgate 密钥格式与缺失、模式与路径互斥冲突、旧 TOML 无新字段解析。
- 控制台互通页新增提供方与托管运行时状态，表单按提供方/模式/认证方式渐进显示参数。
- 管理 API `GET/PUT /api/v1/crossplay` 返回 `runtime`（available/enabled/running/mode/error），文档同步更新。

服务器隔离验收：

- embedded 首次启动自动下载 `libgeyserlite.so`（来自 GitHub Release，SHA-256 校验），`0.0.0.0:19133` 真实 UDP 监听，RakNet Pong 返回配置的两行 MOTD，`/api/v1/crossplay` 显示 `running=true`、`online=true`。
- subprocess 模式以独立子进程运行 GeyserLite；PUT 修改 MOTD 后子进程安全重启，新 MOTD 生效，mc-proxy 进程存活。
- 实测发现 embedded 在进程内“停止后再次启动”会把整个 mc-proxy 带崩（GeyserLite 原生桥接限制），已把该路径改为安全拒绝：保留当前实例运行，并在 `runtime.error` 提示重启 mc-proxy 生效或改用 subprocess。
- 测试进程与 18081/25600/19133 监听全部清理；生产 `mc-proxy.service` 未改动。

遗留边界：

- 真实 Windows/Android/iOS 基岩客户端登录与游玩矩阵尚未执行，需要自有 Paper/Fabric 后端后验收。
- GeyserLite 内嵌的 Geyser 会在日志中输出 log4j/GraalVM 相关告警，不影响 UDP 监听与 Pong 响应。
- geyserlite 0.3.x 上游代码在 Windows 无法编译，内置 GeyserLite 已限定为 Linux 目标；Windows 发布包改用外部 Geyser Standalone（`provider = "external"`），Windows CI 任务已随 v0.13.0 重新构建通过。

---

## v0.11.0 Fabric / Forge / NeoForge 真实矩阵增量报告

- 验收时间：2026-07-30 04:47–05:00（Asia/Shanghai）
- 目标环境：Ubuntu 24.04.4 LTS x86_64，OpenJDK 21
- Minecraft：1.21.1，协议号 767
- Fabric：Loader 0.19.3，通过
- Forge：52.1.16，通过
- NeoForge：21.1.244，通过

验收覆盖：

- 用户明确接受 Minecraft EULA 后，三套隔离测试实例均设为 `eula=true`；执行器自身仍不会修改 EULA。
- 每次只启动一套 JVM，最大堆 512 MiB，真实服务端、临时代理和管理端均仅监听回环地址。
- 后端就绪以成功完成 Minecraft Status 协议为准，避免 Forge 端口已监听但仍在初始化时产生假失败。
- 直连、透明代理和后端状态托管三份 Status JSON 完整相等；Forge `forgeData` 与 NeoForge `isModded` 保真。
- Login Success 和发送 Login Acknowledged 后首个 Configuration 包的 ID、长度与 SHA-256 在直连和代理链路相等。
- 三套 `minecraft-status` 主动健康检查均先达到 `healthy`，停止后端后均达到 `unhealthy`。
- 单项运行和最终三套连续运行全部通过，合并结果 `passed=true`。

运行收尾：

- 合并报告：`tests/modded-matrix-summary.json`
- 报告 SHA-256：`8b20399dd6319a2550f61d7c6ed11326ae52cf67c234274426e2f67541c50dbf`
- 测试端口 26701–26703、26610、28110 全部清理。
- 生产 `mc-proxy.service` 保持 `active`，没有修改或重启生产配置。
- 本次仅修正测试执行器的协议就绪判断，无后端 HTTP API、数据库结构或生产二进制变更。

---

## v0.11.0 Minecraft Status 协议健康检查增量报告

- 构建时间：2026-07-30 03:34–03:50（Asia/Shanghai）
- 目标环境：Ubuntu 24.04.4 LTS x86_64，Rust/Cargo 1.97.1
- rustfmt：通过
- Clippy `-D warnings`：通过，0 个警告
- 单元测试：31 个通过，0 个失败
- 集成测试：18 个通过，0 个失败
- release 构建：通过
- JavaScript 语法与 UI 交付审计：通过

新增覆盖：

- `health_check.mode = "tcp" | "minecraft-status"`；旧配置默认 `tcp`，保持兼容。
- Minecraft 模式验证 Java Handshake、Status Response JSON 基础字段和 Ping/Pong，并将连接、读取与写入纳入同一个超时。
- 探测 Host 支持自动推导和 `minecraft_host` 显式覆盖，协议号可配置，路由 PROXY Protocol v1/v2 同步应用。
- 对模组状态中的 `forgeData`、`modinfo` 和未知扩展保持开放，只要求原版基础状态结构。
- 自动化覆盖有效响应、无效 JSON、可连接 HTTP 伪后端、Host 推导、阈值状态切换和持久化。
- 控制台按探测模式渐进展示 Host/协议参数，并在路由卡片显示 TCP 或 Minecraft Status。

服务器隔离验收：

- 使用 `tests/status_backend_fixture.py` 和 `tests/status-healthcheck.standalone.toml` 启动独立实例。
- 第一阶段：`127.0.0.1:29996` 是可连接的 HTTP 服务但被正确标记 `unhealthy`；`127.0.0.1:29997` 完成 Status/Pong 并标记 `healthy`。
- 停止 29997 后第二阶段：两个后端均为 `unhealthy`，累计失败继续增长。
- 临时代理及 26566/28081/29996/29997 监听全部清理。

生产部署与验收：

- 备份：`/var/backups/mc-proxy/mc-proxy.pre-v0.11.0-20260730`
- 配置备份：`/var/backups/mc-proxy/config.pre-v0.11.0-20260730.toml`
- systemd：`active`
- 状态 API：版本 `0.11.0`；`hyp` 路由保持健康检查关闭，默认模式为 `tcp`，不周期探测第三方后端。
- 回环和服务器公网 IP + `hyp.mc.lic6.top` 握手均返回有效 Hypixel Status/Ping。
- 生产累计连接 2，后端失败 0、转发失败 0；公网页面资产为 `20260730-statusprobe1`，API 文档显示 v0.11.0。

交付产物：

```text
dist/mc-proxy-v0.11.0-ubuntu24-x86_64/mc-proxy
SHA-256: cb3472cb127a23c6b86cacd9ac6a384c22ef510583c7c5b14cb5bb44ce9d3e7c

dist/mc-proxy-v0.11.0-ubuntu24-x86_64.tar.gz
SHA-256: 807354639b588bc33352228ca5369a7c71a9a664e2c514fe269fa8c8c45c1f7d
```

---

## v0.10.0 主动后端健康检查增量报告

- 构建时间：2026-07-30 03:08–03:30（Asia/Shanghai）
- 目标环境：Ubuntu 24.04.4 LTS x86_64，Rust/Cargo 1.97.1
- rustfmt：通过
- Clippy `-D warnings`：通过，0 个警告
- 单元测试：28 个通过，0 个失败
- 集成测试：16 个通过，0 个失败
- release 构建：通过
- JavaScript 语法与 UI 交付审计：通过

新增覆盖：

- 路由级主动 TCP 健康检查，支持检查间隔、连接超时、连续失败阈值和连续恢复阈值，默认关闭。
- 后端状态机包含 `unknown`、`healthy`、`unhealthy`；健康/未知节点优先，离线节点排后，全部离线时仍保留真实连接尝试。
- 无玩家连接时调度器仍会立即首检并按间隔持续探测；同时探测全局限制为 64，防止大量后端耗尽文件描述符。
- 探测任务去重，取消或退出时释放 in-flight 标记；配置热重载后安全回到等待首检状态。
- 状态 API、周期日志、路由卡片和实例状态新增健康检查状态、延迟、连续成功/失败与累计计数。
- UI 使用文字 + 色彩双重表达，复杂阈值按启用状态渐进展开，手机端后端状态改为纵向布局。

服务器隔离验收：

- 使用 `tests/healthcheck.standalone.toml` 启动隔离实例，不修改生产路由。
- 第一阶段：`127.0.0.1:29998 = unhealthy`、`127.0.0.1:29999 = healthy`。
- 停止健康后端后第二阶段：两节点均为 `unhealthy`，累计失败数继续增长。
- 临时实例、后端进程及 26565/28080/29999 监听全部清理。

生产部署与验收：

- 备份：`/var/backups/mc-proxy/mc-proxy.pre-v0.10.0-20260730`
- 配置备份：`/var/backups/mc-proxy/config.pre-v0.10.0-20260730.toml`
- systemd：`active`
- 状态 API：版本 `0.10.0`；`hyp` 路由健康检查保持关闭，避免持续探测第三方后端。
- 回环与公网服务器 IP + `hyp.mc.lic6.top` 握手均返回有效 Hypixel Status/Ping。
- 生产累计连接 2，后端失败 0、转发失败 0；公网页面资产为 `20260730-health1`，API 文档显示 v0.10.0。

交付产物：

```text
dist/mc-proxy-v0.10.0-ubuntu24-x86_64/mc-proxy
SHA-256: 390a470b9edd17383336b5323af5a8a09a34cb63311706b87046010e7d077efe

dist/mc-proxy-v0.10.0-ubuntu24-x86_64.tar.gz
SHA-256: 见 `dist/SHA256SUMS`（打包后生成）
```

---

## v0.9.0 PROXY Protocol v1/v2 增量报告

- 构建时间：2026-07-30 02:45–03:06（Asia/Shanghai）
- 目标环境：Ubuntu 24.04.4 LTS x86_64，Rust/Cargo 1.97.1
- rustfmt：通过
- Clippy `-D warnings`：通过，0 个警告
- 单元测试：24 个通过，0 个失败
- 集成测试：15 个通过，0 个失败
- release 构建：通过
- JavaScript 语法与 UI 静态检查：通过

新增覆盖：

- 路由级 `proxy_protocol = "off" | "v1" | "v2"`，默认关闭，并兼容 Gate 风格布尔值。
- v1 覆盖 IPv4/IPv6 文本头与混合地址族 `UNKNOWN`；v2 覆盖 IPv4/IPv6 二进制地址块与 UNSPEC。
- 普通转发和后端 Status 查询都会在任何 Minecraft 字节前发送头；缓存命中不会重复连接或计数。
- v1/v2 集成测试验证真实 socket 源地址、入口目标地址、首包顺序和后续 Minecraft 握手逐字节不变。
- 状态 API、周期日志和控制台新增 v1/v2 发送计数；路由卡片和编辑器新增版本显示、安全警告与移动端适配。

生产部署与验收：

- 备份：`/var/backups/mc-proxy/mc-proxy.pre-v0.9.0-20260730`
- 配置备份：`/var/backups/mc-proxy/config.pre-v0.9.0-20260730.toml`
- systemd：`active`
- TCP 监听：`0.0.0.0:25565`；管理端：`127.0.0.1:18080`
- 状态 API：版本 `0.9.0`，生产 `hyp` 路由的 `proxy_protocol = "off"`，v1/v2 计数均为 0。
- 生产 Hypixel Status/Ping 经回环和公网服务器 IP + `hyp.mc.lic6.top` 握手均成功，后端失败与转发失败为 0。
- 公网页面已返回新资产版本 `20260730-proxy1`，公网 API 文档显示 v0.9.0。
- 未对 Hypixel 强行发送 PROXY 头；目标未声明支持时启用会破坏 Minecraft 握手。协议启用态由自动化 socket 夹具验收。

交付产物：

```text
dist/mc-proxy-v0.9.0-ubuntu24-x86_64/mc-proxy
SHA-256: 968ad624b930c13041d93520b2c0053627980eb1c88dfb0deefc4caf334aed4a

dist/mc-proxy-v0.9.0-ubuntu24-x86_64.tar.gz
SHA-256: 见 `dist/SHA256SUMS`（打包后生成）
```

---

## v0.8.0 模组握手观察与协议夹具增量报告

- 构建时间：2026-07-30 02:35–02:43（Asia/Shanghai）
- 目标环境：Ubuntu 24.04.4 LTS x86_64，Rust/Cargo 1.97.1
- rustfmt：通过
- Clippy `-D warnings`：通过，0 个警告
- 单元测试：20 个通过，0 个失败
- 集成测试：13 个通过，0 个失败
- release 构建：通过
- JavaScript 与 HTML 静态检查：通过

新增覆盖：

- 按 Gate 的 Host 标记规则区分旧版 `FML`、现代 `FML2/FML3`、1.20.2+ `FORGE/FORGE2` 和无标记握手。
- Host 路由只使用首个 NUL 之前的域名；启用 `modifyVirtualHost` 后仍逐字节保留加载器标记。
- Fabric 1.20.1、Forge 1.12.2、Forge 1.16.5、Forge 1.20.1、NeoForge 1.21.1 与 FORGE NAT 六组双向夹具通过。
- 每组夹具均验证客户端→后端和后端→客户端的 Login/Configuration 类未知负载逐字节不变。
- 新增四类握手聚合指标，并同步至状态 API、周期日志和响应式控制台。

生产部署与协议验收：

- 备份：`/var/backups/mc-proxy/mc-proxy.pre-v0.8.0-20260730`
- 配置备份：`/var/backups/mc-proxy/config.pre-v0.8.0-20260730.toml`
- systemd：`active`
- 健康检查：`/healthz = ok`
- 对生产 `hyp.mc.lic6.top` 分别发送无标记、`FML`、`FML2`、`FML3`、`FORGE`、`FORGE2` Status 握手，六次均由 Hypixel 返回有效 Status 与 Ping/Pong。
- 指标验证：无标记 1、旧 Forge 1、现代 Forge Login 2、Configuration 系 2；后端尝试失败、故障转移、后端失败和转发失败均为 0。
- 公网页面显示“协议握手观察”，公网 API 文档显示 v0.8.0。
- 该结果证明薄代理链路和模拟握手兼容，不替代真实 Fabric/Forge/NeoForge 客户端与服务端验收。

交付二进制：

```text
dist/mc-proxy-v0.8.0-ubuntu24-x86_64/mc-proxy
SHA-256: b29b85b40e6960cc51d8fe63e478607452b25fd324056a24723105d4c4718713

dist/mc-proxy-v0.8.0-ubuntu24-x86_64.tar.gz
SHA-256: ec7115cca883d123dc64ef912f32c0f077a49c94f1862ea31119ce71522b7e77
```

---

## v0.7.0 多后端与 Java/基岩互通增量报告

- 构建时间：2026-07-30 02:06–02:26（Asia/Shanghai）
- 目标环境：Ubuntu 24.04.4 LTS x86_64，Rust/Cargo 1.97.1，OpenJDK 21
- rustfmt：通过
- Clippy `-D warnings`：通过，0 个警告
- 单元测试：18 个通过，0 个失败
- 集成测试：12 个通过，0 个失败
- release 构建：通过
- JavaScript 语法：通过

新增覆盖：

- 旧版单后端字符串与新版后端数组配置兼容，五种选择策略均有确定性测试。
- Java 登录和后端 Status 的首节点失败均可切换至下一节点，并更新逐后端健康指标。
- 模拟 RakNet UDP 端点验证 Crossplay Pong 解析、延迟与 MOTD；禁用状态不会产生网络探测。
- Crossplay Java 目标必须命中现有启用路由，端口必须与 Java 代理监听端口一致。
- 管理器保存 Crossplay 认证模式并保持 Java 数据面运行。

生产部署与协议验收：

- 备份：`/var/backups/mc-proxy/mc-proxy.pre-v0.7.0-20260730`
- 配置备份：`/var/backups/mc-proxy/config.pre-v0.7.0-20260730.toml`
- systemd：`mc-proxy active`
- TCP 监听：`0.0.0.0:25565`；管理端：`127.0.0.1:18080`
- 临时多后端路由先连接 `127.0.0.1:9`，再连接 `mc.hypixel.net:25565`，真实 Status 成功返回 Hypixel 版本、玩家信息与 favicon。
- 指标验证：`backend_attempt_failures = 1`、`backend_failovers = 1`；第一节点失败 1 次，第二节点成功 1 次且记录连接延迟。
- 临时路由已删除，生产只保留 `hyp.mc.lic6.top → mc.hypixel.net:25565`。
- Geyser Standalone 2.11.0 build 1205 已安装，JAR SHA-256 为 `53a9d8483c733317fd80ed8c126204b6c41cef5290bf9e9a57bb1bca796256b2`。
- 因当前没有自有且兼容的 Java 后端，`mc-proxy-geyser` 保持 `disabled/inactive`，UDP 19132 未监听；Crossplay API 如实返回禁用状态。
- 公网页面已显示独立互通页、多后端编辑器与健康状态；API 文档显示 v0.7.0。

交付产物：

```text
dist/mc-proxy-v0.7.0-ubuntu24-x86_64/mc-proxy
SHA-256: 786aeb776730ffc1cca1e41a1541eb48b0a37f6e88d9ea012610831ac2610dfc

dist/mc-proxy-v0.7.0-ubuntu24-x86_64.tar.gz
SHA-256: 73094522011d44c68725c416f04cd6048f39d11d428020baabf99e63c430bade
```

---

## v0.6.0 模组安全状态覆盖增量报告

- 构建时间：2026-07-30 01:40–01:48（Asia/Shanghai）
- 目标环境：Ubuntu 24.04.4 LTS x86_64，Rust/Cargo 1.97.1
- rustfmt：通过
- Clippy `-D warnings`：通过，0 个警告
- 单元测试：13 个通过，0 个失败
- 集成测试：10 个通过，0 个失败
- release 构建：通过
- JavaScript 语法：通过

新增覆盖：

- 后端 Status JSON 只覆盖明确配置字段，并保留 `forgeData`、favicon 与任意未知扩展。
- 状态缓存按 backend 与客户端协议号隔离，第二次请求命中缓存。
- 后端不可用时返回配置的 fallback，并记录后端失败与 fallback 指标。
- 后端模式未填写的覆盖字段反序列化为 `None`，不会误用 custom 默认值。

生产部署与协议验收：

- 备份：`/var/backups/mc-proxy/mc-proxy.pre-v0.6.0-20260730`
- 配置备份：`/var/backups/mc-proxy/config.pre-v0.6.0-20260730.toml`
- systemd：`active`
- 健康检查：`ok`
- 临时 Hypixel 后端覆盖路由连续查询两次，指定 MOTD 生效，后端版本、玩家数、sample 与 favicon 保留。
- 指标验证：`status_cache_hits = 1`、`status_fallbacks = 0`、`backend_failures = 0`。
- 临时路由测试后已删除，生产只保留 `hyp` 规则。
- `hyp.mc.lic6.top` Status 再次返回 Hypixel 有效响应。
- 公网页面包含后端状态来源、TTL 和 fallback 配置；API 文档显示 v0.6.0。

交付产物：

```text
dist/mc-proxy-v0.6.0-ubuntu24-x86_64/mc-proxy
SHA-256: 4719b3c65f1cd00f417abdd334867ae6442380b8ebc6801c71c4fb7e6ca81dfd

dist/mc-proxy-v0.6.0-ubuntu24-x86_64.tar.gz
SHA-256: 64b37ddb5acae6305f45281dfa41ef740bd63a0ee4094991ad254e026bfc8c50
```

---

## v0.5.0 协议感知版本增量报告

- 构建时间：2026-07-30 01:09–01:13（Asia/Shanghai）
- 目标环境：Ubuntu 24.04.4 LTS x86_64，Rust/Cargo 1.97.1
- rustfmt：通过
- Clippy `-D warnings`：通过，0 个警告
- 单元测试：12 个通过，0 个失败
- 集成测试：8 个通过，0 个失败
- release 构建：通过
- JavaScript 与 Python 探针语法：通过

新增覆盖：

- 自定义 Status JSON 与 Ping/Pong，由代理直接响应且不连接后端。
- 白名单玩家提前 Login Disconnect，确认不会连接后端。
- 白名单玩家 Login Start 原包转发。
- Forge/FML NUL Host 扩展和后续插件数据逐字节保真。
- 示例配置反序列化与完整校验。

生产部署与协议验收：

- 备份：`/var/backups/mc-proxy/mc-proxy.pre-v0.5.0-20260730`
- 配置备份：`/var/backups/mc-proxy/config.pre-v0.5.0-20260730.toml`
- systemd：`active`
- 健康检查：`ok`
- 临时隔离路由返回指定 MOTD、协议 767、展示人数 7/100，Ping/Pong 校验成功。
- 非白名单玩家 `Bob` 收到指定 Login Disconnect；指标 `whitelist_denials` 增加。
- 临时路由测试后已删除，生产只保留 `hyp` 规则。
- `hyp.mc.lic6.top` 公网 Status 再次返回 Hypixel 有效状态，`backend_failures = 0`。
- 公网页面已包含 MOTD/白名单配置，API 文档显示 v0.5.0。

交付产物：

```text
dist/mc-proxy-v0.5.0-ubuntu24-x86_64
SHA-256: bbe69412412f347908d21f3b0b8fa6c57e5b17c68d5145d512b6fa42a21f69d2

dist/mc-proxy-v0.5.0-ubuntu24-x86_64.tar.gz
SHA-256: 150f3a415dc62727b05d9baa136d2e3da3d6b81f144d08df8596ab6dd78e05ee
```

---

## 构建环境

- 构建时间：2026-07-29 11:57（Asia/Shanghai）
- 应用版本：mc-proxy 0.2.0
- 服务器系统：Ubuntu 24.04.4 LTS（Noble）
- 内核：Linux 6.8.0-48-generic
- 架构：x86_64
- Rust：rustc 1.97.1
- Cargo：cargo 1.97.1
- C 链接器：GCC 13.3.0
- Tokio：1.53.1
- Axum：0.8.9
- Nginx：1.24.0

## 源码门禁

执行：

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo build --release
node --check web/app.js
```

结果：

- rustfmt：通过。
- Clippy：0 个警告。
- 单元测试：8 个通过，0 个失败。
- 集成测试：4 个通过，0 个失败。
- JavaScript 语法检查：通过。
- release 构建：通过，启用 thin LTO、单 codegen unit、strip 和 panic abort。

覆盖范围：

- 配置默认值、未知字段、非法规则 ID、公开管理监听、重复监听和范围校验。
- 管理令牌精确比较。
- 活跃连接生命周期与实时上下行字节统计。
- 256 KiB 以上数据双向完整性和 TCP 半关闭。
- 后端连接失败、并发上限拒绝和优雅退出。
- 管理器规则变更持久化。

## 管理 API 与 TCP 联合测试

独立启动 release 进程、Python TCP echo 后端，并通过真实 HTTP API 操作：

1. 无认证读取 `/api/v1/status` 返回 401。
2. 首页和中文 API 文档均能通过 HTTP 返回。
3. 通过 `POST /api/v1/rules` 创建启用规则：
   - 监听：`127.0.0.1:25575`
   - 后端：`127.0.0.1:25576`
4. 客户端发送 77824 字节并执行 TCP 写半关闭。
5. 返回数据逐字节一致。
6. 状态接口实时返回上传 77824 字节、下载 77824 字节。
7. 通过 `PUT` 停用规则，再通过 `DELETE` 删除规则。
8. 管理端与后端退出码均为 0。
9. 18080、25575 和 25576 无残留测试监听。

## 正式部署

- 页面：`https://mc.lic6.top`
- API 文档：`https://mc.lic6.top/docs/api`
- 管理进程：`mc-proxy.service`
- 管理监听：仅 `127.0.0.1:18080`
- 公网入口：Nginx 80/443
- 生产默认规则：`main`，停用状态

生产验证：

- HTTP 返回 301 并跳转 HTTPS。
- HTTPS 首页返回 200。
- 安全头包含 CSP、X-Content-Type-Options、X-Frame-Options、Referrer-Policy 和 Permissions-Policy。
- 生产 API 未认证返回 401。
- 认证后状态返回版本 0.2.0，配置中的管理地址为 `127.0.0.1:18080`。
- `mc-proxy`、Nginx 和 Certbot timer 均为 enabled + active。
- 25565 当前没有监听，避免真实后端未配置前开放无效入口。
- systemd security exposure 为 2.8，评级 OK。
- `admin.env` 权限 0600；`config.toml` 权限 0640。

## HTTPS

- 证书：Let’s Encrypt ECDSA
- 域名：`mc.lic6.top`
- 到期时间：2026-10-27
- Certbot timer：已启用
- `certbot renew --dry-run`：成功

## 交付产物

原始 ELF：

```text
dist/mc-proxy-ubuntu24-x86_64
SHA-256: dc56e0059f53df2e4ce9bc9e4761c6159c130a776d44ccd33fc0df45e1c18abd
```

归档包：

```text
dist/mc-proxy-ubuntu24-x86_64.tar.gz
SHA-256: 1de4a9cb3c6c973116b1f9889dbbb77f90c2dced94d1bb8b796d143dfbd8633e
```

归档内 ELF 为 0755，大小 2378048 字节。

ELF 信息：

```text
ELF 64-bit LSB PIE executable, x86-64, dynamically linked,
interpreter /lib64/ld-linux-x86-64.so.2, for GNU/Linux 3.2.0, stripped
```

## 服务器变更

已安装：

- Rust stable 1.97.1 与 Cargo。
- build-essential。
- Nginx、Certbot、python3-certbot-nginx。

已创建：

- 低权限系统用户 `mc-proxy`。
- `/opt/mc-proxy/`、`/etc/mc-proxy/`。
- systemd 服务、Nginx 站点和 API 限速配置。
- Let’s Encrypt 证书及自动续期 timer。

服务器提示存在待重启的新内核，但本次未重启服务器；应用、Nginx 和证书功能均已在当前内核上通过验证。
