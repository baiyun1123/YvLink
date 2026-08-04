# mc-proxy 交付记录

## 任务标题

为每条域名路由增加独立的 Bedrock Crossplay 允许开关。

## 完成时间

2026-08-04 15:17（Asia/Shanghai）

## 变更内容

- `RuleConfig` 新增 `crossplay_enabled`，默认 `false`；旧 TOML 与既有 API 请求未包含该字段时保持兼容，并默认不允许互通。
- 全局 Crossplay 启用时，`crossplay.java_address` 现在必须匹配一条“路由已启用且 `crossplay_enabled = true`”的规则；不能通过停用、删除或取消允许当前目标路由而让运行中的互通失去上游。
- Web 路由编辑器新增“允许基岩版互通”开关，路由卡片展示允许状态；互通页的 Java 路由候选仅列出已启用且已允许的精确 Host。
- 更新配置示例、README 中英文说明、`CROSSPLAY.md` 与响应式可检索 API HTML 文档，覆盖新增字段、请求示例和校验失败场景。
- 新增配置兼容性与互通路由资格单元测试；本机 Termux Rust 标准库缺失导致本地测试无法启动，改由 Ubuntu 24.04 部署服务器使用 Rust 1.88 进行构建验证。

## 关键决策

- 采用“每条路由显式允许 + 全局互通仍只选择一个 `java_address`”的模型，而非让单个 Bedrock UDP 入口同时自动分流到多条路由；这与 Geyser/GeyserLite 的单 Java 上游工作方式一致，并避免含糊的路由选择。
- 新字段默认关闭，避免升级后任何已有路由在未确认兼容性的情况下自动接受基岩版流量。

## 风险与待办

- 单个 Geyser/GeyserLite 实例仍只能选择一条 Java 路由；需要多个独立 Bedrock 入口时，应部署多个互通实例并使用不同 UDP 端口。
- 全局 Crossplay 已启用时编辑其目标规则会受到资格校验保护；如需切换，应先允许新的路由并更新 `java_address`，或先关闭全局互通。
- 待完成 Ubuntu 构建测试、GitHub 推送及目标服务器二进制更新后，使用控制台实际保存一条允许互通的路由进行验收。

## 关联文件

- `src/config.rs`
- `web/index.html`、`web/app.js`
- `config.example.toml`、`deploy/config.production.toml`
- `README.md`、`CROSSPLAY.md`、`docs/api.html`
- `code.md`

---

## 任务标题（补充）

v0.13.0 Release 构建修复：GeyserLite 托管运行时限定 Linux 目标，Windows 发布包恢复构建并完成 GitHub Release 发布。

## 完成时间

2026-07-31 09:45（Asia/Shanghai）

## 变更内容

- GitHub Actions Windows 任务失败原因：`geyserlite 0.3.19` 上游 `src/config.rs` 无条件使用 `std::os::unix::fs::OpenOptionsExt::mode`，在 Windows 无法编译。
- `Cargo.toml` 将 geyserlite 改为 `[target.'cfg(target_os = "linux")'.dependencies]`，`geyserlite`/`geyserlite-download`/`geyserlite-embed` 特性在非 Linux 目标为空操作。
- `src/geyser_lite.rs` 所有内置实现改用 `all(feature = "geyserlite", target_os = "linux")` 门控；非 Linux 构建的 `runtime.available=false` 并返回平台提示。
- 控制台文案与 README/CROSSPLAY/BUILD_UBUNTU24 文档同步说明“内置 GeyserLite 仅 Linux，Windows 使用 external”。
- 删除空的 v0.13.0 标签与 Release，把标签移动到修复提交（`7ef9c92`）后重新发布；GitHub Actions 全平台通过：Ubuntu 22.04/24.04、Windows 2022、musl x86_64/aarch64 共 5 个安装包已上传。

## 关键决策

- 不 fork/升级 geyserlite 0.4.x 来修复 Windows 编译（需重新评估 API 与生命周期），先用目标限定保住 Windows 发布包；Windows 互通能力由外部 Geyser Standalone 覆盖。
- v0.13.0 无资产且刚创建，直接移动标签比新增 v0.13.1 更干净，避免出现一个空 Release。

## 风险与待办

- geyserlite 0.3.x 的 Windows 编译问题属于上游；升级 0.4.x 后可评估恢复 Windows 内置翻译层。
- 真实基岩客户端登录/游玩矩阵仍待自有后端验收。

## 关联文件

- `Cargo.toml`
- `src/geyser_lite.rs`
- `web/app.js`
- `README.md`、`CROSSPLAY.md`、`BUILD_UBUNTU24.md`

---

## 任务标题

为 mc-proxy 增加 GeyserLite 托管 Bedrock 互通（embedded/subprocess），版本升至 v0.13.0 并在 Ubuntu 24.04 构建验证。

## 完成时间

2026-07-31 09:30（Asia/Shanghai）

## 变更内容

- `Cargo.toml` 新增可选依赖 `geyserlite 0.3`（当前锁定 0.3.19）与特性 `geyserlite`、`geyserlite-download`、`geyserlite-embed`；默认特性为 `geyserlite + geyserlite-download`，`--no-default-features` 可完全移除内置翻译层。
- `crossplay` 新增 `provider = "external" | "geyserlite"`（旧配置默认 external，保持兼容）与 `[crossplay.geyserlite]`：`mode`（embedded/subprocess）、`library_path`、`binary_path`、`offline`、`motd_line1/line2`、`floodgate_key`（16 字节 32 位十六进制，仅 floodgate）。
- 新增 `src/geyser_lite.rs`：托管运行时（嵌入式加载或子进程），启动/停止/热更新生命周期、代次防串扰、失败记录到状态而不是回滚配置；未编译特性时提供无操作实现。
- `GET/PUT /api/v1/crossplay` 返回新增 `runtime`（available/enabled/running/mode/error），PUT 保存配置后按 provider 启动/停止/重启托管翻译层；启动阶段拉起并在退出时优雅停止。
- Web 控制台互通页新增提供方与托管运行时状态行，表单按提供方/模式/认证方式渐进显示 GeyserLite 参数（含 Floodgate 密钥密码框与警告）。
- 配置校验：模式与路径互斥、Floodgate 密钥格式与缺失检查、provider 序列化往返测试（修正 `geyserlite` 被误序列化为 `geyser-lite` 的问题）。
- README（中英）、CROSSPLAY.md、config.example.toml、deploy/config.production.toml、docs/api.html 与 GitHub Actions（MSRV 1.85→1.88）同步更新。
- Ubuntu 24.04 x86_64（64.83.19.35，Rust 1.97.1）：rustfmt、Clippy 零警告、38 个单元测试、18 个集成测试、release 构建、`--no-default-features` 检查全部通过；清理了 7 个历史 `collapsible_if` lint。
- 服务器隔离验收：embedded 自动下载 libgeyserlite.so 后 UDP 19133 真实监听、RakNet Pong 返回配置 MOTD、API `running/online` 均为 true；subprocess 模式下 PUT 热更新 MOTD 生效且 mc-proxy 存活。
- 无数据库结构变更，无需 SQL；`docs/api.html` 已同步跨平台接口文档。

## 关键决策

- embedded 模式与 mc-proxy 共享地址空间，实测“停止后再启动”Geyser 原生桥接会把整个进程带崩，因此已运行的 embedded 实例拒绝配置热更新：保留旧实例并在 `runtime.error` 提示重启 mc-proxy 生效；subprocess 模式支持在线热更新，生产建议先用 subprocess 验收。
- 默认启用 `geyserlite-download` 让 provider 开箱即用：运行时从 GitHub Release 获取原生库并校验 SHA-256；离线生产用 `geyserlite-embed` 特性内嵌，或预置 `GEYSERLITE_LIBRARY`。
- Floodgate 密钥以 32 位十六进制存入本地 TOML 与控制台配置（等同 Geyser 的 key 文件），文档与 UI 均标注敏感性和权限要求；未引入 `config_overrides`，第一版只覆盖 typed 字段。
- 保留 `provider = "external"` 为默认值，既有 Geyser Standalone 部署和监控语义完全不变。

## 风险与待办

- 真实 Windows/Android/iOS 基岩客户端登录、移动、聊天、背包、实体、传送与重连矩阵尚未执行，需要自有 Paper/Fabric 后端后验收。
- GeyserLite 内嵌 Geyser 会在日志输出 log4j/GraalVM 告警，属于上游原生镜像噪音，不影响 UDP 监听与 Pong；崩溃隔离依赖 subprocess 模式。
- `geyserlite` crate 锁在 0.3.19（0.4.x 已发布但未评估），后续升级需重新验证 embedded/subprocess 生命周期与 API 字段。
- v0.13.0 尚未打 Git 标签与创建 GitHub Release；本地 gh 已登录，可在推送后由用户或后续任务创建。

## 关联文件

- `Cargo.toml`、`Cargo.lock`
- `src/config.rs`、`src/geyser_lite.rs`、`src/api.rs`、`src/main.rs`、`src/lib.rs`、`src/proxy.rs`
- `web/index.html`、`web/app.js`
- `config.example.toml`、`deploy/config.production.toml`
- `README.md`、`CROSSPLAY.md`、`docs/api.html`、`BUILD_UBUNTU24.md`
- `.github/workflows/build.yml`
- 服务器：`/root/mc-proxy-v0.13-geyserlite/`（构建目录）、`/root/.cache/geyserlite/`（已校验的原生库缓存）

---

## 任务标题

统一仓库公开许可证说明为 AGPL-3.0-only。

## 完成时间

2026-07-30 14:39（Asia/Shanghai）

## 变更内容

- 删除 README 中文许可证章节中的历史版本授权提示。
- 删除 README 英文 License 章节中的对应历史版本授权提示。
- 精简 `NOTICE`，不再按版本区分许可证，只声明当前仓库源码使用 `AGPL-3.0-only`。
- Cargo 元数据继续保持 `AGPL-3.0-only`，完整 GNU AGPL v3 正文保持不变。
- 无新增或变更后端 HTTP API，`docs/api.html` 无需修改；无数据库结构变更，无需 SQL。

## 关键决策

- 当前仓库的用户可见许可证信息统一指向 `LICENSE` 中的 `AGPL-3.0-only` 标准正文。
- 不改写或附加 GNU 官方许可证正文，避免破坏标准许可证识别。

## 风险与待办

