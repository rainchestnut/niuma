# Niuma iOS 端详细设计

## 1. 文档目标

本文档描述 Niuma iOS 客户端的详细设计。客户端使用 `Swift` 开发，工程落在 `/niuma` 目录，负责移动端 UI、设备身份、本地安全、消息同步、审批处理和推送唤醒。

---

## 2. 设计目标

- 提供精简版 Codex 信息架构
- 支持项目、session、thread 的浏览与续接
- 支持移动端发起新任务、向已有 session 继续发送、审批决策
- 在无账号体系下完成匿名设备初始化、配对和设备级鉴权
- 在前后台切换和弱网环境下保持状态一致性

---

## 3. 技术约束

- 开发语言：`Swift`
- 最低运行版本：建议以现代并发能力为前提
- 推送通道：`APNs`
- 本地密钥存储：`Keychain`
- 本地敏感展示保护：`Face ID / Passcode` 可选

---

## 4. 客户端模块拆分

### 4.1 App Shell

负责：

- App 生命周期管理
- 启动路由
- 全局依赖注入
- 前后台状态切换协调

### 4.2 Device Identity

负责：

- 首次启动生成 `ios_device_id`
- 生成长期设备密钥对
- 向 Server 注册匿名设备身份
- 在重装恢复或 iCloud 同步场景中恢复设备材料

### 4.3 Pairing

负责：

- 扫码与配对码输入
- 保存已绑定桌面设备信息
- 保存 `agent_signing_key_fingerprint`
- 保存 `agent_encryption_key_fingerprint`
- 首次配对时校验 Agent 身份

### 4.4 Session Browser

负责：

- 获取设备列表
- 接收 Gateway 推送的 workspace 项目列表
- 接收 Gateway 推送的 projectless conversation session 列表
- 维护最近 session 和本地展示缓存

### 4.5 Thread Runtime

负责：

- 发起任务
- 跟进提问
- 接收流式输出
- 基于 `seq` 渲染增量
- 基于 `cursor` 记录连续确认进度
- 重连后触发 `resume_thread`

### 4.6 Approval Center

负责：

- 展示审批入口
- 拉取审批详情
- 发出本次 allow、会话级 allow 或 reject
- 支持 scoped grant 选项
- 在 session 详情选择默认权限、自动审查、完全访问或自定义权限模式

### 4.7 Crypto Layer

负责：

- 业务 payload 加密 / 解密
- 会话密钥管理
- 请求签名材料准备

### 4.8 Push Coordinator

负责：

- APNs token 注册
- 推送到达后的唤醒处理
- 将提醒映射到线程恢复或审批读取动作
- 前台收到任务进度推送时，只在当前已打开的 session 详情与通知 thread 相同时抑制系统展示；其他前台页面、后台、锁屏和离线场景都应展示通知。

### 4.9 Media Transfer

负责图片、视频和文件在移动端的选择、编码、缓存、上传、下载和展示。

职责：

- 将用户输入组织成 `content_parts`。
- 图片、视频和普通文件统一走 transfer，并在 `content_parts` 中以 `file_ref` + `file_type` 引用形式发送。
- 以 `transfer_id` 为键维护统一本地附件缓存：移动端已发送附件和从 Desktop Gateway 接收的附件都从这里回显和预览。
- 从 Desktop Gateway 收到 `file_ref` 后，先按 `transfer_id` 命中本地附件缓存，未命中时从 Server 临时缓存下载后渲染。
- 在 transfer 下载完成后发送 ACK，让 Server 确认目标端已有本地副本并刷新临时缓存 TTL。

约束：

- 不把下载的业务文件保存为长期真相源；本地只做展示缓存。
- 不依赖同机模拟器直连桌面 Gateway 的临时下载端点。
- 文件、视频和大图片必须显示传输进度、失败重试和过期提示。

---

## 5. 页面设计

### 5.1 首页

结构：

