# Fabric / Forge / NeoForge 真实矩阵运行手册

## 安全边界

- 脚本不会修改 `eula.txt`。用户必须先阅读 <https://aka.ms/MinecraftEULA> 并明确同意，再人工把对应实例设为 `eula=true`。
- 每次只启动一个 JVM，最大堆 512 MiB。
- 服务端、隔离代理和管理端都只绑定 `127.0.0.1`，不开放公网端口。
- 测试使用 `online-mode=false`，仅用于回环协议探针；生产后端仍应使用在线认证。
- 脚本不会修改或重启生产 `mc-proxy.service`，只复用已部署二进制启动临时进程。

## 已安装实例

| 加载器 | Minecraft | 版本 | 目录 | 回环端口 |
|---|---:|---:|---|---:|
| Fabric | 1.21.1 | Loader 0.19.3 | `/opt/mc-proxy-matrix/fabric-1.21.1` | 26701 |
| Forge | 1.21.1 | 52.1.16 | `/opt/mc-proxy-matrix/forge-1.21.1` | 26702 |
| NeoForge | 1.21.1 | 21.1.244 | `/opt/mc-proxy-matrix/neoforge-1.21.1` | 26703 |

## 验收内容

对每套真实服务端依次执行：

1. 重试真实 Minecraft Status 请求，直到加载器完成协议初始化；仅有 TCP 端口监听不算就绪。
2. 启动一份隔离 mc-proxy，配置透明路由和 `status.mode=backend` 路由。
3. 比较直连、透明代理、后端托管三份 Status JSON，要求完整相等，包括 `forgeData`、`modinfo` 等未知扩展。
4. 测试实例关闭网络压缩，使用 Minecraft 1.21.1 协议发送 Login Start；Forge/NeoForge Host 附带 `\0FORGE`。要求直连和代理的 Login Success 一致，随后发送 Login Acknowledged 进入 Configuration，并要求第一份 Configuration 包的 ID、长度与 SHA-256 一致。
5. 等待 `minecraft-status` 主动探测变为 `healthy`。
6. 优雅停止真实后端，并验证代理自动切换为 `unhealthy`。
7. 保存服务端日志、代理日志和 JSON 报告。

## 命令

```sh
python3 tests/run_modded_matrix.py --loader all
```

单独重跑：

```sh
python3 tests/run_modded_matrix.py --loader fabric
python3 tests/run_modded_matrix.py --loader forge
python3 tests/run_modded_matrix.py --loader neoforge
```

默认报告目录：

```text
/opt/mc-proxy-matrix/results/
```

通过必须以 `summary.json` 中 `"passed": true` 为准；只有安装成功、端口可连接或模拟夹具通过不能算真实矩阵通过。

## 2026-07-30 实测结果

- Fabric Loader 0.19.3：通过。
- Forge 52.1.16：通过，Status 中的 `forgeData` 完整一致。
- NeoForge 21.1.244：通过，Status 中的 `isModded` 完整一致。
- 三套连续运行的合并报告：`tests/modded-matrix-summary.json`。
- 合并报告 SHA-256：`8b20399dd6319a2550f61d7c6ed11326ae52cf67c234274426e2f67541c50dbf`。
- 测试结束后 26701–26703、26610、28110 均无残留监听，生产 `mc-proxy.service` 保持 `active`。