- 删除历史说明只改变当前仓库的展示文本，不会通过技术手段追回第三方已经合法取得的历史副本或既有授权。
- v0.12.0 尚未创建正式标签与 Release；创建时应确认构建包内同时包含非空的 `LICENSE` 和 `NOTICE`。

## 关联文件

- `README.md`
- `NOTICE`
- `code.md`

---

## 任务标题

将 YvLink 后续版本许可证切换为 AGPL-3.0-only 并纳入跨平台发布包。

## 完成时间

2026-07-30 14:24（Asia/Shanghai）

## 变更内容

- 新增 GNU Affero General Public License v3.0 only 完整许可证正文 `LICENSE`。
- 新增 `NOTICE`，注明项目名称 YvLink、版权所有者、许可证生效版本及商标边界。
- 将 Cargo 包版本从 v0.11.0 提升到 v0.12.0，并把许可证元数据由 MIT 改为 SPDX 标识 `AGPL-3.0-only`，避免同一版本对应两套许可证。
- README 中英文同步说明：允许个人、企业和商业使用；分发修改版本或通过网络提供修改版本时，需要按照 AGPL v3 提供对应源代码。
- README 中英文明确记录 v0.11.0 及更早已发布版本仍按原 MIT License 授权，既有授权不能追溯撤销。
- GitHub Actions 的 Ubuntu、Windows 和 musl Linux 打包步骤全部加入 `LICENSE`，确保后续二进制发布包携带完整许可证。
- 所有发布包同时加入 `NOTICE`，让二进制分发保留 YvLink 项目名称、版权、生效版本和历史授权说明。
- 无新增或变更后端 HTTP API，`docs/api.html` 无需修改；无数据库结构变更，无需 SQL。

## 关键决策

- 采用标准 `AGPL-3.0-only`，不附加日活、收入或禁止商业化等额外限制，以维持标准开源许可证属性并适配 Codex for Open Source 申请方向。
- 许可证允许商业化，但修改版本对外分发或通过网络向用户提供服务时，需要履行 AGPL 的对应源代码义务。
- 项目名称与图标的商标边界写入独立 `NOTICE`，不修改 GNU 官方许可证正文。
- 许可证变更从 v0.12.0 向后生效；已按 MIT 发布的历史版本保持原授权。

## 风险与待办

- v0.12.0 尚未创建 Git 标签或正式 GitHub Release；本次推送只建立后续开发版本及许可证基线。
- AGPL 合规结论取决于具体分发、修改和网络服务方式，商业部署方应结合自身场景审查许可证义务。
- 若未来接受外部贡献，建议增加贡献者协议或 Developer Certificate of Origin 流程，明确贡献代码可按项目许可证发布。
- 标准许可证正文已与 SPDX `AGPL-3.0-only` 文本逐字比对；`git diff --check` 通过。
- 本机 Termux 的 Rust 标准库缺少 rlib，`cargo check` 在依赖构建阶段失败；这不是本次文档或元数据变更导致的源码错误。
- GitHub Actions 运行 `30519686172` 最终成功：Ubuntu 22.04、Ubuntu 24.04、Windows Server 2022、musl x86_64 和 musl ARM64 的测试、release 编译、携带许可证的打包及 artifact 上传全部通过。

## 关联文件

- `LICENSE`
- `NOTICE`
- `Cargo.toml`
- `Cargo.lock`
- `README.md`
- `.github/workflows/build.yml`
- `code.md`

---

## 任务标题

补充便携 Linux 构建并将版本标签自动发布为带安装包的 GitHub Release。

## 完成时间

2026-07-30 13:11（Asia/Shanghai）

## 变更内容

- 修正“只推送 Git tag”的交付问题：Git tag 仅提供源码快照，不能替代带平台安装包的 GitHub Release。
- 构建矩阵新增 `x86_64-unknown-linux-musl` 便携 Linux 包，适用于 Alpine 和多数 x86_64 Linux 发行版。
- 构建矩阵新增 `aarch64-unknown-linux-musl` ARM64 便携 Linux 包，适用于 ARM64 服务器和树莓派 64 位系统。
- 保留 Ubuntu 22.04 x86_64、Ubuntu 24.04 x86_64 和 Windows Server 2022 x86_64 原生构建，共生成五个平台包。
- 使用 `cross` 与受校验的 `taiki-e/install-action` 完成交叉编译，避免把 Ubuntu runner 名称误当成全部 Linux 支持范围。
- 工作流新增可选 `release_tag` 手动输入；版本标签触发或手动指定已有标签时，等待全部五个构建成功后创建/更新 GitHub Release。
- Release 发布任务使用 GitHub CLI 校验远端标签、创建正式 Release，并把五个平台归档上传为 Release Assets。
- 将 `actions/checkout` 升级到 Node 24 运行时的 v6，将 artifact 上传/下载升级到 v7，消除旧版 Node 20 弃用警告。
- README 中英文新增下载章节，明确安装包从 GitHub Releases 获取，GitHub 自动生成的 `Source code (zip)` 不是安装包。
- 无新增或变更后端 HTTP API，`docs/api.html` 无需修改；无数据库结构变更，无需 SQL。

## 关键决策

- 不为 Debian、CentOS、Arch 等发行版复制相同的动态链接构建，而是提供 musl 便携包覆盖发行版差异，并按 x86_64/ARM64 区分 CPU 架构。
- Windows 使用 `.zip` 是平台归档格式；Linux 使用 `.tar.gz`，避免把 Actions 下载接口的外层 ZIP 当成用户安装包。
- Release 任务仅在所有原生和便携构建成功后执行，防止发布缺少平台文件的不完整版本。
- 对现有 `v0.11.0` 使用手动工作流输入创建 Release，不强制移动已经推送的 Git 标签。

## 风险与待办

- musl 两个目标需要 GitHub runner 的 Docker 与 cross 镜像；首次运行必须实际验证下载、链接和打包。
- ARM64 包通过交叉编译生成，本轮以编译和二进制产物为验收，不在 x86_64 runner 上模拟完整运行。
- Release Assets 上传使用 `--clobber` 仅覆盖同名构建产物；正式发布后不应手工上传同名不同内容文件。

## 关联文件

- `.github/workflows/build.yml`
- `.github/workflows/pages.yml`
- `README.md`
- `code.md`

---

## 任务标题

将文档品牌更正为 YvLink，并新增 GitHub Pages 与多平台构建工作流。

## 完成时间

2026-07-30 12:56（Asia/Shanghai）

## 变更内容

- 将中英双语 `README.md` 中错误的展示品牌 `MC Relay` 全部更正为 `YvLink`，保留二进制、Cargo 包和 systemd 技术标识 `mc-proxy`。
- 使用内置图片编辑能力将 README 双语架构图中央标签精确改为“YvLink / 智能代理”，保持玩家、后端、控制台、布局和配色不变。
- 将 `docs/api.html` 的浏览器标题和页面品牌更正为 YvLink，并同步设计系统文档的项目名。
- 新增 `.github/workflows/pages.yml`：在 `main` 的 API 文档或图标变更时，把 `docs/api.html` 同时发布为 GitHub Pages 的 `/` 与 `/api.html`。
- Pages 工作流使用最小权限 `contents: read`、`pages: write`、`id-token: write`，并通过 `github-pages` environment 部署。
- 新增 `.github/workflows/build.yml`：在 push、PR、版本标签和手动触发时并行构建 Ubuntu 22.04 x86_64、Ubuntu 24.04 x86_64 与 Windows Server 2022 x86_64。
- 三个平台均安装 Rust 1.85.0，执行锁定依赖测试与 release 构建；Linux 输出 `.tar.gz`，Windows 输出 `.zip`，Actions 构建产物保留 14 天。
- README 中英文部分均增加在线 API 文档地址 `https://baiyun1123.github.io/YvLink/`。
- 无新增或变更后端 HTTP API，只变更现有 API 文档的品牌与部署方式；无数据库结构变更，无需 SQL。

## 验证结果

- README、API 文档和设计系统主文档中已无 `MC Relay` 品牌残留。
- 最终架构图确认为有效的 1672×941、8-bit RGB PNG，图片中文字为“YvLink / 智能代理”。
- `cargo fmt --check`、Cargo 元数据解析和 `node --check web/app.js` 通过。
- 本机未安装 PyYAML、Ruby YAML 或 Perl YAML 模块；推送后两个工作流均已被 GitHub Actions 正确解析。
- GitHub Pages 已通过 API 启用 `workflow` 发布源，Pages 构建与部署成功；在线地址返回 HTTP 200，页面标题和主标题均为 YvLink。
- Ubuntu 22.04 与 Windows Server 2022 的 Rust 1.85 锁定依赖测试、release 编译、压缩和 artifact 上传全部成功。
- Ubuntu 24.04 首次运行在格式步骤失败，日志确认原因是 `--profile minimal` 未安装 `rustfmt`，不是源码格式错误；工作流已显式增加 Rust 1.85 的 `rustfmt` 组件。

## 关键决策

- 品牌名与技术标识分离：用户可见名称使用 YvLink，现有可执行文件、配置路径、环境变量和服务名继续使用 `mc-proxy`，避免破坏部署兼容性。
- API 文档不复制维护第二份源码；Pages 构建阶段将同一个 `docs/api.html` 复制为站点首页和 `/api.html`，防止内容漂移。
- 采用 GitHub 官方 Pages Actions 和构建产物 Actions；构建矩阵使用 GitHub 托管 runner，不需要仓库密钥。
- 首轮跨平台范围选择两个常用 Ubuntu LTS runner 与 Windows Server 2022；不加入未经交叉编译验证的 ARM 或 musl 目标。

## 风险与待办

- 需要检查加入 `rustfmt` 组件后的 Ubuntu 24.04 重跑结果，确保三个平台最终全部为成功。
- Windows 包仅能在 GitHub Windows runner 完成真实编译验证，本机 Android/Termux 无法替代该环境。
- Actions 构建产物默认保留 14 天；若需要永久下载，应在后续版本标签流程中自动创建 GitHub Release 并上传产物。
- 历史 `dist/` 发布包保留其当时的文档和二进制内容，没有回写旧版本品牌。

## 关联文件

- `.github/workflows/pages.yml`
- `.github/workflows/build.yml`
- `README.md`
- `docs/api.html`
- `design-system/mc-relay-control/MASTER.md`
- `assets/readme-architecture-bilingual.png`
- `code.md`

