# Niuma Context

## 桌面侧重构边界

Niuma 桌面侧后续从 Python `niuma-plugin` 逐步收敛到 Rust `niuma-cli`。现有
`niuma-plugin` 作为业务逻辑参考来源保留，但 Rust 实现不需要照搬 Python 的模块
划分。

`niuma-cli` 稳定后删除 `niuma-plugin`。在过渡期内，`niuma-plugin` 仅作为参考实现
和可对照的业务逻辑来源，不作为新架构中的并行桌面 runtime。

## 服务端重构边界

`niuma-server/` 将由 Rust 实现替换现有 FastAPI 实现。目录边界保持不变：
`niuma-server/` 仍是独立服务端工程，不并入 `niuma-cli/`，也不把 repo 根目录改成
Cargo workspace。

Rust 版 `niuma-server` 继续承担 Niuma Server 的控制面职责。`niuma-cli` 仍是桌面
Gateway/runtime，二者不能合并为一个长期运行进程。

Rust 版 `niuma-server` 面向服务器部署，继续只支持 PostgreSQL 作为控制面持久化后端。
不引入其他控制面存储后端或测试专用存储分支。

Rust 版 `niuma-server` 第一阶段保持现有 iOS 与 `niuma-cli` 使用的 HTTP / WebSocket
外部协议不变。重构目标是替换服务端内部实现并删除无用逻辑，不在同一阶段改动移动端
或 Gateway 的协议字段。

Rust 版 `niuma-server` 第一阶段不迁移 Python 版 `AuditService` 或 `audit_events`
表。服务端只保留运行日志；审计、指标和限流未来如需上线，应作为独立 observability
设计重新引入。

Rust 版 `niuma-server` 的 transfer payload 继续放在服务端本地临时目录，不写入
PostgreSQL。PostgreSQL 只保存控制面长期状态；文件名、MIME、`file_ref`、`file_type`
等业务字段仍由 iOS 和 Gateway 的业务 payload 处理。

Rust 版 `niuma-server` 沿用 `niuma-server/.env` 和 `NIUMA_` 配置前缀。本地 `.env`
存在时优先于 shell 环境变量；部署环境没有 `.env` 时读取进程环境变量。数据库 URL
使用 Rust/PostgreSQL 标准格式，例如 `postgresql://` 或 `postgres://`。

Rust 版 `niuma-server` 当前启动时自动建表，不引入正式 migration 工具。

替换完成后，`niuma-server/` 不保留 Python 实现、Python 测试或 Python 项目配置。
该目录只保留 Rust server crate 和配套文档/配置示例。

Rust 版 `niuma-server` 的服务端技术栈收敛为 `axum`、`tokio`、`sqlx`、`serde` 和
`tracing`。不引入 ORM、复杂 DI/container 或第二套 Rust Web 框架。

Rust 版 `niuma-server` 的 WebSocket 继续是 payload-blind 路由。Server 只负责连接
鉴权、配对关系校验、移动端 `task_start` 签名校验和点对点/广播转发；不解析
`ciphertext`，不保存 project/session/thread/approval 明文索引，也不引入消息历史缓存。

Rust 版 `niuma-server` 的 WebSocket 连接模型是单设备单活连接。同一个
`ios_device_id` 或 `agent_id` 新连接鉴权通过后替换旧连接，并主动关闭旧连接；服务端
不做同一设备多连接 fanout，也不做离线消息队列。

Rust 版 `niuma-server` 严格保持配对 token 在 desktop signed ack 成功后才 consumed。
`/pair/confirm` 验证 iOS 请求后先把 encrypted handshake 转发给在线 Gateway；只有
Gateway 解密并返回 signed ack，server 验证通过后才创建/激活 binding 并消耗 token。
ack 失败或超时时不创建 active binding。

Rust 版 `niuma-server` 的 challenge 和 session token 都持久化到 PostgreSQL。未过期
session token 在 server 重启后继续有效；token 过期后客户端重新走
`/auth/challenge` 和 `/auth/verify`。