- 未配对时：直接展示配对入口，不再额外展示“未配对/当前状态”卡片
- 已配对时顶部：已配对桌面与在线状态
- 已配对时中部：项目列表
- 已配对时底部：无项目对话 / 最近 session 列表

用户动作：

- 未配对时在首页直接完成首次配对
- 切换目标桌面
- 添加新的桌面设备
- 进入项目
- 进入最近 session
- 查看待审批数量

### 5.2 配对页

功能：

- 编辑并应用 Niuma Server 地址，确保移动端与桌面 Gateway 指向同一个 Server
- 扫描桌面 Gateway 页面二维码
- 在 DEBUG / 模拟器场景下读取当前桌面 Gateway 二维码 payload 模拟扫码
- 展示扫码结果摘要，用户确认后才发起配对
- 配对成功确认

交互约束：

- 首次配对入口和“添加桌面设备”入口复用同一配对界面；差异只在成功后是否返回上一页。
- 已有绑定时，入口语义是“添加桌面设备”，不是覆盖或重置当前绑定。
- 未配对状态是本地生命周期状态，不作为独立业务状态卡片展示。

### 5.3 Session 列表页

功能：

- 展示项目下 session 列表
- 按更新时间排序
- 支持进入已有 session
- 支持创建新 session
- 行操作菜单支持修改 Codex 原 thread 标题、重置本地历史、归档 thread；修改标题必须等待
  Gateway 回传 `thread_sync` 后刷新列表，不直接覆盖本地缓存。

### 5.4 线程页

功能：

- 展示 thread 输出
- 展示运行状态
- 已有 session 继续输入
- 恢复提示
- 审批入口
- 展示图片、视频和文件内容 part
- 支持发送图片、视频和文件附件
- 用户消息与 agent 最终消息通过左右对齐和气泡颜色区分，不额外显示“你”或“Codex”角色文字

### 5.5 审批页

功能：

- 展示审批详情
- allow / reject
- 可选 scoped grant

---

## 6. 本地数据模型

### 6.1 本地持久化内容

- `ios_device_id`
- 长期设备私钥引用
- 已配对桌面列表
- `agent_public_key_fingerprint`
- workspace 项目列表
- session 列表
- thread 本地展示缓存
- `checkpoint`
- `cursor`
- 媒体和文件展示缓存
- transfer 进度、重试状态和本地缓存路径

### 6.2 不落地内容

- 不长期保存完整会话历史作为真相源
- 不保存服务端业务数据副本
- 不保存额外审批正文历史库
- 不把下载的图片、视频和文件作为长期业务归档；缓存可按容量、TTL 或用户操作清理

---

## 7. 关键流程

### 7.1 首次启动

1. 生成 `ios_device_id`
2. 生成长期设备密钥对
3. 本地保存身份；不单独调用 `/devices/register`
4. 首次扫码配对时通过 `/pair/confirm` 带上设备名和签名公钥，由 Server 在配对流程内 upsert iOS 设备

### 7.2 首次配对

1. 扫描二维码得到 `agent_id`、`pair_token`、桌面长期签名公钥、桌面长期加密公钥和本次配对公钥。
2. 校验二维码 payload 签名与展示指纹。
3. 生成或读取 iOS 长期 X25519 加密公钥。
4. 使用本次配对公钥加密 iOS 长期加密公钥和握手材料。
5. 请求 Server 确认配对，请求中同时包含 iOS 设备名和长期签名公钥；Server 先校验 pair token 和签名，再 upsert iOS 设备并 relay 给在线 Gateway。
6. 校验 Gateway signed ack 后，本地保存绑定信息。

### 7.3 发起任务

1. 选择设备、项目、session
2. 输入任务内容
3. 加密 payload
4. 通过实时通道发送 `task_start`
5. 仅在本地 UI 显示临时发送态，等待桌面 Codex 回放稳定消息 ID

### 7.4 流式展示

1. 收到带 `seq` 的事件
2. 检查是否重复
3. 插入 UI 流
4. 更新 `cursor`
5. 按策略批量回 ACK

