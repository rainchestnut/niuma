# Niuma Desktop Agent 详细设计

## 1. 文档目标

本文档描述桌面 Agent 的详细设计。Desktop Agent 使用 `Rust` 开发，工程落在
`/niuma-cli` 目录，并通过 `cargo install --path niuma-cli` 安装为 `niuma`
命令。旧 Python 插件实现已删除，Rust Gateway 是唯一桌面运行时。

本设计明确采用：

> **通过官方 `Codex App Server` 方式连接 Codex。**

这里的 `app-server` 指 Codex 官方提供的集成协议与运行模式，不是 Niuma 自己额外设计的一层本地服务。

---

## 2. 设计定位

Desktop Agent 是移动端与 Codex 之间的桥接层，承担：

- 设备身份与长连接管理
- 作为 `Codex App Server` 客户端接入 Codex
- 移动协议到 Codex 协议的转换
- 项目 / session 视图投影
- 审批事件代理
- 断线恢复支持

它不是：

- 业务真相源
- 业务历史数据库
- 独立会话管理系统
- 自定义本地 App Server

---

## 3. 官方接入方式选择

### 3.1 为什么选 `Codex App Server`

`Codex App Server` 是 Codex 官方用于 rich client 集成的接口，适用于：

- 会话历史接入
- 审批处理
- 流式 agent 事件
- 深度产品级嵌入

不选择 SDK 的原因：

- 当前目标不是 CI 自动化或离线批处理
- 需要长连接、审批、事件流和已存线程续接

### 3.2 支持的连接方式

`Codex App Server` 支持多种传输方式，当前方案只选其中一种作为主实现：

- `stdio`：默认方式，推荐作为 MVP 主方案
- `websocket`：实验性方式，保留为未来扩展

本项目结论：

- **Desktop Agent 优先使用 Codex.app 自带的 `codex app-server` 接入 Codex**
- **如果 Codex.app 不存在，再回退到 PATH 中的 `codex app-server`**
- **MVP 默认采用 `stdio` 方式**
- Niuma 本地 HTTP 只作为 loopback 控制面和配对页面，不作为自定义 Codex App Server

### 3.3 与 Codex 的实际链路

推荐链路：

`iOS App -> Server -> Niuma Gateway -> Codex App Server -> Codex Core`

其中：

- `Niuma Gateway` 是业务桥接层
- `Codex App Server` 是官方集成协议入口
- `Codex Core` 是真实执行与会话真相源

### 3.4 推荐的进程模型

MVP 推荐：

- Gateway 启动一个 `codex app-server` 子进程
- 通过 `stdio` 进行双向 JSON-RPC 通信
- Gateway 负责该进程的生命周期、健康检查与异常重连

备选方案：

- 若未来 Codex Desktop 自身开放可连接的既有 `app-server` 实例，可改为连接已有实例
- 若未来需要远程化或独立部署，可考虑 `websocket` 模式

当前不建议：

- 让 Niuma 本地 HTTP 承担 Codex 协议代理职责

---

## 4. App Server 集成架构

### 4.1 协议与传输

`Codex App Server` 使用：

- 双向 JSON-RPC 2.0 风格消息
- 默认 `stdio` 传输
- 可选 `websocket`

MVP 传输建议：

- Gateway 启动：`codex app-server`
- 标准输入：发送请求
- 标准输出：读取 response 与 notification
- 标准错误：仅用于日志，不参与协议

### 4.2 初始化握手

每个连接必须：

1. 发送 `initialize`
2. 收到初始化响应
3. 发送 `initialized`
4. 之后才能调用其他方法

若未完成初始化就发请求，应视为协议错误。

### 4.3 Core Primitives

`Codex App Server` 的核心抽象是：

- `thread`
- `turn`
- `item`

Niuma 不直接改造这些抽象，而是做投影：

- `project`：由 Codex 桌面保存的 workspace roots 投影得到
- `conversation bucket`：由 Codex 桌面保存的 `projectless-thread-ids` 投影得到
- `session`：映射自 Codex thread 摘要
- `thread runtime`：映射自当前加载中的 thread + turn + item 流

### 4.4 Gateway 与 App Server 的职责边界

Gateway 负责：

- 设备鉴权
- 与业务 Server 的连接
- 将移动端请求转换为 `thread/*`、`turn/*` 等 app-server 调用
- 将 Codex 通知事件转换为移动端可消费消息