---

## 任务标题

重写中英双语 README，并生成匹配项目图标风格的双语架构配图。

## 完成时间

2026-07-30 12:30（Asia/Shanghai）

## 变更内容

- 将原有中文 `README.md` 重构为同页中英双语文档，顶部提供中文、English、API 文档、模组兼容和跨平台互通的快速导航。
- 复用 `assets/icon.png` 作为项目主图标，并为图片补充中英双语替代文本。
- 新增完整的中文与英文项目简介、功能清单、工作原理、环境要求、快速启动、配置示例、关键字段、生产部署、端口、防火墙、验证命令、兼容边界、性能和许可证说明。
- 启动说明同时覆盖 release 二进制与 `cargo run`，明确 `MC_PROXY_ADMIN_TOKEN` 至少 32 个字符，并给出安全随机令牌生成方法。
- 生产说明关联仓库现有 Nginx、API 文档、systemd、Ubuntu 24.04、Geyser 和真实模组矩阵资料，没有虚构新的部署文件或能力。
- 使用内置图片生成能力，以现有项目图标为视觉参考生成 1672×941 的中英双语架构图 `assets/readme-architecture-bilingual.png`。
- 配图最初准确包含“玩家 / Players”“MC Relay / 智能代理”“后端服务器 / Backends”“管理控制台 / Control Panel”，并在第二轮将类似游戏角色的人物替换为原创抽象终端节点；品牌随后按用户要求更正为 YvLink。
- 无新增或变更后端 HTTP API，`docs/api.html` 无需修改；无数据库结构变更，无需 SQL。

## 验证结果

- README 引用的本地图片、配置、部署、API、兼容性和测试手册文件均存在。
- `assets/readme-architecture-bilingual.png` 已确认为有效的 1672×941、8-bit RGB PNG。
- `node --check web/app.js` 通过。
- `cargo fmt --check` 通过。
- `cargo test --all-targets` 未能在当前 Android/Termux 环境完成：本机 Rust 工具链缺少可链接的 `std`、`core`、`alloc` 等 `rlib` 标准库产物，依赖构建阶段即失败；本次仅改文档和图片，未改 Rust 源码。

## 关键决策

- 中英文放在同一个 README 并通过页内锚点切换，避免两份文档长期漂移。
- 保留现有项目的精确版本、配置字段和能力边界，尤其不把薄代理描述为在线认证终止器、协议版本转换器或内置 Bedrock 翻译器。
- 架构图直接使用中英双语标签，兼顾中文用户、英文用户和图片被单独引用时的可理解性。
- 生成图采用项目图标已有的深蓝、青色、像素网络风格；最终版本不包含可识别的第三方游戏人物或商标。
- 生产管理端继续推荐只监听回环地址并由 Nginx 提供 HTTPS，避免 README 示例诱导用户直接暴露管理 API。

## 风险与待办

- 建议在 Ubuntu 24.04 或标准 Rustup Linux 环境再次执行 `cargo test --all-targets` 和 Clippy，排除当前 Termux Rust 标准库安装异常。
- `assets/icon.png` 的文件内容实际为 JPEG，虽然浏览器通常可按文件签名正常显示，但后续若用于要求严格 PNG MIME 的发布流程，建议另行无损转码并统一文件格式。
- README 中的 `<你的仓库地址>` / `<your-repository-url>` 需要在公开仓库地址确定后替换。

## 关联文件

- `README.md`
- `assets/icon.png`
- `assets/readme-architecture-bilingual.png`
- `code.md`

---

## 任务标题

在用户接受 EULA 后完成 Fabric、Forge、NeoForge 真实服务端前段协议矩阵。

## 完成时间

2026-07-30 05:00（Asia/Shanghai）

## 变更内容

- 按用户明确授权，将三套隔离测试实例的 `eula.txt` 精确设置为 `eula=true`，随后再次读取核验。
- 依次启动 Minecraft 1.21.1 Fabric Loader 0.19.3、Forge 52.1.16 和 NeoForge 21.1.244，所有 JVM 最大堆 512 MiB 且仅监听回环地址。
- 修复真实矩阵执行器把“TCP 已监听”误判成“Forge 已完成启动”的问题；后端就绪现在要求成功完成 Minecraft Status 协议，并把启动期 EOF 作为可重试状态。
- 三套单项矩阵与最终连续全量矩阵均通过：直连、透明代理和后端状态托管的 Status JSON 完整相等。
- Forge `forgeData`、NeoForge `isModded` 等加载器扩展字段经代理完整保留。
- 三套 Login Success 与 Login Acknowledged 后首个 Configuration 包的 ID、长度、SHA-256 在直连和代理链路一致。
- 三套主动健康检查均验证后端在线为 `healthy`、停止后为 `unhealthy`。
- 保存合并报告 `tests/modded-matrix-summary.json`，其 SHA-256 为 `8b20399dd6319a2550f61d7c6ed11326ae52cf67c234274426e2f67541c50dbf`。
- 测试结束后确认全部临时端口无残留监听，生产 `mc-proxy.service` 仍为 `active`。
- 无新增或变更后端 HTTP API，`docs/api.html` 无需修改；无数据库结构变更，无需 SQL。

## 关键决策

- 加载器进程打开 TCP 监听端口不代表 Minecraft 协议已经可用；真实 Status 成功才是可复现的就绪条件。
- 当前结论限定为真实服务端登录前段与 Configuration 首包透明性，不夸大为完整游戏客户端进服、在线认证或复杂模组包兼容。
- EULA 的修改仅基于本次用户明确授权；执行器继续保持只读检查，后续不会自动代签。

## 风险与待办

- 尚需真实游戏客户端完成进服、保持会话、Play 阶段和退出流程测试。
- 需要加入带必需网络握手的代表性 Fabric/Forge/NeoForge 模组组合，并覆盖多个 Minecraft/加载器版本。
- Velocity modern forwarding、BungeeCord/BungeeGuard、在线模式认证终止和完整 Forge/NeoForge Configuration 状态机仍未实现。

## 关联文件

- `tests/run_modded_matrix.py`
- `tests/MODDED_MATRIX_RUNBOOK.md`
- `tests/modded-matrix-summary.json`
- `MODDED_COMPATIBILITY.md`
- `BUILD_UBUNTU24.md`
- `code.md`
- 服务器：`/opt/mc-proxy-matrix/results/summary.json`

---

## 任务标题

实现不代签 EULA 的真实模组服务端矩阵执行器与验收手册。

## 完成时间

2026-07-30 04:14（Asia/Shanghai）

## 变更内容

- 新增 `tests/run_modded_matrix.py`，可按 Fabric、Forge、NeoForge 或全部模式逐套启动真实服务端与隔离 mc-proxy。
- 执行器启动前强制读取对应 `eula.txt`；不是明确的 `eula=true` 就立即失败，不写入 EULA、不启动 JVM、不绑定测试端口。
- 每套实例自动生成仅回环监听的 `server.properties`，限制最大堆 512 MiB、视距/模拟距离 2、平坦世界，并关闭公网认证，仅用于回环协议测试。
- 为每个加载器同时生成透明转发路由和 `status.mode=backend` 路由，使用 Minecraft 1.21.1 协议号 767。
- 验收直连、透明转发、后端状态托管三份 Status JSON 完整相等，未知的 Forge/NeoForge 扩展也必须原样保留。
- 新增现代 Login Start 探针；Forge/NeoForge 使用 `\0FORGE` Host 标记，比较直连和代理首个 Login/Configuration 响应的包 ID、长度与 SHA-256。
- 验收主动 `minecraft-status` 健康检查从 `healthy` 到后端停止后的 `unhealthy` 状态转换。
- 每套结果写入独立 JSON、服务端日志、代理日志，汇总结果只有全部通过才输出 `passed=true`。
- 新增 `tests/MODDED_MATRIX_RUNBOOK.md`，记录安全边界、版本、端口、命令和通过标准。
- 本机 Python 语法检查、Status/Pong socket 夹具通过；服务器执行器在 EULA 未接受时正确拒绝，所有矩阵端口均未监听，生产服务保持 `active`。
- 无新增或变更后端 HTTP 接口，`docs/api.html` 无需修改；无数据库结构变更，无需 SQL。

## 关键决策

- 真实矩阵必须比较协议输出和健康状态，不能把“进程启动”或“端口可连接”当作兼容通过。
- 执行器只验证薄代理应保证的透明性，不把一个合成 Login 首包测试夸大为完整客户端游玩或复杂模组包通过。
- EULA 检查发生在任何运行态写入和进程启动之前，确保自动继续也不会越过用户法律授权。
- 测试配置使用回环离线模式，避免把无认证模组服暴露到公网；生产配置和 systemd 服务完全不修改。

## 风险与待办

- 仍等待用户明确接受 Minecraft EULA；未授权前不能生成真实加载器运行结果。
- 接受后先执行 Loader 本体矩阵，再安装代表性 Fabric/Forge/NeoForge 网络模组进行客户端必需握手测试。
- 完整 Configuration 阶段客户端状态机、Velocity modern forwarding 和在线认证终止仍属于后续工作。

## 关联文件

- `tests/run_modded_matrix.py`
- `tests/MODDED_MATRIX_RUNBOOK.md`
- `tests/status_backend_fixture.py`
- `MODDED_COMPATIBILITY.md`
- `code.md`
- 服务器：`/opt/mc-proxy-matrix/run_modded_matrix.py`
- 服务器：`/opt/mc-proxy-matrix/MODDED_MATRIX_RUNBOOK.md`

---

## 任务标题

准备 Fabric、Forge、NeoForge 真实服务端兼容矩阵并停在 EULA 授权边界。

## 完成时间

2026-07-30 04:02（Asia/Shanghai）

## 变更内容

- 核验测试服务器具备 2 核 CPU、约 1.5 GiB 可用内存、2 GiB Swap、36 GiB 可用磁盘和 64 位 OpenJDK 21。
- 从官方 Fabric Meta、MinecraftForge Maven 和 NeoForged Maven 元数据选择 Minecraft 1.21.1 当前版本：Fabric Loader 0.19.3 / Installer 1.1.2、Forge 52.1.16、NeoForge 21.1.244。
- 官方安装器及其 Maven SHA-1 全部校验通过。
- 三套服务端分别安装到 `/opt/mc-proxy-matrix/fabric-1.21.1`、`forge-1.21.1`、`neoforge-1.21.1`，互相隔离且不修改生产 mc-proxy。
- 三套服务端都已首次运行并正常停在 Mojang EULA 检查点，生成的 `eula.txt` 均保持 `eula=false`。
- 计划采用逐个启动、仅绑定回环地址、每实例最多 512 MiB 的方式测试，避免 2 核/2 GiB 服务器并行运行三套 JVM。
- 无数据库结构变更，无需 SQL。