### 7.5 恢复

1. 检查当前 thread 的 `cursor`
2. 重连后发送 `resume_thread`
3. 接收逐条 `task_update`
4. replay 阶段逐条写入 SwiftData，但不让每条历史消息驱动详情页重绘。
5. 收到 `thread_sync_completed` 后更新本地 cursor，再按 Codex `seq` 从 SwiftData 加载并刷新 UI；移动端不根据时间、空闲状态或临时消息 ID 推断同步完成。

### 7.6 媒体和文件发送

1. 用户在输入框选择图片、视频或文件。
2. iOS 计算完整附件 payload 的 SHA-256，并把该值作为 `transfer_id`。
3. iOS 调用 `POST /transfers/:transfer_id/ensure`，只提交路由字段和完整 payload 大小；当前 API 字段名沿用 `encrypted_size_bytes`。如果服务端返回 `needs_upload=false`，说明同一内容地址已有完整临时 payload。
4. 需要上传时，iOS 用 `PUT /transfers/:transfer_id` 一次性上传完整 payload，不再分片，不再发送独立内容校验字段。
5. iOS 在 `content_parts` 中写入 `file_ref`，用 `file_type`、`file_name`、`mime_type`、`size_bytes` 和 `alt` 描述展示元数据。
6. iOS 将附件写入本地附件缓存，并在 SwiftData 中保存 `transfer_id -> localRelativePath` 映射。
7. iOS 发送 `task_start`；继续已有 thread 时携带 `thread_id`，新会话等待桌面 Codex/app-server 回传真实 thread。
8. Gateway 完成下载后保存到 `~/.niuma/transfers/inbound/<transfer_id>/`，并把图片转换为 Codex 可读的真实本地图片输入。

### 7.7 媒体和文件接收

1. iOS 收到 `task_update`。
2. 解密后解析 `content_parts`。
3. Gateway 发送到移动端的图片、视频和文件统一使用 `file_ref`，并用 `file_type` 标明宽类型。
4. iOS 先根据 `transfer_id` 查询本地附件缓存，未命中时从 Server 下载完整 payload。
5. iOS 校验 `sha256(data) == transfer_id` 后写入本地缓存。
6. 下载完成后发送 transfer ACK。
7. UI 根据 MIME 类型显示图片、视频预览、文件名、大小和打开入口。

### 7.8 过程消息展示

1. iOS 按 `seq` 保存每条 Codex 投影消息，不在存储层合并过程项。
2. Gateway 不生成展示分类；iOS 只使用 Codex 原生 `type` 和 `phase` 判断展示形态。
3. 过程组标题显示过程消息首尾时间跨度，例如“已处理 12s”或“已处理 1m 20s”；无法计算时间跨度时显示“已处理”，展开后显示每条过程消息。
4. 过程组宽度应与 agent 最终消息保持一致，不能横向占满整个线程区域。
5. `type=userMessage` 是用户消息；`type=agentMessage` 且 `phase=final_answer` 是最终结论；其余连续条目作为过程组展示。

---

## 8. 接口字段表

### 8.1 `POST /pair/confirm`

| 字段 | 方向 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- | --- |
| `device_id` | request | string | 是 | iOS 设备 ID |
| `device_name` | request | string | 是 | iOS 设备显示名 |
| `public_key` | request | string | 是 | iOS 长期 Ed25519 签名公钥；Server 在校验签名后写入设备表 |
| `os_type` | request | string | 否 | 当前固定传 `iOS` |
| `push_token` | request | string | 否 | APNs 设备 token |
| `pair_token` | request | string | 是 | 一次性配对 token |
| `agent_id` | request | string | 是 | 目标桌面 Agent |
| `agent_pairing_public_key` | request | string | 是 | 本次配对公钥，必须与二维码 payload 一致 |
| `encrypted_handshake` | request | string | 是 | 用本次配对公钥加密后的 iOS 长期加密公钥和握手材料 |
| `signature` | request | string | 是 | 设备签名 |
| `binding_id` | response | string | 是 | 配对绑定 ID |
| `agent_ack` | response | object | 是 | Gateway signed ack |
| `expires_at` | response | integer | 否 | 握手材料过期时间 |