App Server 负责：

- 管理 Codex thread 生命周期
- 驱动 turn 执行
- 输出 item 增量
- 发起审批请求
- 提供 thread 读取、恢复和归档能力

### 4.5 健康检查与版本协商

Gateway 启动后应先完成：

1. 启动 `codex app-server`
2. `initialize`
3. 读取 server capabilities / server info
4. 校验当前版本是否满足 Niuma 所需能力

最低需要的能力集合：

- thread start / resume / read / list
- turn start
- item 流式通知
- 审批通知

推荐能力：

- thread fork
- turn interrupt / steer
- 更细粒度 thread 状态通知

---

## 5. 模块拆分

### 5.1 Gateway Runtime

负责：

- CLI / service 启动
- 配置加载
- 生命周期管理
- `codex app-server` 子进程管理

CLI 命令规则：

- 安装后的二进制名是 `niuma`，顶层命令必须带子命令。
- 顶层入口包括 `niuma gateway`、`niuma service <COMMAND>`、`niuma status` 和 `niuma reset --yes`。
- 长参数使用两个 ASCII 短横线，例如 `--server-url`、`--no-open`、`--help`。
- 短参数使用一个 ASCII 短横线，例如 `-h`、`-V`。
- 不支持长破折号或混合破折号写法，例如 `niuma -—help`。
- 子命令帮助必须按层级查询，例如 `niuma gateway --help`、`niuma service install --help`。

配置优先级：

```text
命令行参数 > 环境变量 > ~/.niuma/config.toml > 内置默认值
```

`niuma gateway` 默认启动完整桌面 runtime；只有显式 `--pairing-page-only` 时才只启动本地配对/诊断页面。
`niuma service install --no-open` 是持久安装参数，会写入 LaunchAgent plist；后续 `niuma service start` 不临时改变 open/no-open 行为。
`niuma status` 只查询 gateway HTTP `/api/status`；`niuma service status` 额外合并 launchd 状态。
`niuma reset --yes` 是破坏性测试命令，会停止/卸载 service 并删除 `~/.niuma`。

### 5.2 Device Identity

负责：

- 首次启动生成 `agent_id`
- 生成长期设备密钥对
- 注册桌面设备身份
- 保存配对状态

### 5.3 Realtime Client

负责：

- 与业务 Server 建立 `/ws/agent` 长连接
- challenge / response 鉴权
- 心跳
- 自动重连

### 5.4 App Server Client

负责：

- 启动或连接 `codex app-server`
- 发送 `initialize` / `initialized`
- 发送 JSON-RPC 请求
- 读取 response 与 notification
- 请求超时与异常恢复

### 5.5 Codex Adapter

负责：

- 将业务动作映射为 app-server 方法
- 管理 thread / turn 调用
- 处理 thread 读取与恢复
- 处理审批相关请求

### 5.6 Session Projection Layer

负责：

- 将 thread 列表投影为移动端 session 列表
- 将 Codex workspace roots 投影为项目维度
- 将 `projectless-thread-ids` 投影为无项目会话维度
- 维护项目、session、thread 三层关系

### 5.7 Recovery Engine

负责：

- 从 Codex `thread/read` 的 turn/item 顺序投影移动端 `seq`
- 管理 `checkpoint`
- 基于 thread 读取结果构造 replay
- 当 app-server 无法直接满足增量恢复时返回明确失败，不引入第二套历史来源

### 5.8 Approval Proxy

负责：

- 接收 app-server 的审批通知
- 转发到 iOS
- 接收移动端决策
- 回写到 app-server

### 5.9 Transfer Store

负责移动端与桌面端之间的图片、视频和文件落盘与格式转换。

约束：

- Transfer Store 默认使用 `~/.niuma/transfers`，与桌面身份、日志和运行态统一收敛到 `~/.niuma`。
- 不使用旧插件安装目录下的 `.niuma-state`，也不迁移旧插件状态。
- 不暴露本地只读下载端点绕过业务 Server。
- 不把文件内容写入 Gateway 的业务状态数据库；当前只按 `transfer_id` 落盘 payload 和 materialized file，不再维护独立 manifest 或校验记录。

推荐目录：

```text
~/.niuma/
  transfers/
    inbound/<transfer_id>/payload.bin
    inbound/<transfer_id>/files/<safe_file_name>
    outbound/<transfer_id>/payload.bin
```

---