## 关键决策

- “真实矩阵”必须以实际加载器服务端为证据，现有模拟字节夹具只能作为回归测试，不能替代真实服务端启动和协议探测。
- 接受 Minecraft EULA 属于用户法律授权；自动化只生成官方 `eula.txt` 并停在 `false`，不会代替用户改为 `true`。
- 测试实例只允许回环监听并顺序运行，不开放公网端口，也不影响生产 `hyp.mc.lic6.top`。

## 风险与待办

- 等待用户明确确认接受 <https://aka.ms/MinecraftEULA> 后，才能把三份 `eula.txt` 改为 `true` 并继续真实启动。
- 授权后依次完成直连 Status/Ping、经隔离 mc-proxy Status/Ping、状态扩展保真、离线登录前段和停止/恢复健康检查。
- 加载器本体运行通过仍不等于复杂模组包兼容；后续还需加入代表性必需客户端/服务端网络模组和真实客户端。

## 关联位置

- 服务器：`/opt/mc-proxy-matrix/installers`
- 服务器：`/opt/mc-proxy-matrix/fabric-1.21.1`
- 服务器：`/opt/mc-proxy-matrix/forge-1.21.1`
- 服务器：`/opt/mc-proxy-matrix/neoforge-1.21.1`
- `MODDED_COMPATIBILITY.md`

---

## 任务标题

实现 Minecraft Status 协议级主动健康检查并编译测试部署 v0.11.0。

## 完成时间

2026-07-30 03:50（Asia/Shanghai）

## 变更内容

- 在原有 TCP 可达性探测之外新增 `minecraft-status` 模式，完整发送 Java Handshake、Status Request 与 Ping，并校验 Status JSON 基础结构及 Pong 原值。
- 路由健康配置新增 `mode`、可选 `minecraft_host` 和 `minecraft_protocol`；旧配置缺失字段时默认继续使用 TCP 模式，保持向后兼容。
- Status 探测自动从精确路由 Host、`modify_virtual_host` 或当前后端地址推导握手 Host，并支持手工覆盖；通配符不会被误发给后端。
- 整次连接、状态响应和 Pong 共用单次超时，避免只限制 TCP connect 后被半开或卡死服务长期占用任务。
- 健康探测复用路由的 PROXY Protocol v1/v2 设置，使明确要求该头的受信任后端也能完成应用层探测。
- 保持健康/未知节点优先、异常节点排后以及“全部异常仍进行真实连接尝试”的安全降级语义。
- 新增有效 Minecraft Status、无效 JSON、可连接 HTTP 伪后端、自动 Host 推导及无玩家调度的单元/集成测试。
- 新增可重复运行的 `tests/status_backend_fixture.py` 与 `tests/status-healthcheck.standalone.toml` 独立验收夹具。
- 控制台路由编辑器新增探测模式、Host 和协议号字段，并按模式渐进显示；路由卡片直接显示当前主动探测类型。
- 按 `ui-ux-pro-max` 保持可见标签、44px 控件、文字与颜色双重状态、键盘焦点、移动端单列及减少动态效果支持。
- 中文 README、模组兼容矩阵、响应式可检索 API 文档和示例/生产配置已同步 v0.11.0。
- 无数据库结构变更，无需 SQL。

## 验证结果

- Ubuntu 24.04 x86_64、Rust/Cargo 1.97.1：31 个单元测试与 18 个集成测试全部通过。
- `cargo fmt --check`、Clippy `-D warnings`、release 构建和 `node --check web/app.js` 全部通过。
- 独立运行态：HTTP 伪后端为 `unhealthy`，Minecraft Status 后端为 `healthy`；停止有效后端后自动转为 `unhealthy`。
- 生产 systemd 为 `active`，API 返回 v0.11.0；`hyp` 路由健康检查保持关闭并补齐新字段默认值。
- 回环和服务器公网 IP 的 `hyp.mc.lic6.top` Status/Ping 均成功，生产后端失败与转发失败均为 0。
- 公网页面加载 `20260730-statusprobe1`，公网中文 API 文档显示 v0.11.0。

## 关键决策

- 保留 `tcp` 模式用于只需低成本端口探测的场景，把更可靠但会产生协议请求的 `minecraft-status` 作为显式选择；新建规则 UI 默认推荐后者。
- Status 成功要求基础原版字段存在，但允许 `forgeData`、`modinfo` 及其他未知字段，避免排斥 Fabric、Forge 或 NeoForge 状态扩展。
- 协议探测只证明服务器列表协议健康，不声称登录、Mojang 认证或模组协商成功；真实加载器矩阵仍需继续执行。
- 生产 `hyp` 指向第三方服务器，因此不擅自开启周期主动探测，只升级能力和控制台。

## 风险与待办

- 下一阶段搭建自有 Fabric、Forge 和 NeoForge 实例，执行真实客户端/服务端登录、Configuration 与插件消息矩阵。
- 完整 Velocity modern forwarding、BungeeGuard、在线认证终止及 Forge 会话中继仍未实现。
- Geyser 已安装但因没有自有 Java 后端仍保持未启用，避免伪造“基岩互通已完成”。

## 关联文件

- `src/config.rs`
- `src/proxy.rs`
- `src/server.rs`
- `src/lib.rs`
- `tests/proxy_integration.rs`
- `tests/status_backend_fixture.py`
- `tests/status-healthcheck.standalone.toml`
- `web/index.html`
- `web/app.js`
- `web/styles.css`
- `config.example.toml`
- `deploy/config.production.toml`
- `README.md`
- `MODDED_COMPATIBILITY.md`
- `docs/api.html`
- `BUILD_UBUNTU24.md`

---

## 任务标题

实现主动后端健康检查、健康感知选路与可视化，并编译测试部署 v0.10.0。

## 完成时间

2026-07-30 03:30（Asia/Shanghai）

## 变更内容

- 路由新增 `health_check` 配置：启用状态、检查间隔、连接超时、连续离线阈值与连续恢复阈值，旧配置默认关闭并保持兼容。
- 后端运行态新增未知、健康、离线三态，以及最近检查时间、探测延迟、连续成功/失败和累计成功/失败。
- 无玩家连接时，服务端每秒调度到期的 TCP 探测；首次启动立即检查，单次最多并发 64 个。
- 同一后端探测使用原子 in-flight 标记去重；任务被取消或服务退出时通过 Drop 释放，避免永久停止探测。
- 选路先执行顺序、随机、轮询、最少连接或最低延迟策略，再稳定地把已确认离线节点排到健康/未知节点之后；全部离线时不清空候选。
- 修正最低延迟策略把“尚无延迟样本”误排到已有样本之前的问题；主动探测延迟参与 EWMA 连接延迟。
- 新增阈值状态转换、恢复、候选顺序、探测去重、成功/失败和无玩家调度器端到端测试。
- 控制台新增健康检查配置区、实例累计计数与逐后端状态列表，明确显示最近检查、延迟和连续状态。
- UI 按 `ui-ux-pro-max` 审计并修正所有路由操作按钮至少 44px、文字与颜色双重状态、渐进披露、动态视口和 reduced-motion。
- 中文 API 文档、README、示例配置、兼容矩阵与 Ubuntu 构建报告同步更新。
- Ubuntu 24.04 上 28 个单元测试、16 个集成测试、rustfmt、Clippy 零警告和 release 构建全部通过。
- 服务器隔离实例真实验证健康→离线状态转换后部署；生产 Hypixel 路由保持检查关闭，公网 Status/Ping 正常且失败指标为 0。

## 关键决策

- Gate Lite 官方采用即时选择和自然故障转移，不做主动健康检查；本项目将主动检查作为显式可选增强，不改变默认行为。
- 使用 TCP connect 而不是伪造 Minecraft 登录：跨原版/Fabric/Forge/NeoForge 通用且副作用低，但只证明端口可达，不能证明协议或模组协商健康。
- 离线节点只降级而不永久剔除；健康检查误判或全部后端离线时，真实玩家连接仍有机会成功并自然故障转移。
- 生产当前后端是第三方 Hypixel，不持续主动探测；启用态使用服务器回环隔离夹具验证。
- 健康状态属于运行态，配置重载后重新从 unknown 首检，不写入 TOML。

## 风险与待办

- TCP 可达性无法识别主线程卡死、Minecraft Status 异常或模组握手失败；下一步可增加可选 Minecraft Status 健康模式。
- 真实 Fabric/Forge/NeoForge 仍缺自有服务端和客户端的完整登录/游玩矩阵。
- 完整代理模式仍缺在线认证终止、Velocity/Bungee 玩家信息转发、FML Login 中继与 Configuration 状态机。
- root 密码曾在聊天中明文提供，建议立即轮换并改用 SSH 密钥。

## 关联文件

- `src/config.rs`
- `src/server.rs`
- `src/metrics.rs`
- `src/lib.rs`
- `tests/proxy_integration.rs`
- `tests/healthcheck.standalone.toml`
- `web/index.html`
- `web/styles.css`
- `web/app.js`
- `config.example.toml`
- `deploy/config.production.toml`
- `docs/api.html`
- `README.md`
- `MODDED_COMPATIBILITY.md`
- `BUILD_UBUNTU24.md`
- `dist/mc-proxy-v0.10.0-ubuntu24-x86_64/`
- `dist/mc-proxy-v0.10.0-ubuntu24-x86_64.tar.gz`
- `dist/SHA256SUMS`
- `/opt/mc-proxy/mc-proxy`
- `/var/backups/mc-proxy/mc-proxy.pre-v0.10.0-20260730`
- `/var/backups/mc-proxy/config.pre-v0.10.0-20260730.toml`
- `code.md`

---

## 任务标题

实现可信后端 PROXY Protocol v1/v2、优化控制台 UI，并编译测试部署 v0.9.0。

## 完成时间

2026-07-30 03:06（Asia/Shanghai）

## 变更内容