Rust 版 `niuma-server` 完全沿用当前 Ed25519 digest/signature 口径，不重新设计签名
字段、拼接顺序或 public key 格式。Rust 替换阶段不引入 JWS/JWT 等新 envelope。

Rust 版 `niuma-server` 不持久化 `online_status`。在线状态只来自当前 WebSocket
connection hub；PostgreSQL 设备表只保留 `last_seen_at` 等长期可解释字段。

Rust 版 `niuma-server` 的 pair binding 只保留 `active` 和 `revoked` 两种长期状态。
`pending`、`failed` 或 ack 超时属于 pair token 尝试过程，不落到 binding 表。

Rust 版 `niuma-server` 的 pair token 必须保存本次 `agent_pairing_public_key`，并在
`/pair/confirm` 校验 iOS 提交的 `agent_pairing_public_key` 与 token 记录一致。该字段
只绑定一次性二维码公钥，不作为长期 encryption public key 目录。

Rust 版 `niuma-server` 替换完成必须通过三层验证：Rust 单测、HTTP/WebSocket 接口级
集成测试，以及真实 iOS 模拟器联调。联调至少覆盖 Rust server 启动、`niuma gateway`
连接、模拟扫码配对、session 列表刷新、文本消息发送和一次文件/图片 transfer。

Rust 版 `niuma-server` 开始实现 APNs / Push Service。APNs 只用于离线、锁屏或后台
场景下提醒移动端“任务有新的进度”，不承载明文 session、thread、prompt、审批详情或
执行结果。通知 payload 可以携带 Gateway 生成的端到端加密密文；移动端点击通知后解密
密文中的 `thread_id`，再通过现有 `resume_thread` 详情刷新链路拉取新数据。

Rust 版 `niuma-server` 第一阶段不实现完整限流或风控系统，只保留协议内必要约束：
challenge/session/pair token TTL、pair token failed attempts 上限，以及 transfer TTL
和最大 payload 大小。

## 术语

### Niuma Server

`niuma-server` 是移动端和桌面 Gateway 之间的 payload-blind 控制面服务，负责设备、
鉴权、配对、WebSocket 路由和临时 transfer 中转，不保存 Codex 会话业务历史。

### Niuma Gateway

`niuma gateway` 是前台运行的桌面网关命令，负责启动配对服务、连接 Niuma Server、
连接 Codex runtime，并把移动端任务转发给 Codex。它替代当前插件 dashboard
承担的长期配对入口能力。

`gateway` 是完整桌面 runtime，不只是二维码页面。它同时负责本地 HTTP 配对页、
`/api/pairing/payload`、桌面 agent identity、pair token 请求、`/ws/agent`
长连接、Codex app-server 子进程、文件 transfer 本地物化，以及 `task_start`、
`resume_thread`、审批等移动端消息处理。

### 移动端配对入口

移动端配对入口只负责连接或添加桌面 Gateway。未配对时，移动端不把“未配对”渲染成
独立状态对象；已有绑定时，入口语义是“添加桌面设备”，不是覆盖当前绑定。

### Niuma Service

`niuma service` 是后台服务管理命令，负责把 gateway 注册为系统后台进程，并提供
安装、启动、停止、卸载等生命周期操作。macOS 上优先按 launchd 语义设计。

当前阶段只支持 macOS `launchd`，管理
`~/Library/LaunchAgents/com.niuma.gateway.plist`。暂不抽象 Linux systemd 或 Windows
Service。

`niuma service install` 写入 launchd plist 时，必须解析当前 `niuma` 二进制的绝对
路径并写入 `ProgramArguments`，不依赖 launchd 环境中的 `PATH`。后台日志写入：

```text
~/.niuma/logs/gateway.out.log
~/.niuma/logs/gateway.err.log
```

`niuma service install` 默认只写入 plist，不隐式启动服务。启动必须通过
`niuma service start`，或显式使用 `niuma service install --start`。