## 6. Niuma 视图模型与 App Server 原语映射

### 6.1 Project 映射

Niuma 中的 `project` 不是 app-server 原生对象，而是投影结果。

当前唯一映射方式：

- 读取 Codex 桌面 `~/.codex/.codex-global-state.json`。
- workspace 项目按 `project-order`、`electron-saved-workspace-roots`、`active-workspace-roots` 合并去重。
- `project_id` 使用 workspace root 的稳定哈希，不把本地绝对路径暴露给移动端。
- 无项目对话不生成 project，而是把 `projectless-thread-ids` 下的 thread 投影到 `__conversation__` 会话桶。

不得再从 thread 的任意 `cwd`、rollout metadata 或最近使用目录反推项目；这些来源会制造 Codex 桌面与移动端项目列表不一致。

### 6.2 Session 映射

Niuma 不再单独派生 `session` 实体，移动端列表直接使用 Codex `thread` 投影。

映射关系：

- `thread_id = thread.id`
- `thread title = thread.name / preview`
- `thread updated_at = thread.updatedAt`
- `thread status = thread.status`

### 6.3 Thread Runtime 映射

移动端当前“线程页”展示的内容，来自：

- 当前 thread
- 当前或最近 turn
- turn 下的 items 增量

### 6.4 审批映射

审批不是单独持久化对象源，而是由 app-server 的通知与 thread/item 状态派生。

Gateway 需要把以下内容映射为统一审批模型：

- shell / command execution approval
- file change approval
- app / tool request user input

### 6.5 媒体与文件映射

Codex 当前对不同内容类型的表达并不完全统一，Gateway 必须在 Niuma 的 `content_parts` 与 Codex 当前格式之间做稳定适配。

已确认的输入 / 输出形态：

- 用户图片输入：Codex 消息可包含 `input_image`，其中 `image_url` 为 `data:image/png;base64,...` 这类 data URL。
- 工具或模型图片输出：可能是 `input_image` data URL，也可能是 `{type: "image", data: "...", mimeType: "image/png"}`，或者最终回答里的 Markdown 图片语法。
- 用户文件 / 视频输入：当前主要出现在普通文本块 `# Files mentioned by the user:` 中，条目包含文件名和桌面本地绝对路径。
- Codex 文件 / 视频输出：当前主要通过 Markdown 链接或 Markdown 图片语法引用桌面本地绝对路径，例如 `[file](</absolute/path>)` 或 `![alt](/absolute/path.png)`。

Niuma 统一投影：

- `text`：直接映射为 Codex 文本 item。
- `file_ref`：图片、视频和普通文件统一使用这一种协议形态。
- `file_type`：`file_ref` 的宽类型，当前使用 `image`、`video`、`file`。Codex 原生 raster data URL、Markdown 图片和本地 raster 路径只允许作为 Gateway 内部输入形态，发往移动端前必须转换为 transfer-backed `file_ref`。

安全要求：

- `transfer_id` 是完整 transfer payload 的 SHA-256，Gateway 入站和出站都用它校验本地 payload。
- 原始文件名、MIME、大小和展示说明只进入业务 `content_parts`。
- Server 只接触路由设备、方向、payload 大小、`transfer_id` 和过期时间，不能解析业务文件语义。

移动端到桌面端：

1. iOS 将文本、图片、文件和视频组织成 `content_parts`。
2. 图片、视频和文件统一写成 `file_ref`，用 `file_type` 描述宽类型，并通过 Server transfer 中转完整 payload。
3. Gateway 下载完整 payload，校验 `sha256(payload) == transfer_id`，并保存到 `inbound/<transfer_id>/payload.bin`。
4. Gateway 根据 `content_parts` 将 payload materialize 成 Codex 可读取的本地文件；图片优先转换为 Codex 的真实本地图片输入。
5. Gateway 调用 `turn/start` 或 `turn/steer` 时传入结构化 Codex 输入；普通文件在 Codex 现有能力限制下使用本地路径引用。
6. Gateway 将移动端原始 `content_parts` envelope 绑定到 Codex `turn_id`。后续 live echo 与历史 replay 都以该 envelope 投影用户消息，不能用 Codex 输入适配后的文本替代移动端原始消息。

桌面端到移动端：

