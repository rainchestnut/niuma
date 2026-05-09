# Niuma Server 端详细设计

## 1. 文档目标

本文档描述 Niuma Server 的详细设计。Server 使用 `Rust` 开发，工程落在 `/niuma-server` 目录，仅承担控制面职责，不保存业务 payload。

---

## 2. 职责边界

Server 负责：

- 设备注册
- challenge / response 设备鉴权
- 配对 token 管理
- iOS 与 Desktop Agent 长连接管理
- WebSocket 路由
- 元数据实时转发

Server 不负责：

- 保存业务明文
- 保存业务密文历史
- 保存 prompt / 输出 / diff / 审批正文

---

## 3. 技术栈建议

- 开发语言：`Rust`
- Web 框架：`axum`
- 实时连接：WebSocket
- 持久化：PostgreSQL（本地运行读取 `niuma-server/.env` 中的 `NIUMA_DATABASE_URL`）
- 异步运行时：`tokio`
- PostgreSQL 访问：`sqlx`

运行时存储约束：

- Server 启动必须有有效的 `postgresql://` 或 `postgres://` 连接串。
- `.env` 是本地桌面端 / 移动端联调的配置来源。
- 缺少数据库连接串或使用非 PostgreSQL URL 都属于配置错误，启动应失败。

---

## 4. 模块划分

### 4.1 Device Registry

负责：

- 注册 `ios_device_id`
- 注册 `agent_id`
- 保存设备公钥
- 保存最后在线时间

### 4.2 Device Auth

负责：

- 生成一次性 challenge
- 验证设备签名
- 校验 `nonce`
- 校验时间窗口

### 4.3 Pairing Service

负责：

- 生成 `pair_token`
- 校验 `pair_token`
- 建立 `ios_device_id <-> agent_id` 绑定
- 撤销配对关系

### 4.4 Realtime Gateway

负责：

- `/ws/mobile`
- `/ws/agent`
- 会话级连接管理
- 点对点路由
- 设备在线状态广播

### 4.5 Metadata Relay

负责：

- 校验 WebSocket 连接与配对关系
- 转发 `project_sync`、`thread_sync`、`approval_sync`
- 转发 `metadata_refresh_completed` / `metadata_refresh_failed`
- 不保存 project、thread、approval 明文索引

### 4.6 Protection

负责：

- challenge/session/pair token TTL
- pair token 失败次数统计
- transfer TTL 和最大 payload 大小

开始实现 APNs / Push Service；不迁移 audit service / audit_events，不实现完整限流系统。

### 4.7 Temporary Transfer Relay

负责图片、视频和文件的短时中转。该模块只处理传输生命周期，不理解文件业务语义。

职责：

- 根据调用端提供的 content-hash `transfer_id` 创建或刷新上传 / 下载上下文。
- 接收 iOS 或 Desktop Agent 上传的完整 transfer payload。
- 校验 `sha256(body) == transfer_id`、payload 大小和设备绑定关系。
- 通过 WebSocket 通知目标端拉取或继续下载。
- 在目标端 ACK 时确认本地可用并刷新有效期；TTL 到期、传输失败或服务启动清理时删除临时文件。

明确不负责：

- 不把文件 blob 写入 PostgreSQL。
- 不永久保存用户图片、视频、文件或消息正文。
- 不读取原始文件名、MIME、展示说明等业务元数据；这些内容只存在于业务 `content_parts` 中。
- 不为本地开发提供绕过 Server 的直连下载端点。

---

## 5. 数据模型

### 5.1 ios_devices

- `ios_device_id`
- `device_name`
- `public_key`
- `push_token`
- `last_seen_at`

### 5.2 agents

- `agent_id`
- `device_name`
- `os_type`
- `public_key`
- `last_seen_at`

### 5.3 pair_bindings

- `binding_id`
- `ios_device_id`
- `agent_id`
- `status`
- `created_at`
- `revoked_at`