`niuma service start` 如果发现本地 gateway 端口已被前台进程或其他进程占用，应失败
并提示占用地址和 PID，不自动 kill。`niuma service restart` 用于重启 launchd 管理
的后台 gateway。

前台 `niuma gateway` 和后台 `niuma service` 启动 gateway 时，都默认打开本地
loopback 配对页面。是否打开浏览器必须能通过参数控制；默认行为是打开，显式
`--no-open` 才禁止打开。service 模式下该行为写入 launchd plist 的
`ProgramArguments`，使未配对用户通过 `niuma service start` 也能直接进入配对。

`niuma service install --no-open` 是持久安装参数，会把 gateway 的 `--no-open` 写入
plist。`niuma service start` 只按已安装 plist 启动，不临时改写 open/no-open 行为。

`niuma service status` 需要同时读取 launchd 状态和 gateway `GET /api/status`。输出
至少包括 launchd loaded/running/pid、gateway HTTP 是否可达、server 是否可达、agent
鉴权/配对状态、app-server runtime 来源，以及本地页面 URL。

`niuma status` 和 `niuma service status` 都保留。`niuma status` 只读取当前 gateway
HTTP `/api/status`，适合前台或后台 gateway 的通用诊断；`niuma service status` 额外
读取 launchd 状态并合并 gateway HTTP 状态。

### Installation

当前阶段 `niuma-cli` 只承诺本地安装：

`cargo install --path niuma-cli`

暂不引入 crates.io 发布、Homebrew tap 或额外安装器。

`niuma-cli/` 是当前 repo 根目录下的独立 Rust crate。不要把整个 Niuma 容器仓库改成
Cargo workspace；根目录仍然只是 iOS、server、desktop、design/docs 的容器。

crate 目录名是 `niuma-cli/`，安装后的二进制命令名是 `niuma`。用户入口是
`niuma gateway`、`niuma service ...`、`niuma status` 和 `niuma reset --yes`。

### Local State

`niuma-cli` 的用户级状态目录固定为 `~/.niuma`。前台 `niuma gateway` 和后台
`niuma service` 管理的 launchd 进程都使用这个目录，不依赖源码 checkout 或旧插件
安装目录。

`niuma-cli` 不迁移旧 `niuma-plugin` 状态，不读取旧 `.niuma-state`，也不读取旧
`~/Library/Application Support/Niuma/agent/identity.json`。Rust CLI 使用新的
`~/.niuma` 状态；旧插件稳定后整体删除。

`niuma reset --yes` 用于测试阶段清理本机状态。它应先停止并卸载 launchd service，
再删除 `~/.niuma`，从而清除 identity、配对密钥、transfer 缓存、日志和后台安装状态。
没有 `--yes` 时不得执行删除。

建议目录结构：

```text
~/.niuma/
  identity/
  transfers/
  logs/
  runtime/
```

### Configuration

`niuma gateway` 的配置来源优先级为：

```text
命令行参数 > 环境变量 > ~/.niuma/config.toml
```

后台 service 运行时应主要依赖 `~/.niuma/config.toml`，前台开发和诊断时可通过命令行
参数临时覆盖。基础配置至少包括 `server_url`、`device_name`、`dashboard_host` 和
`dashboard_port`。

### Server Availability

`niuma gateway` 默认是完整 runtime。如果启动时无法连接 `niuma-server`，应直接失败
退出并打印清晰错误，不进入半可用状态。

诊断场景可以显式使用：

`niuma gateway --pairing-page-only`

该模式只启动本地配对页面用于排查，不建立真实 Server WebSocket 或 Codex runtime。

### Codex App Runtime

在本项目语境里，"优先连接 Codex App" 指优先使用 Codex.app 应用包内置的
`codex` 二进制创建 app-server，例如：

`/Applications/Codex.app/Contents/Resources/codex app-server`