- 每条路由新增 `proxy_protocol = "off" | "v1" | "v2"`，默认关闭；TOML 同时兼容 Gate Lite 风格的 `true`（v1）与 `false`（off）。
- PROXY 头在任何改写或原始 Minecraft 握手数据前写入；普通游戏转发和 `status.mode = "backend"` 查询使用相同行为。
- v1 实现 IPv4/IPv6 文本头和混合地址族 UNKNOWN；v2 实现标准签名、PROXY 命令、TCP4/TCP6 地址块和 UNSPEC。
- PROXY 头写入失败计入后端尝试失败并继续候选池；成功发送分别累加 v1/v2 指标。
- 新增精确字节编码、真实 socket 首包顺序、Minecraft 握手保真和后端 Status 缓存不重复发头测试。
- 控制台路由编辑器新增 off/v1/v2 选择、后端兼容与防火墙安全警告；路由卡片和实例状态展示当前模式及累计发送量。
- UI 按 `ui-ux-pro-max` 复核 44px 控件、可见标签、键盘焦点、提交加载反馈、reduced-motion、z-index 和移动端动态视口。
- 中文 API 文档、README、配置示例、生产模板与模组兼容矩阵同步更新。
- Ubuntu 24.04 x86_64 上 rustfmt、24 个单元测试、15 个集成测试、Clippy 零警告和 release 构建全部通过。
- 备份 v0.8.0 后部署到生产；API 返回 v0.9.0，`hyp` 路由保持 `proxy_protocol = "off"`，Hypixel Status/Ping 正常且失败指标为 0。

## 关键决策

- PROXY Protocol 是后端监听器双方约定，不做自动探测；普通 Minecraft 后端收到前置头会握手失败，因此生产默认关闭。
- 当前 Hypixel 后端不受本项目控制，也未声明接收 PROXY Protocol，不能用它做启用态生产测试；启用态由 IPv4/IPv6 与真实 socket 集成夹具验证。
- v2 只发送标准地址块，不附加未经配置的 TLV；v1/v2 都传递客户端连接到代理入口时的源/目标地址。
- 真实 IP 传递不等于 Velocity/Bungee 玩家信息转发，后端仍需限制只允许代理访问，否则来源地址信任模型可被绕过。
- UI 使用持续可见的安全说明，而不是只在提交后报错，避免管理员误把 v1/v2 当作通用加速选项。

## 风险与待办

- 下一阶段仍需主动后端健康检查，避免只有真实玩家或状态查询触发失败判定。
- 仍需自有 Fabric/Forge/NeoForge/Paper 后端完成真实客户端矩阵，并在支持 PROXY 的监听器上补一次生产同构验收。
- 完整会话代理仍缺在线认证终止、Velocity/Bungee 转发、FML 中继与 Configuration 状态机。
- root 密码曾在聊天中明文提供，建议立即轮换并改用 SSH 密钥。

## 关联文件

- `src/config.rs`
- `src/proxy.rs`
- `src/metrics.rs`
- `src/server.rs`
- `src/lib.rs`
- `tests/proxy_integration.rs`
- `web/index.html`
- `web/styles.css`
- `web/app.js`
- `docs/api.html`
- `config.example.toml`
- `deploy/config.production.toml`
- `MODDED_COMPATIBILITY.md`
- `README.md`
- `BUILD_UBUNTU24.md`
- `dist/mc-proxy-v0.9.0-ubuntu24-x86_64/`
- `dist/mc-proxy-v0.9.0-ubuntu24-x86_64.tar.gz`
- `dist/SHA256SUMS`
- `/opt/mc-proxy/mc-proxy`
- `/var/backups/mc-proxy/mc-proxy.pre-v0.9.0-20260730`
- `/var/backups/mc-proxy/config.pre-v0.9.0-20260730.toml`
- `code.md`

---

## 任务标题

建立 Fabric/Forge/NeoForge 薄代理协议矩阵与握手可观测能力，并编译测试部署 v0.8.0。

## 完成时间

2026-07-30 02:43（Asia/Shanghai）

## 变更内容

- 参考 Gate 官方实现，识别 NUL 分隔的旧版 `FML`、现代 `FML2/FML3` 和 1.20.2+ `FORGE/FORGE2` Host 标记；路由域名仍只取第一个 NUL 之前的部分。
- 新增无标记、旧 Forge、现代 Forge Login 和 Configuration 系四类握手原子指标，自动进入状态 API、周期日志和运行时聚合。
- `modifyVirtualHost` 新增表驱动测试，确认替换基础域名时完整保留 `FML`、`FML2`、`FML3`、`FORGE` 与 `FORGE2` 后缀。
- 新增 Fabric 1.20.1、Forge 1.12.2、Forge 1.16.5、Forge 1.20.1、NeoForge 1.21.1 和 FORGE NAT 六组双向协议夹具。
- 每组夹具都验证客户端到后端、后端到客户端的 Login Plugin Message、Configuration 自定义负载和任意二进制数据逐字节不变。
- 控制台运行概览新增响应式“协议握手观察”面板，并明确说明 Fabric 与原版无法在初始 Handshake 中可靠区分、指标不等于成功登录。
- 中文 API 文档同步新增四个状态字段、语义、限制和 v0.8.0 示例；兼容矩阵将模拟协议证据与真实客户端验收明确分开。
- Ubuntu 24.04 x86_64 上 rustfmt、Clippy 零警告、20 个单元测试、13 个集成测试和 release 构建全部通过。
- 备份 v0.7.0 后部署到生产；对 `hyp.mc.lic6.top` 发送六种真实公网 Status 握手，全部返回 Hypixel 有效 Status 与 Ping/Pong。
- 生产指标准确记录 1 个无标记、1 个旧 Forge、2 个 FML2/FML3、2 个 FORGE/FORGE2 握手，后端与转发失败均为 0。

## 关键决策

- 本版保持薄代理架构，不为了“识别模组”而终止加密会话或解析后续流量；否则会引入在线认证、压缩、状态机和密钥管理的完整代理复杂度。
- Fabric 没有统一初始 Host 标记，不能根据无标记握手断言客户端一定是 Fabric，因此 UI 使用“原版 / Fabric”合并统计。
- 协议夹具只证明代理不会改写模拟字节流，不替代真实加载器、具体模组包和服务端组合的端到端验收。
- `FORGE` 前缀包括可选 NAT 版本号，按 Gate 的行为归入 Configuration 系，避免把 `FORGE2` 漏判为普通连接。
- UI 沿用 `ui-ux-pro-max` 现有密集型运维设计系统，四类指标使用独立响应式面板，避免继续拉长实例故障列表。

## 风险与待办

- 仍需自有后端完成真实 Fabric、Forge 1.12.2、Forge 1.16.5、Forge 1.20.1、Forge/NeoForge 1.20.2+ 客户端登录与游玩矩阵。
- 完整会话代理模式仍缺在线认证终止、Velocity/Bungee 玩家信息转发、FML Login 中继/重放和 Configuration 状态机。
- 下一阶段可先实现可信内网后端的 PROXY Protocol v1/v2 与主动健康检查，再准备容器化模组后端矩阵。
- root 密码曾在聊天中明文提供，建议立即轮换并改用 SSH 密钥。

## 关联文件

- `src/proxy.rs`
- `src/metrics.rs`
- `src/server.rs`
- `tests/proxy_integration.rs`
- `web/index.html`
- `web/styles.css`
- `web/app.js`
- `docs/api.html`
- `MODDED_COMPATIBILITY.md`
- `README.md`
- `Cargo.toml`
- `Cargo.lock`
- `BUILD_UBUNTU24.md`
- `dist/mc-proxy-v0.8.0-ubuntu24-x86_64/`
- `dist/mc-proxy-v0.8.0-ubuntu24-x86_64.tar.gz`
- `dist/SHA256SUMS`
- `/opt/mc-proxy/mc-proxy`
- `/var/backups/mc-proxy/mc-proxy.pre-v0.8.0-20260730`
- `/var/backups/mc-proxy/config.pre-v0.8.0-20260730.toml`
- `code.md`

---

## 任务标题

实现多后端故障转移与 Java/基岩互通管理模式，优化控制台 UI，并编译测试部署 v0.7.0。

## 完成时间

2026-07-30 02:26（Asia/Shanghai）

## 变更内容

- 路由后端由单地址升级为最多 128 个地址，旧版字符串配置保持兼容；新增顺序、随机、轮询、最少连接和最低延迟五种策略。
- Java 登录和后端 Status 均支持逐后端故障转移；新增后端尝试失败、成功切换、活动连接、成功/失败次数和 EWMA 连接延迟指标。
- 新增 `[crossplay]` 配置、`GET/PUT /api/v1/crossplay` 和真实 RakNet Unconnected Ping/Pong 健康探测，明确区分“已配置”和“Geyser 在线”。
- 增加独立“互通模式”页面：展示 Bedrock UDP 入口、Java 目标、认证模式、延迟与故障信息，并提供兼容性提示和带提交反馈的配置表单。
- 路由编辑器新增多后端文本输入和负载均衡策略，路由卡片展示每个后端的连接健康与故障转移状态。
- 更新响应式中文 API 文档、README、示例配置、模组兼容矩阵，并新增 `CROSSPLAY.md` 和 Geyser systemd 模板。
- 新增多后端 TCP/Status 故障转移、策略选择、配置兼容、健康指标、Crossplay 校验和模拟 RakNet 端点测试。
- 版本升级为 v0.7.0；Ubuntu 24.04 x86_64 上 rustfmt、Clippy 零警告、18 个单元测试、12 个集成测试和 release 构建全部通过。
- 生产临时路由以无效地址作为第一后端、Hypixel 作为第二后端，真实验证第二后端成功接管，`backend_attempt_failures = 1`、`backend_failovers = 1`；临时路由随后删除。
- 服务器安装 Java 21 与 Geyser Standalone 2.11.0 build 1205；由于当前只有不受控制的 Hypixel 后端，Geyser 服务保持禁用，避免对外暴露不可用的 Bedrock 入口。
- 备份 v0.6.0 生产文件并部署至 `64.83.19.35`；systemd、Java 25565、管理端 18080、公网页面、API 文档和生产 Hypixel Status 均正常。

## 关键决策