1. Gateway 只在 Codex `item/completed` 或 `turn/completed` 后解析稳定 item 和最终消息文本，不能在 `agentMessage/delta` 流式片段里反复读取 thread 并上传媒体。
2. 遇到 raster data URL 图片时，必须同时通过 MIME 和文件签名校验，不能直接塞入 `task_update` 大帧；Gateway 先创建 `agent_to_ios` transfer，再把消息内容改写为带 `transfer_id` 和 `file_type=image` 的 `file_ref`。
3. Gateway 只在 Codex `turn/completed` 后请求 APNs 任务进度推送。推送请求只包含目标
   iOS 设备和端到端加密业务定位密文；过程 `task_update`、`agentMessage/delta` 和
   `item/completed` 不能触发后台通知。
3. 遇到 Markdown 本地图片路径时，确认路径存在、MIME 是 iOS 可直接渲染的 raster 图片且文件签名匹配，再走同一条 `file_ref` transfer 链路。
4. Gateway 用完整 payload SHA-256 作为 `transfer_id`，先 `ensure`，必要时再用 `PUT /transfers/:transfer_id` 一次性上传。
5. iOS 下载并校验 `sha256(data) == transfer_id` 后展示或提供打开入口。

---

## 7. App Server 方法与接口字段表

### 7.1 `initialize`

| 字段 | 方向 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- | --- |
| `clientInfo.name` | request | string | 是 | 客户端内部名，例如 `niuma-cli` |
| `clientInfo.title` | request | string | 是 | 用户可读标题 |
| `clientInfo.version` | request | string | 是 | Gateway 版本 |

### 7.2 `initialized`

| 字段 | 方向 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- | --- |
| `params` | request | object | 是 | 一般为空对象 |

### 7.3 `thread/start`

| 字段 | 方向 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- | --- |
| `model` | request | string | 否 | 模型名 |
| `cwd` | request | string | 否 | 工作目录 |
| `approvalPolicy` | request | string | 否 | 审批策略 |
| `approvalsReviewer` | request | string | 否 | 审批审查人，`user` 或 `guardian_subagent` |
| `sandbox` | request | string | 否 | thread 级 sandbox 模式 |
| `serviceName` | request | string | 否 | 集成名，建议填 `niuma-cli` |
| `thread.id` | response | string | 是 | 新 thread ID |

### 7.4 `thread/resume`

| 字段 | 方向 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- | --- |
| `threadId` | request | string | 是 | 已有 thread ID |
| `cwd` | request | string | 否 | 恢复时覆盖 cwd |
| `approvalPolicy` | request | string | 否 | 可覆盖审批策略 |
| `approvalsReviewer` | request | string | 否 | 可覆盖审批审查人 |
| `sandbox` | request | string | 否 | 可覆盖 thread 级 sandbox 模式 |
| `thread.id` | response | string | 是 | thread ID |

### 7.5 `thread/read`

| 字段 | 方向 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- | --- |
| `threadId` | request | string | 是 | thread ID |
| `includeTurns` | request | boolean | 否 | 是否返回 turns |
| `thread` | response | object | 是 | 线程详情 |

### 7.6 `thread/list`

| 字段 | 方向 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- | --- |
| `cwd` | request | string | 是 | 必须是某个 Codex workspace root；禁止使用 `cwd=null` 做全局扫描 |
| `archived` | request | boolean | 否 | 是否查询已归档 thread |
| `threads` | response | array | 是 | thread 列表 |

### 7.7 `turn/start`

| 字段 | 方向 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- | --- |
| `threadId` | request | string | 是 | 目标 thread |
| `input` | request | array | 是 | 用户输入 items |
| `cwd` | request | string | 否 | 可覆盖工作目录 |
| `approvalPolicy` | request | string | 否 | 可覆盖审批策略 |
| `approvalsReviewer` | request | string | 否 | 可覆盖审批审查人 |
| `sandboxPolicy` | request | object | 否 | 可覆盖 sandbox |
| `turn.id` | response | string | 是 | turn ID |

### 7.8 `turn/steer`

| 字段 | 方向 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- | --- |
| `threadId` | request | string | 是 | 当前 thread |
| `input` | request | array | 是 | 追加输入 |
| `turnId` | response | string | 是 | 当前 turn ID |

### 7.9 `turn/interrupt`

| 字段 | 方向 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- | --- |
| `threadId` | request | string | 是 | 当前 thread |
| `result` | response | object | 是 | 成功一般为空对象 |

### 7.10 通知事件

Gateway 至少需要处理：