这不等同于连接 Codex App 当前 Electron 主进程或当前桌面 UI 会话。当前已确认的
Codex App 主进程 app-server 默认通过 stdio 与 App 私有通信，外部进程不能可靠
复用它来刷新当前桌面 UI 会话。

### Codex CLI Runtime

如果 Codex.app 不存在，`niuma gateway` 才尝试使用 PATH 上的 `codex` CLI 创建
app-server。该路径是降级运行方式，不能假定拥有 Codex App 的桌面 UI 能力。

### 回合变更摘要

回合变更摘要表示一个用户消息触发的 Codex turn 在最终结论处给出的文件变化统计和
详情。它只统计最终结论关联的 `fileChange` diff，不统计同一 turn 内的中间过程
`fileChange`。
回合变更摘要附着在 Codex app-server 原生 `fileChange` 条目上，由移动端按同一 turn
聚合展示，不作为独立的合成会话消息。
回合 diff 详情通过临时 transfer 传递；实时消息只承载轻量摘要和 diff bundle 引用，
不得把完整 diff 扩张进 WebSocket 控制帧。
Gateway 负责从 app-server `fileChange` 的 unified diff 预解析出文件级增删统计、
hunk 和 line 结构；diff bundle 必须同时保留 app-server 原始 `raw_diff`，供移动端在
结构化解析不可用时退回纯文本展示。
Gateway 以同一 turn 内 `type=agentMessage` 且 `phase=final_answer` 的条目作为结论
边界；回合变更摘要只聚合该边界之前最后一组连续的 `status=completed` `fileChange`。
没有 `final_answer` 的 turn 只能按过程变化展示，不生成回合变更摘要。若 app-server
未来提供 `fileChange` 与最终结论的显式关联字段，Gateway 优先使用显式关联。
实时协议不新增 `task_update` 顶层字段；Gateway 通过 `ContentPartsPayload.content_parts[]`
新增 `type=file_change_summary` 结构化 part，承载 `files`、`additions`、`deletions`、
文件级摘要和 diff bundle 的 `transfer_id`。完整 diff bundle 仍通过 transfer 读取。
`file_change_summary` 的最小字段为 `files`、`additions`、`deletions`、`transfer_id`
和 `files_summary[]`；`files_summary[]` 每项包含 `path`、`change_type`、`additions`
和 `deletions`。新增/删除行数从最终结论关联的 `changes[].diff` 计算：以 `+` 开头但
不是 `+++` 的行为新增，以 `-` 开头但不是 `---` 的行为删除；`files` 是去重后的文件数。
移动端数据归属仍保留在原生 `fileChange` entry 上，但时间线视觉上把回合变更摘要挂到
同一 turn 的最终回答下方。默认渲染紧凑卡片，显示文件数、总新增/删除和文件级摘要；
点击卡片进入全屏 sheet，按文件展示 hunk/line 详情，结构化详情不可用时展示 `raw_diff`。
diff bundle 是版本化 JSON，`version=1`，`source=codex_app_server_file_change`，
包含 `turn_id`、`final_answer_entry_id`、`summary` 和 `files[]`。`files[]` 每项保留
`path`、`old_path`、`change_type`、`additions`、`deletions`、`raw_diff` 和解析后的
`hunks[]`；`hunks[].lines[]` 使用 `context`、`delete`、`add` 标识行类型，并携带
`old_line`、`new_line` 和 `content`。解析失败时 `hunks=[]`，移动端回退展示同文件的
`raw_diff`。diff bundle 不承载分支统计字段。
既有历史不做全局后台重放，也不对当前 thread 自动发起 `cursor=0` 的全量刷新。移动端
在线程列表为单个 thread 提供右侧小弹窗操作入口，操作列表当前只包含“重置历史”；
用户触发后清空 iOS 端该 thread 相关的 SwiftData 历史和同步状态，重新进入详情时继续走
现有详情刷新逻辑。“重置历史”保留 thread 列表行和 thread metadata，只清空该 thread
对应的 `StoredThreadEntry`、cursor/checkpoint、内存 timeline/refresh state、transient
entry，以及只被这些 entry 引用的本地 attachment 文件。删除 thread 时复用同一段本地历史
清理逻辑，再删除对应 `StoredThread`。
thread 行操作按钮点开菜单后选择“重置历史”必须二次确认；确认文案说明该操作不影响桌面
Codex 历史或服务端记录，只清空本机缓存，重新进入详情会重新同步。确认完成后停留在线程
列表，不自动跳转详情。