- Geyser 作为独立 UDP→Java 协议翻译器运行，mc-proxy 继续专注于 Java TCP 域名选路；互通健康状态通过 RakNet 实测而非进程名推断。
- 互通模式只允许指向能被现有域名路由命中的 Java 目标，且 Java 端口必须与代理监听端口一致，防止保存无法到达的配置。
- 未取得自有、版本兼容的 Java 后端前不启用 UDP 19132；“组件已安装”和“服务可用”在 UI 中分别表达。
- UI 遵循 `ui-ux-pro-max` 的渐进披露、44px 控件、键盘焦点、加载反馈、响应式断点和 reduced-motion 规则。

## 风险与待办

- 需要准备自有 Paper/Fabric 测试后端，选择 `online` 或 Floodgate 认证后，才能开启 Geyser 并做真实 Windows/Android/iOS 基岩客户端登录与游玩矩阵。
- Geyser 只解决基岩客户端到 Java 协议的兼容；依赖 Java 客户端模组界面的 Fabric/Forge/NeoForge 玩法不能自动转换。
- 后续继续完成真实 Fabric/Forge/NeoForge 版本矩阵、PROXY Protocol 和主动后端健康检查。
- root 密码曾在聊天中明文提供，建议立即轮换并改用 SSH 密钥。

## 关联文件

- `src/config.rs`
- `src/proxy.rs`
- `src/crossplay.rs`
- `src/manager.rs`
- `src/api.rs`
- `src/metrics.rs`
- `src/server.rs`
- `tests/proxy_integration.rs`
- `web/index.html`
- `web/styles.css`
- `web/app.js`
- `docs/api.html`
- `config.example.toml`
- `CROSSPLAY.md`
- `MODDED_COMPATIBILITY.md`
- `deploy/geyser/mc-proxy-geyser.service`
- `README.md`
- `BUILD_UBUNTU24.md`
- `dist/mc-proxy-v0.7.0-ubuntu24-x86_64/`
- `dist/mc-proxy-v0.7.0-ubuntu24-x86_64.tar.gz`
- `dist/SHA256SUMS`
- `/opt/mc-proxy/mc-proxy`
- `/opt/mc-proxy/geyser/Geyser-Standalone.jar`
- `/etc/systemd/system/mc-proxy-geyser.service`
- `/var/backups/mc-proxy/mc-proxy.pre-v0.7.0-20260730`
- `/var/backups/mc-proxy/config.pre-v0.7.0-20260730.toml`
- `code.md`

---

## 任务标题

实现模组安全的后端 Status 覆盖、缓存与离线 fallback，并编译测试部署 v0.6.0。

## 完成时间

2026-07-30 01:48（Asia/Shanghai）

## 变更内容

- 每条路由的 `status` 新增 `custom` 与 `backend` 两种来源；后端模式读取真实 Status JSON，只覆盖显式填写的 MOTD、版本、协议号和展示人数。
- 后端 Status 的 `forgeData`、旧版 `modinfo`、favicon、玩家 sample 和未知扩展字段保持原样，避免模组客户端因代理自定义 MOTD 丢失服务端模组信息。
- 新增按“backend + 客户端协议号”隔离的 TTL 缓存，`-1` 或 `0` 不复用；新增后端连接、超时或 JSON 解析失败时的可配置 fallback。
- 新增 `status_cache_hits` 与 `status_fallbacks` 指标，并同步到状态 API、周期日志和前端实例状态。
- 前端路由编辑器新增状态来源、缓存 TTL、可空覆盖字段与渐进展开的离线 fallback；后端模式切换时默认清空覆盖字段，以保留后端原值。
- 更新响应式中文 API 文档、README、示例配置与模组兼容矩阵；P0“模组安全状态响应”标记完成。
- 新增后端状态未知字段保留、缓存命中、离线 fallback 与反序列化显式覆盖语义测试。
- 版本升级为 v0.6.0；Ubuntu 24.04 x86_64 上 rustfmt、Clippy 零警告、13 个单元测试、10 个集成测试和 release 构建全部通过。
- 备份 v0.5.0 生产文件后部署到 `64.83.19.35`；systemd、回环健康检查、25565/18080 监听、公网页面和 API 文档正常。
- 临时创建 Hypixel 后端覆盖路由，连续两次真实 Status/Ping 验证 MOTD 覆盖、后端版本/玩家数/favicon 保留与缓存命中；测试路由已删除，生产只保留 `hyp`。

## 关键决策

- 后端模式的所有覆盖字段使用 `Option`；配置中未填写的字段必须反序列化为 `None`，不能继承 custom 模式默认值。
- 缓存保存后端原始 JSON，而不是已覆盖结果，同一后端缓存可在解析后再应用当前路由覆盖，并保留未知字段。
- fallback 只在后端模式生效；成功返回 fallback 时同时记录后端失败与 fallback 指标。
- 继续保持薄代理边界，真实 Fabric/Forge/NeoForge 登录矩阵、多后端负载均衡和 PROXY Protocol 留到后续阶段。
- 前端遵循 `ui-ux-pro-max` 的渐进披露与触控尺寸要求，新增 select 保持 44px 控件高度，并保留键盘焦点样式。

## 风险与待办

- 自动测试已模拟 `forgeData` 和未知字段，但仍需用真实 Fabric、Forge 旧版、Forge 新版和 NeoForge 服务端做容器化版本矩阵。
- 下一阶段实现多后端故障转移、负载均衡、每路由健康指标与可信后端 PROXY Protocol。
- root 密码曾在聊天中明文提供，建议立即轮换并改用 SSH 密钥。

## 关联文件

- `src/config.rs`
- `src/proxy.rs`
- `src/metrics.rs`
- `src/server.rs`
- `src/lib.rs`
- `tests/proxy_integration.rs`
- `web/index.html`
- `web/styles.css`
- `web/app.js`
- `config.example.toml`
- `docs/api.html`
- `MODDED_COMPATIBILITY.md`
- `README.md`
- `BUILD_UBUNTU24.md`
- `dist/mc-proxy-v0.6.0-ubuntu24-x86_64/`
- `dist/mc-proxy-v0.6.0-ubuntu24-x86_64.tar.gz`
- `dist/SHA256SUMS`
- `/opt/mc-proxy/mc-proxy`
- `/var/backups/mc-proxy/mc-proxy.pre-v0.6.0-20260730`
- `/var/backups/mc-proxy/config.pre-v0.6.0-20260730.toml`
- `code.md`

---

## 任务标题

实现原版协议感知基础能力并部署 v0.5.0。

## 完成时间

2026-07-30 01:22（Asia/Shanghai）

## 变更内容

- 将数据面从只解析 Handshake 扩展为解析 Handshake、Status Request/Ping 和 Login Start，后续 Login、Configuration、Play 与模组插件消息继续透明转发。
- 每条域名路由新增可选 `status`：代理可直接返回自定义 MOTD、版本名称、可选协议号、在线人数和最大人数，不连接后端，并正确响应 Ping/Pong。
- 每条域名路由新增 `whitelist_enabled`、`whitelist` 和 `whitelist_message`：代理在连接后端前读取玩家名，非白名单玩家收到原版 Login Disconnect。
- 白名单玩家的 Login Start 包保持原始字节并随握手一起写入后端；新增 Forge/FML NUL Host 扩展和后续插件数据字节保真集成测试。
- 新增 `local_status_responses` 与 `whitelist_denials` 指标，并在状态 API、运行日志和前端实例状态中展示。
- 管理前端路由弹窗新增渐进展开的 MOTD 与白名单配置区，支持 § 颜色、JSON 文本组件、协议自动跟随、玩家名单和自定义拒绝消息。
- 更新响应式中文 HTML API 文档、README、示例 TOML 和生产模板；API 规则结构同步新增协议字段。
- 新增无第三方依赖的 `tests/minecraft_probe.py`，可对任意目标发送真实 Status 或 Login 探针。
- 新增 `MODDED_COMPATIBILITY.md`，明确原版、Fabric、Forge 与 NeoForge 当前边界、风险和 P0/P1/P2 实施顺序。
- 版本升级为 v0.5.0；在 Ubuntu 24.04 x86_64 完成 rustfmt、Clippy 零警告、12 个单元测试、8 个集成测试和 release 构建。
- 备份 v0.4.2 二进制与生产配置后部署到 `64.83.19.35`；systemd、健康检查、25565 监听及公网前端/API 文档均正常。
- 临时创建隔离协议验收路由，真实验证自定义 MOTD/Ping 与白名单 Login Disconnect 后删除；生产 Hypixel 路由再次完成公网状态查询且后端失败为 0。

## 关键决策

- 白名单只做认证前快速筛选，不宣称能够证明玩家身份；生产安全仍依赖后端 `online-mode=true` 和防火墙阻止绕过代理直连。
- 自定义状态响应默认跟随客户端协议号，避免仅因代理填写固定版本而显示错误的不兼容状态。
- 只预读白名单所需的一个 Login Start 包，放行时逐字节转发，不消费 Fabric/Forge/NeoForge 后续协商消息。
- 当前继续保持薄代理架构；Velocity modern forwarding、FML LoginPluginMessage 中继与 Configuration 状态机列入完整会话代理阶段，不能用“TCP 透明”冒充已支持。
- 模组路由若依赖后端 Status 中的 `forgeData`/`modinfo`，v0.5.0 暂不启用代理自定义 MOTD；下一阶段应实现保留未知字段的状态覆盖与缓存。
- 前端依照 `ui-ux-pro-max` 已持久化设计系统，复杂协议配置使用渐进展开，避免基础路由表单默认过载。

## 风险与待办

- 下一优先级是后端 Status 透传覆盖、TTL 缓存与离线 fallback，同时保留 Forge/NeoForge 未知 JSON 字段和 favicon。
- Fabric、Forge、NeoForge 当前只有协议透明性测试，仍需真实服务端版本矩阵验收。
- 完整玩家信息转发需要在线认证、加密会话和 Velocity/Bungee/FML 状态机，不能在薄代理里简单拼接字段。
- root 密码曾在聊天中明文提供，建议轮换并改用 SSH 密钥。

## 关联文件

- `src/config.rs`
- `src/proxy.rs`
- `src/metrics.rs`
- `src/server.rs`
- `src/lib.rs`
- `tests/proxy_integration.rs`
- `tests/minecraft_probe.py`
- `web/index.html`
- `web/styles.css`
- `web/app.js`
- `config.example.toml`
- `deploy/config.production.toml`
- `docs/api.html`
- `MODDED_COMPATIBILITY.md`
- `Cargo.toml`
- `Cargo.lock`
- `README.md`
- `dist/mc-proxy-v0.5.0-ubuntu24-x86_64`
- `dist/mc-proxy-v0.5.0-ubuntu24-x86_64.tar.gz`
- `/opt/mc-proxy/mc-proxy`
- `/var/backups/mc-proxy/mc-proxy.pre-v0.5.0-20260730`
- `/var/backups/mc-proxy/config.pre-v0.5.0-20260730.toml`
- `code.md`