### 8.2 Metadata sync

移动端不再通过 server HTTP 全量读取 project/thread/approval 列表。列表数据由桌面 Gateway 通过 WebSocket `project_sync`、`thread_sync`、`approval_sync` 逐条推送，移动端按消息单位写入 SwiftData 并刷新 UI。无项目对话只按 `thread_sync.project_id == "__conversation__"` 展示，不再把未知 project 的 thread 自动归到无项目对话。

### 8.3 WebSocket `task_start`

| 字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `device_id` | string | 是 | iOS 设备 ID |
| `agent_id` | string | 是 | 目标桌面 Agent |
| `project_id` | string | 是 | 项目 ID |
| `thread_id` | string | 否 | 已有 thread ID；新任务不传，由桌面 Codex/app-server 创建 |
| `ciphertext` | string | 是 | 加密后的任务正文 |
| `signature` | string | 是 | 设备签名 |
| `model` | string | 否 | 当前选择的 Codex 模型 |
| `effort` | string | 否 | 当前选择的推理强度 |
| `approval_policy` | string | 否 | 移动端权限模式映射出的 Codex 审批策略 |
| `approvals_reviewer` | string | 否 | 移动端权限模式映射出的审批审查人 |
| `sandbox_mode` | string | 否 | 移动端权限模式映射出的 sandbox 模式 |

说明：

- 移动端不生成 canonical message id，也不创建本地 session 或把本地临时发送态作为持久消息保存。
- `thread_id` 存在时，`project_id` 只参与本次签名/加密上下文，不应被 Gateway 用来重绑 Codex 桌面端的 thread 归属。
- 默认权限模式不发送以上三个权限覆盖字段，保持桌面 Codex 配置为准。

### 8.4 WebSocket `resume_thread`

| 字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `thread_id` | string | 是 | thread ID |
| `cursor` | integer | 是 | 已连续同步的最大消息游标 |
| `checkpoint` | string | 否 | 辅助恢复游标 |

### 8.5 WebSocket `thread_rename_request`

| 字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `request_id` | string | 是 | 移动端生成的请求 ID，用于匹配 result/failed |
| `thread_id` | string | 是 | 要改名的 Codex thread ID |
| `title` | string | 是 | trim 后非空的目标标题 |

说明：

- 移动端只发起请求并进入提交中状态，不直接修改 SwiftData 标题。
- 成功以 `thread_rename_result` 结束提交中状态，并等待随后 `thread_sync` 把 Codex 原数据标题写回本地。
- 失败以 `thread_rename_failed` 结束提交中状态并展示错误。

### 8.7 WebSocket `task_update`

| 字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `thread_id` | string | 是 | thread ID |
| `seq` | integer | 是 | 事件序号 |
| `ciphertext` | string | 是 | 加密后的事件内容 |
| `checkpoint` | string | 否 | 当前输出检查点 |
| `role` | string | 否 | 展示角色 |
| `type` | string | 是 | Codex app-server 原生或 Gateway 合成记录的受控 item type；缺失视为协议错误 |
| `phase` | string | 否 | Codex app-server 原生 item phase |
| `entry_id` | string | 否 | Codex/app-server 投影出的稳定条目 ID |

### 8.8 WebSocket `thread_sync_completed`

| 字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `thread_id` | string | 是 | thread ID |
| `cursor` | integer | 是 | 本次同步完成后的游标 |
| `checkpoint` | string | 否 | 本次同步完成后的检查点 |
| `entry_count` | integer | 否 | 本次同步的消息数量 |

说明：

- 这是 session 详情同步的唯一完成事件；移动端不通过超时或空闲推断完成。