- `thread/started`
- `thread/status/changed`
- `turn/started`
- `item/started`
- `item/completed`
- `item/agentMessage/delta`
- `thread/closed`

### 7.11 审批通知

Gateway 至少需要处理：

- `item/commandExecution/requestApproval`
- `item/fileChange/requestApproval`
- `item/tool/requestUserInput`
- `serverRequest/resolved`

说明：

- 审批应按官方 app-server 审批交互 schema 回应
- Niuma 不自定义审批底层协议，只做转发和投影

---

## 8. Gateway 对业务 Server / iOS 的接口字段表

### 8.1 `registerAgent`

| 字段 | 方向 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- | --- |
| `device_type` | request | string | 是 | 固定为 `agent` |
| `device_id` | request | string | 是 | `agent_id` |
| `device_name` | request | string | 是 | 桌面设备名 |
| `os_type` | request | string | 是 | 系统类型 |
| `public_key` | request | string | 是 | 长期签名公钥 |

### 8.2 `pair_request`

| 字段 | 方向 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- | --- |
| `agent_id` | request | string | 是 | Agent ID |
| `pair_token` | response | string | 是 | 一次性 token |
| `expires_at` | response | integer | 是 | 过期时间 |

### 8.3 配对二维码 payload

| 字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `agent_id` | string | 是 | Agent ID |
| `agent_name` | string | 是 | 桌面展示名 |
| `pair_token` | string | 是 | 一次性 token |
| `expires_at` | integer | 是 | token 过期时间 |
| `agent_signing_public_key` | string | 是 | 桌面长期 Ed25519 签名公钥 |
| `agent_encryption_public_key` | string | 是 | 桌面长期 X25519 加密公钥 |
| `agent_pairing_public_key` | string | 是 | 本次配对的一次性 X25519 公钥 |
| `agent_signing_key_fingerprint` | string | 是 | 长期签名公钥展示指纹 |
| `agent_encryption_key_fingerprint` | string | 是 | 长期加密公钥展示指纹 |
| `signature` | string | 是 | 桌面长期签名私钥对 payload digest 的签名 |

### 8.4 入站消息 `task_start`

| 字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `project_id` | string | 是 | 项目 ID |
| `thread_id` | string | 否 | 已有 thread ID；为空时通过 Codex app-server 创建新 thread |
| `ciphertext` | string | 是 | 加密任务正文 |
| `model` | string | 否 | 移动端选择的 Codex 模型 |
| `effort` | string | 否 | 移动端选择的推理强度 |
| `approval_policy` | string | 否 | 移动端权限模式映射出的 Codex 审批策略 |
| `approvals_reviewer` | string | 否 | 移动端权限模式映射出的审批审查人 |
| `sandbox_mode` | string | 否 | 移动端权限模式映射出的 sandbox 模式 |

说明：

- 解密后的任务正文应采用 `content_parts`，而不是只支持单一字符串。
- `content_parts` 中的大文件引用必须先完成 transfer，再进入 Codex app-server 调用。
- Gateway 将 `sandbox_mode` 映射为 `thread/start` / `thread/resume` 的 `sandbox` 字符串，以及 `turn/start` 的 `sandboxPolicy` 对象。
- 移动端不生成本地 session 或 canonical message id；新任务的 thread ID 与稳定消息 ID 来自 Codex app-server 的投影。

### 8.5 出站消息 `task_update`

| 字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `thread_id` | string | 是 | thread ID |
| `seq` | integer | 是 | 单调递增序号 |
| `ciphertext` | string | 是 | 加密增量正文 |
| `checkpoint` | string | 否 | 当前检查点 |
| `role` | string | 否 | 移动端展示角色 |
| `type` | string | 是 | Codex app-server 原生或 Gateway 合成记录的受控 item type；缺失视为投影错误 |
| `phase` | string | 否 | Codex app-server 原生 item phase |
| `entry_id` | string | 否 | Codex/app-server 投影出的稳定条目 ID |

说明：

- 加密增量正文可以包含 `content_parts`。
- 当内容引用 Server 临时 transfer 时，Gateway 必须先完成上传，再发送对应 `transfer_id`。
- 移动端排序来源必须是 Codex/app-server 的 thread/turn/item 顺序。若移动端协议仍保留 `seq`，该值只能由这条 Codex 顺序投影而来，不能由实时通知和历史回放各自计数生成。
- 实时通知和 `thread/read` replay 必须使用同一套稳定条目：用户消息 entry 为 Codex `turn_id`，普通 item 为 `item:<item_id>`，没有 item id 时使用稳定 fallback。
- Gateway 不折叠 Codex item，不生成“已处理 N 项”摘要，也不生成 Niuma 自定义展示分类；只透传 Codex 原生 `type` 和 `phase`，由移动端按连续过程消息做 UI 聚合。