### 5.4 metadata index tables

已取消。Server 不再持久化 project、session、thread、approval 明文索引；这些数据只在 WebSocket 上从桌面 Gateway 转发到移动端，由移动端 SwiftData 保存。

### 5.5 transfer_manifest（临时运行时模型）

`transfer_manifest` 是短时传输清单，不属于长期业务数据模型。它落在服务本地临时目录，例如 `niuma-server/.tmp/transfers/<transfer_id>/manifest.json`，不写入 PostgreSQL。`transfer_id` 是完整 payload 的 SHA-256 内容地址，因此不再需要独立校验字段、分片状态或完成状态字段。

- `transfer_id`
- `source_device_id`
- `target_device_id`
- `direction`：`ios_to_agent` 或 `agent_to_ios`
- `encrypted_size_bytes`
- `expires_at`

临时目录规则：

- 服务启动时清理已过期或无 manifest 的孤儿文件。
- 单个 transfer 收到 ACK 后刷新有效期，用于同一内容地址的短期复用。
- 超过 TTL 或 payload 不完整的 transfer 由后台清理任务删除。

---

## 6. API 接口字段表

### 6.1 `POST /devices/register`

| 字段 | 方向 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- | --- |
| `device_type` | request | string | 是 | 当前由桌面 Gateway 以 `agent` 注册自身；iOS 设备由 `/pair/confirm` 内部 upsert |
| `device_id` | request | string | 是 | 设备 ID |
| `device_name` | request | string | 是 | 设备名 |
| `public_key` | request | string | 是 | 长期签名公钥 |
| `os_type` | request | string | 否 | Agent 注册时建议提供 |
| `registered` | response | boolean | 是 | 是否成功 |
| `server_time` | response | integer | 是 | 服务端时间 |

### 6.2 `POST /auth/challenge`

| 字段 | 方向 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- | --- |
| `device_id` | request | string | 是 | 发起鉴权的设备 ID |
| `challenge_id` | response | string | 是 | challenge ID |
| `challenge` | response | string | 是 | challenge 内容 |
| `expires_at` | response | integer | 是 | 过期时间 |

### 6.3 `POST /auth/verify`

| 字段 | 方向 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- | --- |
| `device_id` | request | string | 是 | 设备 ID |
| `challenge_id` | request | string | 是 | challenge ID |
| `timestamp` | request | integer | 是 | 请求时间 |
| `nonce` | request | string | 是 | 防重放随机串 |
| `request_digest` | request | string | 是 | 请求摘要 |
| `signature` | request | string | 是 | 设备签名 |
| `verified` | response | boolean | 是 | 验签结果 |
| `session_token` | response | string | 否 | 短时连接凭据 |

### 6.4 `POST /pair/request`

| 字段 | 方向 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- | --- |
| `agent_id` | request | string | 是 | Agent ID |
| `agent_pairing_public_key` | request | string | 是 | 本次二维码的一次性配对公钥，Server 将其与 pair token 绑定 |
| `pair_token` | response | string | 是 | 一次性配对 token |
| `expires_at` | response | integer | 是 | 过期时间 |

### 6.5 `POST /pair/confirm`

| 字段 | 方向 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- | --- |
| `device_id` | request | string | 是 | iOS 设备 ID |
| `agent_id` | request | string | 是 | Agent ID |
| `pair_token` | request | string | 是 | 一次性 token |
| `agent_pairing_public_key` | request | string | 是 | 本次配对公钥，必须与二维码 payload 一致 |
| `encrypted_handshake` | request | string | 是 | Server payload-blind relay 给在线 Gateway 的握手密文 |
| `signature` | request | string | 是 | iOS 设备签名 |
| `binding_id` | response | string | 是 | 配对绑定 ID |
| `status` | response | string | 是 | 配对状态 |
| `agent_ack` | response | object | 是 | Gateway signed ack |