### 分支变更摘要

分支变更摘要表示当前 Git branch 或 worktree 相对指定基准的最终文件变化统计和详情。
它可能包含本回合之前已经存在的未提交改动，不等同于回合变更摘要。
分支变更摘要以 thread 为入口和缓存单位，独立于会话时间线，由 Gateway 在移动端打开
某个 thread 详情页顶部“分支变更”入口时按 Git
当前状态即时计算。默认口径是当前 `HEAD` 到 worktree 的 staged + unstaged 变化；
PR 或跨分支对比必须显式传入 base ref，不能复用回合变更摘要的 `file_change_summary`。
分支变更摘要通过独立 WebSocket 请求/响应获取：移动端发送 `branch_changes_request`，
Gateway 回复 `branch_changes_result` 或 `branch_changes_failed`。请求携带 `request_id`、
`thread_id` 和可选 `base_ref`；Gateway 通过 Codex app-server 读取该 thread payload，使用其中的 `cwd` 定位 Git workspace。结果摘要 payload 使用 iOS/Gateway 端到端加密，完整 diff
bundle 仍通过 transfer 读取，niuma-server 不得看到代码 diff 或文件路径。

### 任务进度推送

任务进度推送表示 Gateway 确认一个 Codex turn 已到达终态后，通过 APNs 发送给移动端
的后台提醒。它只说明“有任务有新的进度”，不等同于 `task_update` 过程消息，也不替代
`resume_thread` 详情同步。

### Codex Plugin Policy

`niuma gateway` 启动 Codex app-server 时默认不传 `--disable plugins`，不禁止用户
现有 Codex 插件能力。

诊断场景可以显式使用：

`niuma gateway --disable-codex-plugins`

只有该参数存在时，才向 app-server 命令追加 `--disable plugins`。

### MCP Migration

`niuma-cli` 第一阶段不迁移旧 `niuma-plugin` 的 MCP 工具。旧工具只作为行为参考，
对应能力在新架构中收敛为 CLI 命令或本地 HTTP endpoint：

- `launch_pairing_dashboard` -> `niuma gateway`
- `request_pairing_artifacts` -> `GET /api/pairing/payload`
- `runtime_status` -> `niuma status` 或 `GET /api/status`

### Migration Scope

`niuma-cli` 第一阶段以替换桌面侧实现为主，但允许为配对协议同步修改
`niuma-server` 和 iOS 配对协议。除配对握手外，Rust gateway 仍应兼容当前
`/ws/agent`、task、resume、approval 和 transfer 合约，让迁移风险集中在桌面侧和
配对安全边界。

配对协议纳入第一阶段重构。目标是补齐双端公钥交换，而不是复制旧插件只创建 binding
的流程。项目处于测试阶段，不保留旧配对协议兼容，也不在实现里维护 v1/v2 分支。

配对协议的基本流程：

1. `niuma gateway` 注册 agent，并请求 `pair_token`。
2. gateway 为本次配对生成一次性 X25519 pairing key。
3. QR payload 携带 agent id/name、pair token、过期时间、agent 签名公钥、agent 长期
   encryption 公钥、agent pairing 公钥和 agent 签名。
4. iOS 扫码获取桌面端公钥；模拟器 debug 只跳过扫码，通过本地 HTTP 获取同一 payload。
5. iOS 注册自己的 Ed25519 signing public key 到 server。
6. iOS 生成或读取自己的长期 X25519 encryption key，并用 agent pairing 公钥加密移动端
   handshake，随 `/pair/confirm` 发给 server。