---

## 任务标题

修复 `hyp.mc.lic6.top` 无法代理 Hypixel，并部署 v0.4.2。

## 完成时间

2026-07-30 00:50（Asia/Shanghai）

## 变更内容

- 在生产服务器分别对后端和代理入口发送真实 Minecraft Java Status 握手，确认 DNS、25565 监听、防火墙和后端 TCP 连通性均正常。
- 定位根因：Hypixel 收到客户端原始握手域名 `hyp.mc.lic6.top` 后会立即断开，使用其后端域名 `mc.hypixel.net` 握手则正常响应。
- 为规则新增 `modify_virtual_host` 配置；启用时代理会将 Minecraft Handshake 的 Host 改为 backend 主机名，同时保留 Forge 等扩展使用的 NUL 后缀和其余包字段。
- 管理前端新增“改写后端握手 Host”开关和路由状态展示，静态资源缓存标识更新为 `20260730-hostfix1`。
- 新建路由自动插入到已启用的 `host = "*"` 兜底规则之前，避免兜底规则按配置顺序抢先匹配；同时禁止将 `*` 与其他 Host 写在同一规则。
- 更新示例配置、README 和中文 HTML API 文档，规则请求与响应均包含 `modify_virtual_host`。
- 版本升级为 v0.4.2，在 Ubuntu 24.04 x86_64 完成 rustfmt、Clippy 零警告、10 个单元测试、5 个集成测试和 release 构建。
- 部署到 `64.83.19.35`，生产规则 `hyp` 已设置 `modify_virtual_host = true`，systemd 与健康检查均正常。
- 使用内网 `127.0.0.1:25565` 和公网 `64.83.19.35:25565` 两条路径，以虚拟主机 `hyp.mc.lic6.top` 完成真实状态握手；两次均收到 Hypixel 有效状态响应，后端失败计数为 0。

## 关键决策

- `modify_virtual_host` 默认关闭，保持现有自建后端路由的兼容性；只对明确要求固定握手域名的后端按规则开启。
- Host 改写目标取 backend 地址中的主机部分，不把端口写入握手 Host。
- 保持“配置顺序优先”的匹配语义，通过创建位置保护兜底规则的末位行为，不改变已有具体规则之间的优先级。
- 生产二进制升级前已备份，便于人工快速回滚。

## 风险与待办

- 该修复已验证 Minecraft 状态查询与 TCP 转发成功；实际登录仍受 Hypixel 自身账号、版本、地区及服务策略约束。
- root 密码曾在聊天中明文提供，建议轮换并改用 SSH 密钥。

## 关联文件

- `src/config.rs`
- `src/manager.rs`
- `src/proxy.rs`
- `tests/proxy_integration.rs`
- `web/index.html`
- `web/styles.css`
- `web/app.js`
- `config.example.toml`
- `deploy/config.production.toml`
- `docs/api.html`
- `Cargo.toml`
- `Cargo.lock`
- `README.md`
- `dist/mc-proxy-v0.4.2-ubuntu24-x86_64`
- `dist/mc-proxy-v0.4.2-ubuntu24-x86_64.tar.gz`
- `/opt/mc-proxy/mc-proxy`
- `/var/backups/mc-proxy/mc-proxy.pre-v0.4.2-20260730`
- `code.md`

---

## 任务标题

使用 ui-ux-pro-max 重设计管理前端并部署 v0.4.1。

## 完成时间

2026-07-30 00:32（Asia/Shanghai）

## 变更内容

- 使用 `ui-ux-pro-max` 为 Minecraft 代理运维场景生成并持久化设计系统，采用实时运维仪表盘、暗色高对比、高信息密度和低干扰动效方向。
- 全面重构登录页、侧边导航、顶部状态区、指标卡、实时流量图、实例状态、域名路由卡片、配置表单和规则编辑弹窗。
- 新增侧边栏 Minecraft 入口地址与运行状态，明确区分管理端在线和游戏入口启停。
- 新增实时流量图暂停/继续控制；暂停只冻结图表采样，实时数值仍持续更新。
- 增加跳至主内容链接、键盘焦点环、导航 `aria-selected`、移动菜单展开状态、弹窗焦点回归和图表按钮 `aria-pressed`。
- 表单异步提交时禁用按钮并显示“正在验证/保存/应用”，避免重复提交。
- 全部交互目标至少 44px，移动端输入字号提升至 16px，支持 390/620/860/1140px 响应式布局和 `prefers-reduced-motion`。
- 保持无 Emoji 图标、内联 SVG、无第三方字体/CDN、无外部运行时依赖。
- 静态资源增加版本查询参数，避免浏览器继续使用旧版 CSS/JavaScript 缓存。
- 版本升级为 v0.4.1，同步 API 文档版本标识和 Ubuntu 24 构建产物。
- 在 Ubuntu 24.04 x86_64 使用 Rust 1.97.1 完成 rustfmt、Clippy 零警告、9 个单元测试、5 个集成测试和 release 构建。
- 本地完成 JavaScript 语法、HTML 结构、元素 ID、外部依赖、Emoji 和可访问性规则检查。
- 部署至 `64.83.19.35`，备份 v0.4.0 二进制后重启服务；systemd 状态、健康检查和公网新资源均验证通过。

## 关键决策

- 遵循设计系统的绿色状态色、深色运维面板与等宽技术标题，但不加载其推荐的 Google Fonts，改用系统字体栈以维持零外部依赖。
- 流量上传和下载除颜色外还使用实线/虚线区分，降低仅依赖颜色造成的可访问性问题。
- 动效只用于页面切换、抽屉、反馈和状态变化，并提供 reduced-motion 降级。
- 本次未修改任何后端接口或生产业务配置，部署时保留现有域名路由和 `proxy_enabled` 状态。

## 风险与待办

- 当前环境没有可用的 Chromium 自动截图工具，本次通过静态结构检查、响应式 CSS 审核和公网资源核验完成前端验收；后续可补充 Playwright 视觉回归。
- 实时 SVG 图表提供文本 KPI 和暂停功能，但尚未提供逐时间点键盘提示或数据表导出。
- root 密码曾在聊天中明文提供，仍建议轮换并改为 SSH 密钥。

## 关联文件

- `design-system/mc-relay-control/MASTER.md`
- `web/index.html`
- `web/styles.css`
- `web/app.js`
- `Cargo.toml`
- `Cargo.lock`
- `docs/api.html`
- `README.md`
- `dist/mc-proxy-v0.4.1-ubuntu24-x86_64`
- `dist/mc-proxy-v0.4.1-ubuntu24-x86_64.tar.gz`
- `/opt/mc-proxy/mc-proxy`
- `/var/backups/mc-proxy/mc-proxy.pre-v0.4.1-20260730`
- `code.md`

---

## 任务标题

将 v0.4.0 单端口域名路由版本部署到生产服务器。

## 完成时间

2026-07-30 00:16（Asia/Shanghai）

## 变更内容

- 核对生产环境，确认此前仍运行旧版二进制、旧版配置和旧版前端资源。
- 备份生产二进制到 `/var/backups/mc-proxy/mc-proxy.pre-v0.4.0-20260730`。
- 备份生产配置到 `/var/backups/mc-proxy/config.pre-v0.4.0-20260730.toml`。
- 安装服务器原生编译的 v0.4.0 ELF 到 `/opt/mc-proxy/mc-proxy`。
- 将旧版“每规则监听地址”配置迁移为全局单入口加 `host → backend` 路由配置。
- 重启 `mc-proxy.service`，确认 systemd 状态为 `active`、管理健康检查返回 `ok`。
- 通过公网 HTTPS 验证新前端已包含 host 路由表单和握手超时设置，API 文档显示 v0.4.0。
- 核对生产二进制 SHA-256 与本地交付产物一致。

## 关键决策

- 保持 `proxy_enabled = false`，因为当前后端仍是未确认的占位地址 `127.0.0.1:25566`；管理页面已经上线，但不会误开放 25565 游戏入口。
- 本次只迁移原本停用的占位规则，没有擅自填写真实业务域名或后端地址。
- 保留升级前二进制与配置，出现兼容问题时可人工快速回滚。

## 风险与待办

- 用户需在管理页面填写真实的 host 和 backend，确认无误后再到全局配置启用 Minecraft 入口。
- 浏览器可能缓存旧版静态资源；公网源站已确认返回 v0.4.0，必要时执行强制刷新。
- root 密码曾通过聊天明文提供，建议立即轮换并改用 SSH 密钥登录。

## 关联文件

- `/opt/mc-proxy/mc-proxy`
- `/etc/mc-proxy/config.toml`
- `/var/backups/mc-proxy/mc-proxy.pre-v0.4.0-20260730`
- `/var/backups/mc-proxy/config.pre-v0.4.0-20260730.toml`
- `deploy/config.production.toml`
- `dist/mc-proxy-v0.4.0-ubuntu24-x86_64`
- `code.md`

---

## 任务标题

按 Gate Lite 模型改为同一 IP、同一端口基于 Minecraft 握手域名分流。

## 完成时间

2026-07-30 00:12（Asia/Shanghai）

## 变更内容

- 将“每条规则各自监听一个端口”的模型改为单一全局 Minecraft 入口；监听地址、启用状态和最大并发统一放入 `settings`。
- 每条转发规则只描述 `host → backend`，同一入口按配置顺序选择首个匹配规则。
- `host` 在 TOML 中同时接受单个字符串和字符串数组，匹配不区分大小写，支持 `*`、`?` 通配以及最后一条 `host = "*"` 兜底。
- 后端地址由仅接受 IP 的 `SocketAddr` 改为 `主机:端口` 字符串，支持 DNS 后端。
- 代理对每个连接读取 Minecraft Java 首个 Handshake，选择后端后将握手原始字节完整转发。
- 运行时管理器改为只维护一个监听器；增删改、启停 host 路由时校验、原子持久化并重载整张路由表，失败时回滚旧入口。
- 管理控制台移除每条规则里的监听地址、默认后端和独立并发字段，新增全局监听入口、全局并发和握手超时配置。
- 同步更新响应式中文 API 文档、README、示例配置和生产配置模板。
- 版本升级到 v0.4.0。
- 在 Ubuntu 24.04 x86_64 服务器使用 Rust 1.97.1 完成 rustfmt、Clippy 零警告、9 个单元测试、5 个集成测试和 release 构建。
- 本地 Node.js v24 完成 `web/app.js` 语法检查；服务器未安装 Node，因此该项使用本地结果。
- 回传 Ubuntu 24 x86_64 ELF 与保留 0755 权限的 tar.gz。