### 6.6 Metadata reads

已取消 `/projects`、`/sessions`、`/threads`、`/approvals` HTTP 读接口。Server 只认证和转发 metadata sync，不保存明文列表数据。

### 6.7 Transfer APIs

Transfer API 用于短时中转媒体和文件。当前实现用 `X-Session-Token` 表示已经通过设备签名认证后的短会话；同时校验参与设备 ID、配对关系和 transfer 所属绑定关系。上传端接口通过 `X-Device-ID` 指明当前请求设备，下载接口通过 `device_id` 查询参数指明接收设备。

#### `POST /transfers/:transfer_id/ensure`

| 字段 | 方向 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- | --- |
| `transfer_id` | request path | string | 是 | 完整 payload 的 SHA-256 内容地址 |
| `source_device_id` | request | string | 是 | 上传端设备 ID |
| `target_device_id` | request | string | 是 | 接收端设备 ID |
| `direction` | request | string | 是 | `ios_to_agent` 或 `agent_to_ios` |
| `encrypted_size_bytes` | request | integer | 是 | 完整 payload 大小，字段名沿用现有 API |
| `transfer_id` | response | string | 是 | 服务端确认的内容地址 |
| `expires_at` | response | integer | 是 | 过期时间 |
| `needs_upload` | response | boolean | 是 | 服务端是否缺少完整 payload |

#### `PUT /transfers/:transfer_id`

| 字段 | 方向 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- | --- |
| `transfer_id` | request path | string | 是 | 完整 payload 的 SHA-256 内容地址 |
| `X-Device-ID` | request header | string | 是 | 上传端设备 ID，必须等于 transfer 的 source |
| `body` | request | binary | 是 | 完整 payload，Server 校验 `sha256(body) == transfer_id` |
| `uploaded` | response | boolean | 是 | 是否上传成功 |
| `expires_at` | response | integer | 是 | 刷新后的过期时间 |

#### `GET /transfers/:transfer_id`

| 字段 | 方向 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- | --- |
| `transfer_id` | request path | string | 是 | 传输 ID |
| `device_id` | request query | string | 是 | 下载端设备 ID，必须等于 transfer 的 target |
| `body` | response | binary | 是 | 完整 transfer payload |

#### `POST /transfers/:transfer_id/ack`

| 字段 | 方向 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- | --- |
| `receiver_device_id` | request | string | 是 | 完成接收的设备 ID |
| `acknowledged` | response | boolean | 是 | ACK 后服务端确认目标端已有本地副本并刷新 TTL |

---

## 7. WebSocket 详细设计

### 7.1 建连前提

- 设备已注册
- challenge / response 验签成功
- 短时连接凭据有效

### 7.2 连接状态

- `connecting`
- `authenticated`
- `routable`
- `closing`
- `closed`

### 7.3 路由规则

- iOS 只能访问已配对的 Agent
- Agent 只能接收与自己绑定的 iOS 请求
- 未配对设备不允许读项目、session、thread、approval 元数据
- 配对关系在 WebSocket 建连时校验，并把移动端连接绑定到对应 Agent；逐条 `task_update` 只按已认证连接路由，不在热路径重复查询存储。

### 7.4 入站消息字段

#### `task_start`

| 字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `device_id` | string | 是 | iOS 设备 ID |
| `agent_id` | string | 是 | Agent ID |
| `project_id` | string | 是 | 项目 ID |
| `thread_id` | string | 否 | 已有 thread ID；新任务不传，由桌面 Codex/app-server 创建 |
| `ciphertext` | string | 是 | 加密业务内容 |
| `signature` | string | 是 | 设备签名 |
| `model` | string | 否 | 当前选择的 Codex 模型 |
| `effort` | string | 否 | 当前选择的推理强度 |
| `approval_policy` | string | 否 | 移动端权限模式映射出的 Codex 审批策略 |
| `approvals_reviewer` | string | 否 | 移动端权限模式映射出的审批审查人 |
| `sandbox_mode` | string | 否 | 移动端权限模式映射出的 sandbox 模式 |