7. server 验证 pair token 和 iOS 签名，创建 binding，并把 encrypted handshake 通过
   agent WebSocket 转发给桌面端。
8. 桌面端解密后保存移动端长期 encryption public key，后续消息和 transfer 才有端到端
   加密基础。

iOS 的 Ed25519 signing key 和 X25519 encryption key 是两类不同密钥。签名 key 用于
注册、鉴权和 envelope 签名；长期 encryption key 保存在 Keychain，用于配对后消息和
transfer 的端到端加密身份。

桌面端也持有长期 X25519 encryption key，保存在 `~/.niuma`。一次性 pairing key 只用于
本次配对握手，不作为长期通信密钥。

配对成功后，双方用各自长期 X25519 encryption key 与对方长期 encryption public key
建立 shared secret，并结合 `binding_id` 通过 HKDF 派生 pair-scoped 对称密钥。后续
task、update 和 transfer payload 使用该对称密钥做 AEAD 加密；长期 X25519 key 不直接
加密每条业务消息。

业务 realtime schema 保留现有 `ciphertext` 外壳。配对协议重构后，`ciphertext` 内部
承载 AEAD envelope，server 继续只验证路由和签名，不解析业务明文。

AEAD envelope 至少包含算法、key id、nonce 和密文。key id 绑定 `binding_id` 或密钥
版本，用于双方选择正确的 pair-scoped 对称密钥。

`niuma-server` 不作为 encryption public key 目录。它不持久化双方长期 encryption
public key；pair confirm 时只验证路由和签名，创建 binding，并把 encrypted handshake
短暂转发给在线 agent。iOS 本地保存 agent encryption public key，desktop 本地保存
mobile encryption public key。

Pair confirm 要求对应 desktop agent 在线。若 agent 不在线或 encrypted handshake 无法
实时投递，server 应让配对失败；不创建待补发的离线 handshake 队列。

Pair confirm 的成功条件包括 desktop agent 已解密 encrypted handshake、保存 mobile
encryption public key，并通过 WebSocket 向 server 返回 ack。server 收到 ack 后再向
iOS 返回 `/pair/confirm` 成功，保证成功响应代表双方密钥状态已经一致。

Pair token 在 agent ack 成功前不应 consumed，也不应提前创建 binding。若 agent ack
失败，server 计入 failed attempts；token 可继续使用直到过期或达到失败次数上限。

QR payload 删除旧 `fingerprint` 字段，改用明确字段表达密钥语义：

```json
{
  "agent_id": "...",
  "agent_name": "...",
  "pair_token": "...",
  "expires_at": 0,
  "agent_signing_public_key": "...",
  "agent_encryption_public_key": "...",
  "agent_pairing_public_key": "...",
  "signature": "..."
}
```

如果 UI 需要展示人工核验值，可单独提供 `agent_signing_key_fingerprint` 等显示字段，
不得复用 `fingerprint`。

`/pair/confirm` 的 iOS 签名不能只覆盖 `device_id`、`agent_id` 和 `pair_token`。签名
digest 必须覆盖至少以下字段，防止 encrypted handshake 被替换：

```text
device_id
agent_id
pair_token
expires_at
encrypted_handshake
agent_pairing_public_key
```

server 验证该签名和 pair token，但不解密 `encrypted_handshake`。

desktop agent 的 pairing ack 也必须签名。ack 使用 agent Ed25519 signing key，digest
至少覆盖：

```text
binding_id
device_id
agent_id
pair_token
handshake_hash
ack_status
```

server 使用已注册 agent signing public key 验证 ack，验证通过后才向 iOS 返回
`/pair/confirm` 成功。

`niuma gateway` 启动完整 runtime 时应立即请求并维护一个有效 pair token，使
`GET /api/pairing/payload` 能快速返回当前可扫码 payload。pair token 接近过期时自动
刷新；页面刷新按钮只负责强制刷新当前 payload。