### 8.9 WebSocket `thread_sync_failed`

| 字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `thread_id` | string | 是 | thread ID |
| `cursor` | integer | 是 | 失败请求原 cursor，移动端不得将其视为新的历史进度 |
| `checkpoint` | string | 否 | 失败请求原检查点 |
| `error` | string | 是 | Gateway 读取或投影失败原因 |

说明：

- 这是 session 详情同步失败事件；移动端清除同步中状态并展示错误，不回写更小 cursor。

### 8.10 WebSocket `approval_response`

外层 envelope 明文携带 `approval_id`，用于 server/gateway 在离线或回写失败时返回对应审批的失败回执；审批决策正文仍在 `ciphertext` 内端到端加密。

| 字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `approval_id` | string | 是 | 审批 ID |
| `decision` | string | 是 | `allow` 或 `reject` |
| `grant_scope` | object | 否 | scoped grant 配置 |
| `grant_scope.scope` | string | 否 | `turn` 或 `session` |
| `grant_scope.thread_id` | string | 否 | 生效 thread |
| `grant_scope.approval_type` | string | 否 | 审批类型 |
| `grant_scope.ttl_seconds` | integer | 否 | 授权有效期 |

### 8.11 WebSocket `approval_response_failed`

| 字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `approval_id` | string | 是 | 审批 ID |
| `error` | string | 是 | server 或 gateway 返回的失败原因 |

说明：

- 移动端发送审批响应后先进入提交中状态，只有收到 `approval_sync(status=resolved)` 才视为处理完成。
- 收到 `approval_response_failed` 时维持审批待处理，展示错误并允许用户重新操作。

### 8.12 `content_parts`

`task_start` 和 `task_update` 解密后的业务 payload 应统一使用 `content_parts`，避免移动端只支持纯文本。移动端发送中的临时展示项只存在内存 UI 层，不进入协议、不落库；Codex/app-server 回传 canonical 用户消息后，移动端按同一内容移除本地临时展示项。

| 字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `type` | string | 是 | `text`、`file_ref` |
| `text` | string | 否 | 文本内容 |
| `transfer_id` | string | 否 | 完整 payload SHA-256，作为临时 transfer 引用 |
| `file_type` | string | 否 | `image`、`video` 或 `file` |
| `file_name` | string | 否 | 文件名 |
| `mime_type` | string | 否 | MIME 类型 |
| `size_bytes` | integer | 否 | 文件大小 |
| `alt` | string | 否 | 图片或文件的可读说明 |

说明：

- 移动端发送附件时只使用 `file_ref`，不要把图片作为大 Base64 内嵌在实时消息里。
- `file_ref` 必须有 `transfer_id` 和 `file_type`；能提供文件名、MIME、大小和说明时一并携带。
- `file_name`、`mime_type`、`size_bytes` 和 `alt` 属于业务展示元数据，必须位于业务 payload 中，不能作为 Server 明文字段。
- UI 不根据文件扩展名直接信任类型，必须以 MIME 和安全探测结果共同决定展示方式。

---

## 9. 状态机

### 9.1 设备配对状态机

- `uninitialized`
- `device_registered`
- `unpaired`
- `pairing`
- `paired`
- `pair_failed`

状态流转：

1. `uninitialized -> device_registered`
2. `device_registered -> unpaired`
3. `unpaired -> pairing`
4. `pairing -> paired`
5. `pairing -> pair_failed`
6. `pair_failed -> unpaired`

### 9.2 线程同步状态机

- `idle`
- `submitting`
- `streaming`
- `waiting_approval`
- `reconnecting`
- `replaying`
- `completed`
- `failed`

关键流转：

1. `idle -> submitting`
2. `submitting -> streaming`
3. `streaming -> waiting_approval`
4. `streaming -> reconnecting`
5. `reconnecting -> replaying`
6. `replaying -> streaming`
7. `streaming -> completed`
8. 任意运行态 -> `failed`

### 9.3 WebSocket 连接状态机