说明：

- 移动端不生成 canonical message id，也不创建本地 session。消息入库、真实 thread id 与稳定 item id 由桌面 Codex / app-server 负责。
- Server 只校验签名并转发这些权限覆盖字段，不解释其语义。

#### `resume_thread`

| 字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `thread_id` | string | 是 | thread ID |
| `cursor` | integer | 是 | 移动端已同步到的消息游标 |
| `checkpoint` | string | 否 | 补充检查点 |

### 7.5 出站消息字段

#### `task_update`

| 字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `thread_id` | string | 是 | thread ID |
| `seq` | integer | 是 | 增量序号 |
| `ciphertext` | string | 是 | 加密后的更新内容 |
| `checkpoint` | string | 否 | 当前检查点 |
| `role` | string | 否 | 移动端展示角色 |
| `type` | string | 是 | Codex app-server 原生或 Gateway 合成记录的受控 item type；缺失时拒绝该消息 |
| `phase` | string | 否 | Codex app-server 原生 item phase |
| `entry_id` | string | 否 | Codex/app-server 投影出的稳定条目 ID |

#### `thread_sync_completed`

| 字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `thread_id` | string | 是 | thread ID |
| `cursor` | integer | 是 | 本批同步完成后的服务端游标 |
| `checkpoint` | string | 否 | 本批同步完成后的检查点 |
| `entry_count` | integer | 否 | 本批同步的消息条数 |

说明：

- `thread_sync_completed` 是 session 详情同步的唯一完成标识。移动端不得根据超时、空闲或 metadata refresh 推断同步完成。

#### `thread_sync_failed`

| 字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `thread_id` | string | 是 | thread ID |
| `cursor` | integer | 是 | 失败请求原 cursor，不代表可确认历史进度 |
| `checkpoint` | string | 否 | 失败请求原检查点 |
| `error` | string | 是 | Gateway 读取或投影失败原因 |

说明：

- 失败事件只用于结束移动端的同步中状态和展示错误；Server 转发但不解释业务 payload，也不把失败 cursor 当作历史提交点。

#### `approval_request`

| 字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `approval_id` | string | 是 | 审批 ID |
| `thread_id` | string | 是 | 所属 thread |
| `approval_type` | string | 是 | 审批类型 |
| `ciphertext` | string | 否 | 实时场景下的加密审批详情 |

#### `transfer_ready`

| 字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `transfer_id` | string | 是 | 临时传输 ID |
| `direction` | string | 是 | 传输方向 |
| `source_device_id` | string | 是 | 上传端设备 ID |
| `target_device_id` | string | 是 | 接收端设备 ID |
| `encrypted_size_bytes` | integer | 是 | 完整 payload 大小，字段名沿用现有 API |
| `expires_at` | integer | 是 | 过期时间 |

说明：

- `transfer_ready` 只通知目标端有文件可取，不承载文件内容。
- 原始文件名、MIME 和展示说明随业务消息到达目标端，Server 不参与解析。
- 文件内容通过 Transfer API 拉取，拉取完成后目标端必须调用 ACK。
- `transfer_id` 本身就是完整 payload 的 SHA-256，不再单独发送内容校验字段。

---

## 8. 状态机

### 8.1 设备鉴权状态机

- `unknown`
- `registered`
- `challenge_issued`
- `verified`
- `rejected`

流转：

1. `unknown -> registered`
2. `registered -> challenge_issued`
3. `challenge_issued -> verified`
4. `challenge_issued -> rejected`
5. `rejected -> challenge_issued`

### 8.2 配对状态机

- `unpaired`
- `token_issued`
- `confirming`
- `paired`
- `revoked`
- `expired`

### 8.3 路由连接状态机