gateway 页面在已有 paired device 时仍显示新的配对二维码，但 UI 语义是“添加设备”，
不是“未配对”。配对状态和新增设备入口需要分开展示。

### Capability Parity

`niuma-cli` 稳定前必须完整实现当前 `niuma-plugin` 已有的桌面侧业务能力，而不是只做
文本任务或文件传输的子集。验收标准是 Rust gateway 可以在现有 iOS App 和
`niuma-server` 不改协议的前提下，等价替换插件 runtime。

至少包括：

- 桌面 identity、注册、鉴权、配对 payload 和 pair token 流程
- 本地配对页面与 `/api/pairing/payload`
- `/ws/agent` 长连接、心跳、重连和 server 消息处理
- Codex app-server 创建、初始化、thread/turn 调用、历史读取和通知投影
- project/session/thread/approval metadata 同步
- mobile -> desktop 和 desktop -> mobile 文件 transfer
- 移动端文件物化为 Codex 可读的真实本地文件输入
- Codex 输出文件转为 server transfer，并回传 `file_ref`、`file_type`、`transfer_id`
- 不恢复旧 `image_ref`、`image_base64`、`video_ref` 兼容路径
- 不恢复分片传输

配对网页要求保留能力，不要求照搬旧 Python dashboard UI。Rust gateway 必须提供
`GET /`、`GET /api/pairing/payload` 和 `GET /api/status` 等本地 HTTP 能力；页面
可重新设计，只要能展示二维码、连接状态和错误信息。

本地 HTTP 只作为 loopback 控制面，默认监听 `127.0.0.1:8765`。真实手机通过扫描桌面
浏览器页面上的 QR payload 配对，不直接访问本地 HTTP 服务。iOS Simulator debug 可
通过 `GET /api/pairing/payload` 获取同一 payload，跳过摄像头扫码。

默认不监听 `0.0.0.0`，也不把局域网访问作为标准配对路径。

不提供终端二维码作为第二套配对 UI。`niuma gateway` 命令行只打印本地页面 URL 和
状态；二维码展示统一由 loopback 网页控制面承担。

gateway 无论由前台命令还是 service 启动，默认都打开本地网页控制面；显式
`--no-open` 才关闭自动打开。

## 决策

- 桌面侧唯一任务执行源仍是 Codex app-server 协议。
- app-server 创建来源优先级是 Codex.app 内置 `codex`，其次是 PATH 上的
  `codex` CLI。
- 不把 "启动独立 app-server 子进程" 描述为 "连接 Codex App 当前会话"。
- 暂不把 Codex App 当前桌面会话自动刷新作为 `niuma-cli` 的前置目标。
- 不重新引入桌面侧消息历史缓存；Codex runtime 仍是历史和任务状态的事实来源。
- `niuma-plugin` 不再作为长期运行形态设计；`niuma-cli` 稳定后应删除插件实现。
- `niuma-cli` 当前通过 `cargo install --path niuma-cli` 安装，不设计额外分发渠道。
- `niuma service` 当前只支持 macOS launchd：install、start、stop、restart、
  uninstall、status。
- `niuma service install` 写入当前 `niuma` 二进制绝对路径和 `~/.niuma/logs`
  下的 gateway 日志路径，不依赖后台 `PATH`。
- `niuma service install` 不默认启动；显式 `service start` 或 `service install --start`
  才启动后台 gateway。
- `niuma service start` 不抢占已有前台 gateway 或其他占端口进程；重启后台进程使用
  `niuma service restart`。
- `niuma gateway` 是唯一长期运行的桌面 runtime；配对页面、Server WebSocket、
  Codex app-server 适配和文件物化都在这个进程内完成。
- `niuma-cli` 的本地状态根目录是 `~/.niuma`。
- `niuma gateway` 的配置优先级是命令行参数、环境变量、`~/.niuma/config.toml`。
- `niuma gateway` 默认要求 `niuma-server` 可用；只有显式 `--pairing-page-only`
  才允许只启动本地诊断页面。