### 8.6 出站消息 `thread_sync_failed`

| 字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `thread_id` | string | 是 | thread ID |
| `cursor` | integer | 是 | 失败请求原 cursor，不代表已确认历史进度 |
| `checkpoint` | string | 否 | 失败请求原检查点 |
| `error` | string | 是 | Codex app-server read 或 projection 失败原因 |

说明：

- `thread_sync_failed` 不替代 `thread_sync_completed`，也不推进移动端 cursor；它只结束本次 resume 的等待状态。

### 8.7 出站消息 `approval_request`

| 字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `approval_id` | string | 是 | 审批 ID |
| `thread_id` | string | 是 | 所属 thread |
| `approval_type` | string | 是 | 审批类型 |
| `ciphertext` | string | 否 | 审批详情 |

### 8.8 入站消息 `approval_response`

| 字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `approval_id` | string | 是 | 审批 ID |
| `decision` | string | 是 | allow / reject |
| `grant_scope` | object | 否 | 授权范围 |
| `grant_scope.scope` | string | 否 | `turn` 或 `session` |

### 8.9 入站消息 `resume_thread`

| 字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `thread_id` | string | 是 | thread ID |
| `cursor` | integer | 是 | 移动端已同步到的消息游标 |
| `checkpoint` | string | 否 | 辅助恢复点 |

---

## 9. 配对与鉴权

### 9.1 首次注册

1. 生成 `agent_id`
2. 生成长期 Ed25519 签名密钥对
3. 生成长期 X25519 加密密钥对
4. 向业务 Server 注册桌面设备身份；Server 只保存签名公钥用于控制面鉴权

### 9.2 配对 token

Agent 负责：

- 请求一次性 `pair_token`
- 生成二维码展示内容
- 在二维码中带上长期签名公钥、长期加密公钥、本次配对的一次性加密公钥和展示指纹
- 使用长期签名私钥签名二维码 payload

### 9.3 会话握手

配对握手由二维码 payload、业务 Server relay 和在线 Gateway ack 共同完成：

1. iOS 扫码后获得桌面长期公钥、本次配对公钥和 `pair_token`。
2. iOS 生成或读取本机长期 X25519 加密公钥。
3. iOS 使用桌面本次配对公钥加密自己的长期加密公钥和握手材料。
4. iOS 调用 `/pair/confirm`，请求中带本机设备名、长期签名公钥、`encrypted_handshake` 和覆盖关键字段的签名。
5. Server 校验 token、请求公钥签名和在线状态，通过后 upsert iOS 设备，但不解密握手内容。
6. Server 将 `encrypted_handshake` relay 给在线 Gateway。
7. Gateway 用本次配对私钥解密，保存移动端长期加密公钥，并对 ack 材料签名。
8. Server 只在收到 Gateway signed ack 后消费 token、创建 binding，并把 ack 返回给 iOS。
9. 后续业务消息使用双方长期 X25519 公钥与 `binding_id` 派生 pair-scoped 对称密钥。

### 9.4 App Server 鉴权与账户态

Gateway 不重新设计 Codex 身份系统。

推荐策略：

- 优先复用桌面已有的 Codex 可用态
- Gateway 只检查 app-server 是否可正常服务
- 若 app-server 返回账户态缺失或不可用，Gateway 向移动端呈现“桌面侧 Codex 不可用”

---

## 10. 状态机

### 10.1 Agent 生命周期状态机

- `booting`
- `starting_app_server`
- `app_server_initializing`
- `registered`
- `connecting`
- `authenticated`
- `online`
- `degraded`
- `offline`

### 10.2 配对状态机

- `unpaired`
- `pair_token_ready`
- `pairing`
- `paired`
- `revoked`

### 10.3 Thread Runtime 状态机

- `idle`
- `thread_started`
- `turn_running`
- `waiting_approval`
- `replaying`
- `completed`
- `failed`

### 10.4 App Server 连接状态机

- `not_started`
- `starting`
- `initializing`
- `ready`
- `protocol_error`
- `degraded`
- `closed`