## 关键决策

- 监听入口只配置一次，避免用户为不同域名重复填写相同 IP:端口，这与 Gate Lite 的 `bind + routes` 语义一致。
- 路由按配置出现顺序首个命中，不额外人为提升精确匹配优先级；需要兜底时明确把 `*` 放在最后。
- 未匹配任何规则时关闭连接并计入转发异常，不再使用隐式默认后端。
- 本次实现聚焦域名选路核心，不宣称支持 Gate Lite 的 Proxy Protocol、TCPShield RealIP、负载均衡、MOTD 缓存或 `$1` 后端参数替换。
- 未覆盖或重启服务器现网服务，只在 `/tmp/mc-proxy-v04.LJR6vQ` 隔离编译测试。

## 风险与待办

- 配置结构与 v0.3.0 不兼容，升级前需按 `config.example.toml` 把旧规则迁移成全局入口加 host 路由。
- 路由变更当前会重载唯一监听器，并按宽限期等待现有连接结束；后续可改为无停机原子替换内存路由表。
- Android 共享存储中的原始 ELF 显示为 0660；传回 Ubuntu 时优先解压 tar.gz，归档内 ELF 为 0755。
- 服务器未安装 Node.js，不影响 Rust 服务运行；前端脚本已在本地通过 Node 语法检查。

## 关联文件

- `Cargo.toml`
- `Cargo.lock`
- `src/config.rs`
- `src/proxy.rs`
- `src/manager.rs`
- `src/server.rs`
- `src/api.rs`
- `src/lib.rs`
- `tests/proxy_integration.rs`
- `web/index.html`
- `web/app.js`
- `docs/api.html`
- `config.example.toml`
- `deploy/config.production.toml`
- `README.md`
- `dist/mc-proxy-v0.4.0-ubuntu24-x86_64`
- `dist/mc-proxy-v0.4.0-ubuntu24-x86_64.tar.gz`
- `code.md`

---

## 任务标题

新增无 Emoji 的 SVG Web 前端、Admin 管理控制台、多规则配置和实时指标，并部署到 mc.lic6.top。

## 完成时间

2026-07-29 11:57（Asia/Shanghai）

## 变更内容

- 将单规则配置升级为 `admin + settings + rules[]` 多规则模型，支持规则 ID、名称、监听地址、后端地址、启用状态和独立最大并发。
- 新增运行时管理器，支持在线新增、修改、启停和删除规则；变更失败时回滚旧规则，成功后通过临时文件和 rename 原子持久化 TOML。
- 将双向复制改为自定义异步数据泵，在数据成功写入另一端后实时累加上下行字节，长连接无需断开即可在面板显示流量。
- 新增 Axum 管理 HTTP 服务，管理端强制只监听回环地址，并要求至少 32 字符的 Bearer Token。
- 新增状态、配置和规则管理 API，统一 JSON 成功/错误结构。
- 新增响应式 Web 控制台，包含运行概览、实时 SVG 吞吐图、线路状态、规则管理和全局配置页面。
- 所有界面图标使用内联 SVG，未使用 Emoji，也未加载第三方 CDN 资源。
- 新增中文响应式、可检索 HTML API 文档并内嵌到 `/docs/api`。
- 增加部署模板：systemd 沙箱服务、Nginx 反向代理、API 限速和生产配置。
- 在 Ubuntu 24 服务器完成 8 个单元测试、4 个集成测试、Clippy 零警告和 release 构建。
- 通过管理 API 动态创建临时 TCP 规则，完成 77824 字节双向数据与实时指标联合测试，再在线停用和删除。
- 安装并配置 Nginx/Certbot，`mc.lic6.top` 已启用 HTTPS、HTTP 跳转、安全响应头和证书自动续期。
- 以低权限 `mc-proxy` 用户部署 systemd 服务；管理端仅监听 `127.0.0.1:18080`，systemd security exposure 为 2.8（OK）。
- 回传 v0.2.0 Ubuntu 24 x86_64 ELF 和保留 0755 权限的 tar.gz。

## 关键决策

- 管理页面只公开静态 UI；所有状态和变更 API 均需要 Bearer Token，令牌不进入配置响应。
- 浏览器只在 `sessionStorage` 保存令牌，关闭标签后失效，减少长期泄露风险。
- 生产默认规则保持停用，避免尚未获得真实 Minecraft 后端地址时监听 25565。
- 管理监听地址强制为 loopback，公网只允许经过 Nginx HTTPS、安全头和请求限速。
- 规则更新采用“停止受影响规则、尝试新规则、失败回滚、成功持久化”的事务式流程；未变化的规则不会重启。
- 实时字节数使用 Relaxed 原子增量，避免逐包锁竞争；前端根据两次快照差值计算吞吐率。

## 风险与待办

- 用户需要登录管理页面编辑真实后端地址并启用 `main` 规则，目前 25565 未监听。
- 管理认证是单一高强度令牌，不包含多用户、角色权限或操作审计；如需多人管理应增加账户系统和审计日志。
- 配置变更会优雅停止受影响规则；有长连接时 API 最长可能等待 `shutdown_grace_secs`。
- 当前统计驻留内存，服务重启后清零；如需长期历史曲线需接入 Prometheus 或时序数据库。
- 当前仍是纯 Java TCP L4 转发，不解析 Minecraft Handshake，不支持同端口按域名分流和玩家真实 IP。
- 服务器存在待重启的新内核，本任务没有执行服务器重启。

## 关联文件

- `Cargo.toml`
- `Cargo.lock`
- `config.example.toml`
- `src/api.rs`
- `src/config.rs`
- `src/lib.rs`
- `src/listener.rs`
- `src/main.rs`
- `src/manager.rs`
- `src/metrics.rs`
- `src/proxy.rs`
- `src/server.rs`
- `src/web.rs`
- `web/index.html`
- `web/styles.css`
- `web/app.js`
- `docs/api.html`
- `deploy/config.production.toml`
- `deploy/mc-proxy.service`
- `deploy/nginx-mc.lic6.top.conf`
- `deploy/nginx-rate-limit.conf`
- `tests/proxy_integration.rs`
- `README.md`
- `BUILD_UBUNTU24.md`
- `dist/mc-proxy-ubuntu24-x86_64`
- `dist/mc-proxy-ubuntu24-x86_64.tar.gz`
- `code.md`

---

## 任务标题

实现 Minecraft Java 高性能 Rust TCP 转发器，并在 Ubuntu 24 服务器编译测试。

## 完成时间

2026-07-29 11:19（Asia/Shanghai）

## 变更内容

- 创建独立 Rust 2024 工程，使用 Tokio 多线程运行时和 socket2 实现固定后端的 Minecraft Java 四层 TCP 转发。
- 支持 TOML 配置、后端连接超时、TCP_NODELAY、socket 缓冲区、backlog，以及 Linux/Android 可选 SO_REUSEPORT。
- 使用 Tokio 双向复制完成两个方向并发透传，默认每方向 32 KiB 用户态缓冲区。
- 使用 Semaphore 限制最大并发连接数，达到上限时拒绝新连接并记录统计。
- 使用 JoinSet 跟踪连接任务，支持 Ctrl+C/SIGTERM 停止监听、宽限等待和超时取消。
- 增加原子连接/失败/字节统计和 tracing 周期聚合日志，正常单连接只在 debug 级别记录。
- 增加 5 个单元测试和 3 个集成测试，覆盖配置校验、连接计数、大包转发、半关闭、后端失败、连接上限和退出。
- 在 Ubuntu 24.04.4 x86_64 服务器安装 Rust 1.97.1 和标准编译工具，完成 rustfmt、Clippy、测试和 release 构建。
- 使用独立代理进程和 TCP echo 后端完成 164096 字节双向端到端测试，三方退出码均为 0，测试后无残留监听。
- 将服务器原生编译的 ELF 和保留 0755 权限的 tar.gz 归档回传至 `dist/`。

## 关键决策

- 首版保持纯 L4 固定后端转发，不解析 Minecraft 握手包，以降低延迟、CPU 和协议状态复杂度。
- SO_REUSEPORT 默认关闭，仅在 Linux 多实例共享端口场景由配置显式开启。
- 热路径不逐包记录日志；原子统计在连接结束时合并双向字节数，降低共享状态开销。
- 使用集成测试验证 TCP 半关闭语义，确保客户端停止发送后仍能接收后端响应。
- Ubuntu 产物同时交付原始 ELF 和 tar.gz；Android 共享存储可能丢失执行位，归档更适合传回 Linux。

## 风险与待办

- 当前仅支持 Minecraft Java TCP 固定后端，不支持基岩版 UDP/RakNet、域名分流或协议级真实 IP。
- 连接持续期间的字节数只在连接结束后计入聚合统计；若需要实时流量曲线，可改为自定义双向复制循环分批累加。
- 真实生产部署前需要修改 `config.toml` 后端地址、设置低权限运行用户和文件描述符上限。
- 是否引入 splice、thread-per-core 或 io_uring 应以生产压测和 CPU profile 为依据。
- 服务器提示存在待重启的新内核，但此次构建和测试使用 Linux 6.8.0-48-generic 并已全部通过；本任务未执行服务器重启。

## 关联文件

- `Cargo.toml`
- `Cargo.lock`
- `config.example.toml`
- `src/lib.rs`
- `src/main.rs`
- `src/config.rs`
- `src/listener.rs`
- `src/proxy.rs`
- `src/server.rs`
- `src/metrics.rs`
- `tests/proxy_integration.rs`
- `MC_PROXY_PLAN.md`
- `README.md`
- `BUILD_UBUNTU24.md`
- `dist/mc-proxy-ubuntu24-x86_64`
- `dist/mc-proxy-ubuntu24-x86_64.tar.gz`
- `code.md`