- `disconnected`
- `connected`
- `authenticated`
- `routable`
- `stale`
- `closed`

---

## 9. 推送设计

APNs / Push Service 用于后台、锁屏或离线场景下提醒移动端“任务有新的进度”。Server
只保存和更新 iOS `push_token`，并在收到已配对 Gateway 的 `task_progress_push` 请求后
调用 APNs。

安全边界：

- 推送展示文案固定为通用提醒，不包含 session 标题、prompt、审批详情、diff 或执行
  结果。
- Gateway 负责在 Codex turn 终态后生成加密业务定位密文；密文至少包含
  `thread_id`，由 iOS 点击通知后解密。
- Server 不解密、不解析、不持久化推送密文，只校验 agent 与 iOS device 的 active
  binding，并把密文放入 APNs 自定义 payload。
- `task_update` 过程消息不能触发 APNs。只有 Gateway 确认 Codex turn 已到达终态后，
  才能发送任务进度推送请求。
- APNs 不是消息历史或离线队列；移动端点击通知后必须通过现有 `resume_thread` 详情
  刷新链路获取最新数据。

---

## 10. 安全设计

### 10.1 设备鉴权

- challenge 一次性使用
- `nonce` 去重
- 时间窗口严格校验
- 公钥验签失败立即拒绝

### 10.2 暴露面控制

- 公开接口仅保留注册、challenge、配对、WebSocket 路由和临时 transfer relay
- 配对 token 短时有效
- 配对尝试次数限制
- 第一阶段不实现完整限流系统，只保留 TTL、失败次数和 payload 大小约束

### 10.3 数据最小化

- 不记录业务 payload
- 不迁移 Python audit service / audit_events；只保留运行日志
- Transfer 临时文件不进入数据库；ACK 只确认目标端本地可用，过期或启动清理时删除临时文件

---

## 11. 子项目目录结构

建议 `/niuma-server` 目录采用如下结构：

```text
niuma-server/
  Cargo.toml
  Cargo.lock
  README.md
  src/
    main.rs
    routes.rs
    config.rs
    crypto.rs
    db.rs
    hub.rs
    models.rs
    transfer.rs
```

关键目录说明：

- `routes.rs`：HTTP / WebSocket 协议入口
- `db.rs`：PostgreSQL 控制面状态
- `hub.rs`：单设备单活 WebSocket 网关
- `transfer.rs`：临时媒体和文件中转
- `crypto.rs`：challenge、pair、task_start 的 digest / signature 口径

---

## 12. MVP 任务拆解

### 12.1 Phase 1：基础框架

- 初始化 Rust 工程
- 建立基础配置与日志
- 启动时自动建表

### 12.2 Phase 2：设备注册与鉴权

- 实现桌面 Gateway `POST /devices/register`
- 在 `POST /pair/confirm` 内实现 iOS 设备 upsert
- 实现 `POST /auth/challenge`
- 实现 `POST /auth/verify`
- 实现 nonce 去重与时间窗口校验

### 12.3 Phase 3：配对

- 实现 `POST /pair/request`
- 实现 `POST /pair/confirm`，并在 Gateway signed ack 成功后才消费 token、创建 binding
- 实现 `POST /pair/revoke`
- 实现配对 token 失效与失败次数限制

### 12.4 Phase 4：实时路由

- 实现 `/ws/mobile`
- 实现 `/ws/agent`
- 实现已配对设备路由校验
- 实现单设备单活连接管理；不持久化 `online_status`

### 12.5 Phase 5：元数据转发与 transfer

- projects / sessions / threads / approvals realtime relay
- 临时 transfer relay、TTL 清理和大小限制

### 12.6 Phase 6：风控与收尾

- 指标埋点
- 集成联调

---

## 13. 待确认项

- 短时连接凭据是否单独签发
- WebSocket 水平扩展方案
- 临时 transfer 的默认 TTL、最大文件大小和并发传输上限