---

## 11. 实时协议处理

### 11.1 入站消息

- `task_start`
- `approval_response`
- `resume_thread`

### 11.2 出站消息

- `project_sync`
- `thread_sync`
- `task_update`
- `approval_request`
- `task_update`
- `thread_sync_completed`
- `thread_delta`
- `device_status`
- `transfer_ready`

### 11.3 顺序语义

- 每个移动端 `thread` 的 `seq` 只由 Codex `thread/read(includeTurns=true)` 返回的 `turns` 顺序和 turn 内 `items` 顺序投影生成
- Gateway 不在 app-server 通知流上自增 `seq`，通知只触发对当前 Codex thread 快照的重新投影
- 使用 Codex/app-server 投影出的 `entry_id` 做幂等；`seq` 只负责排序和 cursor
- 对恢复请求按 `cursor` 回放
- `transfer_id` 不参与消息排序，只作为内容 part 的外部资源引用；消息进入 UI 前必须确保对应 transfer 可访问或可重试

说明：

- `Codex App Server` 原生并不提供 Niuma 的 `seq` 语义
- `seq` 是 Gateway 对 Codex 既有顺序的薄投影，不保存为独立业务状态

---

## 12. 恢复设计

### 12.1 恢复来源

- 恢复数据来源始终是 Codex thread 数据
- Gateway 不保存完整业务会话副本

### 12.2 恢复策略

优先级如下：

1. `thread/read(includeTurns=true)` 按 Codex turn/item 顺序投影
2. `thread/resume` 后由 app-server 通知触发再次读取 `thread/read`
3. 继续按 `task_update* -> thread_sync_completed` 投影移动端缺失记录

### 12.3 App Server 参与恢复方式

推荐流程：

1. Gateway 收到 iOS 的 `resume_thread`
2. Gateway 调用 `thread/read`
3. 若 thread 未加载，则先 `thread/resume`
4. Gateway 将 `turns + items` 重建为移动端视图
5. Gateway 按 Codex 顺序投影出的 `entry_id` 和 `seq` 发送缺失消息
6. 后续 app-server 通知继续触发同一条 `thread/read` 投影路径

### 12.4 本地保存内容

- 最近 `checkpoint`
- `turn_id -> mobile content_parts envelope` 映射，仅保存移动端发起的 turn，用于保持用户消息 replay 与 live echo 同构；该数据位于本地 Gateway state，不进入 niuma-server。
- 最近已发送 cursor，用于避免同一运行时重复推送已经完成的 Codex 条目
- 必要运行配置

---

## 13. 审批设计

### 13.1 审批来源

审批来自 `Codex App Server` 的官方通知，不由 Niuma 自行生成。

### 13.2 Gateway 处理方式

1. 接收审批通知
2. 解析为统一审批模型
3. 转发到 iOS
4. 接收 iOS 决策
5. 按 app-server 官方审批响应 schema 回写

### 13.3 支持的审批类型

- command execution approval
- file change approval
- tool / app user input approval

### 13.4 Scoped Grant

移动端的 scoped grant 是 Niuma 的产品层语义。

实现方式建议：

- Gateway 在本地维护授权缓存
- 收到重复审批请求时先命中授权缓存
- 再决定是否自动按 app-server schema 响应

---

## 14. App Server 失败处理

### 14.1 失败分类

- `codex app-server` 启动失败
- 初始化握手失败
- 协议版本不兼容
- app-server 可用但 Codex thread 能力异常
- 事件流断开
- turn 调用超时

### 14.2 处理策略

- 启动失败：进入 `degraded`
- 初始化失败：不进入 `online`
- 版本不兼容：阻止继续接入，要求升级
- 能力异常：向业务 Server 报告桌面在线但业务不可执行
- 事件流断开：重建 app-server 连接并恢复 thread
- 超时：有限次重试，超过阈值后降级

---

## 15. 子项目目录结构

建议 `/niuma-cli` 目录采用如下结构：

```text
niuma-cli/
  README.md
  Cargo.toml
  src/
    main.rs
    cli.rs
    config.rs
    gateway.rs
    identity.rs
    pairing.rs
    server.rs
    realtime.rs
    codex.rs
    codex_app_server.rs
    metadata.rs
    tasks.rs
    transfers.rs
    service.rs
    status.rs
```

关键目录说明：