- Codex app-server 默认不禁用插件；只有显式 `--disable-codex-plugins` 才追加
  `--disable plugins`。
- `niuma-cli` 不迁移旧 MCP 工具；旧插件 MCP 能力转为 CLI 命令或本地 HTTP endpoint。
- 配对协议纳入第一阶段；允许同步修改 `niuma-server` 和 iOS 配对协议以完成双端公钥
  交换，不保留旧配对协议兼容或 v1/v2 分支。
- `niuma-cli` 稳定验收要求完整等价实现当前 `niuma-plugin` 已有业务能力。
- gateway 本地 HTTP 是 `127.0.0.1` loopback 控制面；真实手机扫码页面 QR，不直连
  本地 HTTP。
- Rust 版 `niuma-server` 替换现有 `niuma-server/`，保持独立服务端工程；不并入
  `niuma-cli/`，不把 repo 根目录改成 Cargo workspace。
- Rust 版 `niuma-server` 面向服务器部署，持久化后端继续只支持 PostgreSQL；不引入
  其他控制面存储后端或测试专用存储分支。
- Rust 版 `niuma-server` 第一阶段保持现有 HTTP / WebSocket 外部协议不变，避免同时
  重构 iOS、Gateway 和 Server 三端协议。
- Rust 版 `niuma-server` 不迁移 Python 版 audit service / audit_events 表；第一阶段
  只保留结构化运行日志。
- Rust 版 `niuma-server` 的 transfer payload 继续使用服务端本地临时目录，不进入
  PostgreSQL，也不恢复分片传输或业务文件元数据持久化。
- Rust 版 `niuma-server` 沿用 `niuma-server/.env` 和 `NIUMA_` 配置前缀；数据库 URL
  使用标准 PostgreSQL URL。
- Rust 版 `niuma-server` 当前启动自动建表，不做历史 schema migration。
- `niuma-server/` 替换完成后只保留 Rust server crate；删除 Python 源码、Python 测试
  和 Python 项目配置，不维护双实现。
- Rust 版 `niuma-server` 使用 `axum + tokio + sqlx` 作为核心服务端技术栈，不引入 ORM
  或复杂依赖注入层。
- Rust 版 `niuma-server` 的 WebSocket 只做 payload-blind 认证与路由，不解析业务
  payload，不保存消息历史或 metadata 明文索引。
- Rust 版 `niuma-server` 采用单设备单活 WebSocket；新连接替换旧连接并关闭旧连接。
- Rust 版 `niuma-server` 严格保持 pair token 在 desktop signed ack 成功后才 consumed；
  ack 失败或超时不创建 active binding。
- Rust 版 `niuma-server` 将 challenge 和 session token 持久化到 PostgreSQL；server
  重启不应使未过期 session token 失效。
- Rust 版 `niuma-server` 完全沿用现有 digest/signature 口径，不改签名字段或拼接顺序。
- Rust 版 `niuma-server` 删除持久化 `online_status`；在线状态只由运行时 WebSocket hub
  表示。
- Rust 版 `niuma-server` 的 pair binding 只保留 `active/revoked` 两态，不创建
  pending/failed binding。
- Rust 版 `niuma-server` 的 pair token 保存并校验本次 `agent_pairing_public_key`，防止
  pair confirm 阶段替换二维码公钥。
- Rust 版 `niuma-server` 完成标准包括 Rust 单测、HTTP/WebSocket 集成测试和 iOS
  模拟器真实联调。
- Rust 版 `niuma-server` 实现 APNs / Push Service；通知只携带通用提醒文案和
  Gateway 生成的加密业务定位密文，移动端解密后继续通过 `resume_thread` 获取详情。
- Rust 版 `niuma-server` 第一阶段不实现完整限流/风控，只保留 token TTL、pair failed
  attempts 和 transfer size/TTL 等协议内约束。