- `disconnected`
- `authenticating`
- `connected`
- `degraded`
- `retrying`

---

## 10. 网络与实时通信

### 10.1 鉴权

- 控制面 HTTP 请求通过 challenge / response 换取短时 session token
- WebSocket 建连携带 session token
- `task_start` 仍对路由字段和业务 payload digest 做设备签名

### 10.2 WebSocket 消息类型

- `project_sync`
- `thread_sync`
- `task_update`
- `thread_sync_completed`
- `approval_request`
- `approval_response`
- `metadata_refresh`
- `metadata_refresh_completed`
- `metadata_refresh_failed`
- `model_sync`
- `transfer_ready`

### 10.3 去重策略

- 优先基于 `entry_id`
- `seq` 只作为 Codex 顺序投影下发的排序和 cursor 字段，不作为消息身份
- Transfer 下载完成后再把对应 part 标记为可展示；重复 `transfer_ready` 只触发一次下载

---

## 11. 安全设计

### 11.1 设备密钥

- 存储于 Keychain
- 私钥不可导出或仅在受控场景导出

### 11.2 本地展示保护

- 应用切后台时敏感页面可遮罩
- 审批页可配置二次生物验证

### 11.3 风险控制

- 配对前不信任任意 Agent
- APNs 仅作为提醒与唤醒信号；通知展示文案不包含明文业务内容，点击后解密密文中的
  `thread_id` 并触发现有详情刷新。
- 前台通知展示策略必须由移动端按当前可见 thread 判断；server 不感知页面状态。
- 审批持续授权必须带 thread 范围和 TTL

---

## 12. 子项目目录结构

建议 `/niuma-ios` 目录采用如下结构：

```text
niuma-ios/
  README.md
  niuma.xcodeproj
  niuma/
    App/
    Features/
      Pairing/
      Projects/
      Threads/
    Core/
      Networking/
      Realtime/
      Crypto/
      Storage/
      Push/
      Media/
      Models/
    Resources/
    Tests/
```

目录说明：

- `App/`：入口、依赖注入、导航
- `Features/`：按页面和业务能力拆分
- `Core/`：跨 Feature 基础能力
- `Tests/`：单测与集成测试

---

## 13. MVP 任务拆解

### 13.1 Phase 1：基础工程

- 初始化 Xcode 工程
- 建立 App Shell 与导航骨架
- 接入 Keychain
- 建立基础网络层和 WebSocket 层

### 13.2 Phase 2：设备与配对

- 实现匿名设备初始化
- 实现扫码与配对码输入
- 通过 `POST /pair/confirm` 完成 iOS 设备 upsert、加密握手 relay 和绑定确认
- 实现配对确认与指纹校验

### 13.3 Phase 3：项目与 session

- 实现设备列表
- 实现 workspace 项目列表
- 实现无项目对话 session 列表
- 实现最近 session 缓存

### 13.4 Phase 4：线程运行时

- 实现 `task_start`
- 实现 `task_update` 流式渲染
- 实现 `entry_id` 去重、`seq` 排序与 `cursor` 维护
- 实现 `content_parts` 渲染基础能力
- 实现图片、视频、文件发送入口和 transfer 进度展示

### 13.5 Phase 5：审批与恢复

- 实现审批详情读取
- 实现 allow / reject
- 实现 scoped grant
- 实现 `resume_thread`
- 实现逐条 replay 渲染

### 13.6 Phase 6：推送与收尾

- APNs token 上报
- 推送唤醒和任务进度提醒
- 通知点击后解密业务定位密文并调用 `resume_thread`
- 断网与前后台切换联调
- 关键链路测试
- 媒体和文件收发链路测试

---

## 14. 待确认项

- iCloud 同步范围是否包含设备身份材料
- 本地 thread 缓存保留时长
- 审批页是否强制二次生物验证
- session 列表分页与搜索是否纳入首版
- 图片 base64 直传阈值、单文件最大大小和缓存清理策略