- `cli.rs`：`niuma gateway`、`niuma service`、`niuma status`、`niuma reset` 命令声明
- `gateway.rs`：前台 gateway runtime、loopback 配对页面和状态接口
- `service.rs`：macOS LaunchAgent 安装、启动、停止、重启、卸载和状态查询
- `identity.rs`：`~/.niuma/identity` 下的桌面身份与密钥持久化
- `pairing.rs`：配对二维码 payload、一次性配对密钥和 signed ack 材料
- `codex_app_server.rs`：官方 app-server 客户端、进程管理、JSON-RPC response / notification 分发
- `metadata.rs`：Codex workspace root 读取与 project / session / thread 视图投影
- `tasks.rs`：`task_start`、`resume_thread`、Codex turn replay 与 `task_update` 映射
- `server/`：业务 Server HTTP / WebSocket 协议模型与客户端
- `realtime.rs`：业务 Server `/ws/agent` 长连接
- `transfers.rs`：`~/.niuma/transfers` 下的媒体和文件收发、校验、Codex 路径映射

---

## 16. MVP 任务拆解

### 16.1 Phase 1：Rust CLI 骨架

- 初始化 Rust 工程
- 建立 `niuma gateway` / `niuma service` / `niuma status` / `niuma reset`
- 建立顶层和子命令 help 规则，明确只接受 ASCII `-` / `--` 参数写法
- 建立 `~/.niuma` 状态目录、配置加载和 LaunchAgent 管理

### 16.2 Phase 2：业务 Server 连接

- 生成 `agent_id`
- 生成签名密钥和加密密钥
- 实现设备注册
- 实现 challenge / response 鉴权
- 实现 `/ws/agent` 长连接和心跳
- 实现二维码 payload、一次性配对密钥、加密握手 relay 和 signed ack

### 16.3 Phase 3：Codex App Server 集成

- 以子进程方式启动 `codex app-server`
- 实现 `initialize` / `initialized`
- 实现 JSONL `stdio` 收发
- 实现 response / notification 分发；当前 Rust 实现会把 app-server notification 广播给 realtime runtime

### 16.4 Phase 4：Codex Thread/Turn 适配

- 实现 `thread/start`
- 实现 `thread/resume`
- 实现 `thread/read`
- 实现按 workspace root 调用的 scoped `thread/list`
- 实现 `turn/start`
- 当前 Rust 实现已覆盖 metadata refresh、`task_start` 和 `resume_thread` 的基础 app-server 调用

### 16.5 Phase 5：移动协议映射

- 建立 `project` 投影
- 建立 `session` 投影
- 建立 `seq` 语义
- 将 item 增量包装为 `task_update`
- 建立 `content_parts` 与 Codex item / Markdown / 文件路径之间的映射
- 当前 Rust 实现已覆盖文本、带本地路径的图片输入，以及只有 `transfer_id` 的 iOS 入站文件引用；Gateway 会下载 server transfer、校验 SHA-256 后 materialize 成 Codex 可读路径

### 16.6 Phase 6：审批与恢复

- 接入官方审批通知
- 转发审批到 iOS
- 回写审批决策
- 实现 `resume_thread`
- 实现 thread 重建与 replay
- 实现图片、视频和文件 transfer 的入站 / 出站适配
- 当前 Rust 实现已覆盖 app-server notification 触发的活跃 thread replay、审批请求/响应、request-user-input 请求/响应、iOS 入站 transfer materialization，以及 Codex 出站 inline 图片转 `agent_to_ios` transfer

### 16.7 Phase 7：联调与稳态

- App Server 异常恢复
- thread 恢复测试
- 审批链路联调
- 与 iOS / Server 端联调

---

## 17. 失败处理

- Server 断连：自动重连并重新鉴权
- `codex app-server` 不可用：上报桌面受限状态
- Codex thread 读取失败：返回可恢复错误
- 审批回写失败：维持待处理态并提示重试
- 恢复失败：返回可恢复错误，不能伪造本地完成态
- Transfer 下载或校验失败：不进入 Codex 调用，向 iOS 返回可重试错误
- Transfer 上传到 Server 失败：保留本地 outbound 临时文件，按 TTL 后清理

---

## 18. 待确认项

- MVP 是否只支持 `stdio`，暂不开放 `websocket`
- Codex Desktop 未来是否开放可直接连接的既有 app-server 实例
- 审批响应的最终 app-server schema 对接细节
