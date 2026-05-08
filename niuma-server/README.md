# Niuma Server

Niuma Server 是移动端和桌面 Gateway 之间的 payload-blind 控制面服务。当前实现是 Rust crate，负责设备注册、challenge/session 鉴权、配对、WebSocket 路由和 content-addressed 临时 transfer 中转。

Server 不保存 Codex 会话明文历史，不解析 `ciphertext`，不持久化 project/session/thread/approval 明文索引，也不把 transfer payload 写入 PostgreSQL。

## 目录

```text
niuma-server/
  Cargo.toml
  Cargo.lock
  src/
    main.rs       # 启动、PostgreSQL 连接、schema 初始化、清理任务
    routes.rs     # HTTP / WebSocket 协议入口
    db.rs         # PostgreSQL 控制面状态
    hub.rs        # 单设备单活 WebSocket hub
    transfer.rs   # 本地文件系统临时 transfer relay
    apns.rs       # APNs 任务进度推送
    crypto.rs     # 既有 digest / Ed25519 验签口径
    config.rs     # .env + NIUMA_ 配置
```

## 配置

本地启动读取 `niuma-server/.env` 中的 `NIUMA_*` 配置项；当 `.env` 存在时，它优先于启动命令里的同名环境变量。部署环境如果没有本地 `.env`，仍可通过进程环境变量提供同名配置。

| 变量 | 说明 |
|------|------|
| `NIUMA_APP_ENV` | 运行环境标记，默认 `development`。 |
| `NIUMA_LOG_LEVEL` | 日志级别，默认 `info`。 |
| `NIUMA_HOST` | 监听地址，默认 `127.0.0.1`。 |
| `NIUMA_PORT` | 监听端口，默认 `8000`。 |
| `NIUMA_DATABASE_URL` | 必填 PostgreSQL 连接串，必须使用 `postgresql://` 或 `postgres://`。 |
| `NIUMA_DATABASE_POOL_SIZE` | PostgreSQL 连接池大小，默认 `5`。 |
| `NIUMA_DATABASE_CONNECT_TIMEOUT_SECONDS` | PostgreSQL 连接超时秒数，默认 `10`。 |
| `NIUMA_CHALLENGE_TTL_SECONDS` | challenge TTL，默认 `120` 秒。 |
| `NIUMA_PAIR_TOKEN_TTL_SECONDS` | pair token TTL，默认 `300` 秒。 |
| `NIUMA_SESSION_TOKEN_TTL_SECONDS` | session token TTL，默认 `3600` 秒。 |
| `NIUMA_PAIR_TOKEN_MAX_ATTEMPTS` | pair token 失败次数上限，默认 `5`。 |
| `NIUMA_TRANSFER_STORAGE_DIR` | 临时 transfer 目录；留空则使用 `niuma-server/.tmp/transfers`。 |
| `NIUMA_TRANSFER_TTL_SECONDS` | 临时 transfer TTL，默认 `1800` 秒。 |
| `NIUMA_TRANSFER_MAX_ENCRYPTED_BYTES` | 单个 transfer payload 最大大小，默认 `100 MiB`。 |
| `NIUMA_APNS_KEY_ID` | APNs Auth Key ID；留空时推送服务禁用。 |
| `NIUMA_APNS_TEAM_ID` | Apple Developer Team ID。 |
| `NIUMA_APNS_TOPIC` | APNs topic，通常是 iOS bundle id：`com.rainchestnut.niuma`。 |
| `NIUMA_APNS_AUTH_KEY_PATH` | APNs `.p8` 私钥路径；与 `NIUMA_APNS_AUTH_KEY_PEM` 二选一。 |
| `NIUMA_APNS_AUTH_KEY_PEM` | APNs `.p8` 私钥内容；与 `NIUMA_APNS_AUTH_KEY_PATH` 二选一。 |
| `NIUMA_APNS_ENVIRONMENT` | `sandbox` 或 `production`，默认 `sandbox`。 |

数据库连接串示例：

```dotenv
NIUMA_DATABASE_URL=postgresql://localhost:5432/niuma
```

启动时会自动创建当前阶段所需表；缺少 `NIUMA_DATABASE_URL` 或使用非 PostgreSQL 连接串时启动失败。

## 运行

```bash
cd /Users/rainchestnut/research/openai/niuma/niuma-server
cargo run
```

可用接口：

- `GET /healthz`
- `POST /devices/register`（桌面 Gateway 注册自身；iOS 设备随 `/pair/confirm` upsert）
- `POST /devices/push-token`
- `POST /auth/challenge`
- `POST /auth/verify`
- `POST /pair/request`
- `POST /pair/confirm`
- `POST /pair/revoke`
- `POST /transfers/{transfer_id}/ensure`
- `PUT /transfers/{transfer_id}`
- `GET /transfers/{transfer_id}`
- `POST /transfers/{transfer_id}/ack`
- `WS /ws/mobile`
- `WS /ws/agent`

## 配对约束

`POST /pair/request` 需要桌面 Gateway 的 `X-Session-Token`，请求体必须同时提交 `agent_id` 和本次二维码的一次性 `agent_pairing_public_key`。Server 将 pair token 与该公钥绑定。

`POST /pair/confirm` 先校验 iOS 签名、pair token、`agent_pairing_public_key`，再把 `encrypted_handshake` 转发给在线 Gateway。只有 Gateway 解密成功并返回 signed ack 后，Server 才创建 `active` binding 并消耗 pair token。ack 失败、超时或签名不匹配都不会创建 binding。

## WebSocket 约束

- `/ws/mobile` 需要 `device_id`、`agent_id`、`session_token` 查询参数，并要求 iOS 与 Gateway 已配对。
- `/ws/agent` 需要 `agent_id`、`session_token` 查询参数。
- 同一个 `device_id` 或 `agent_id` 只保留一个活跃连接；新连接替换旧连接并主动关闭旧连接。
- Server 只按 `kind` 做路由和最小鉴权，不缓存消息历史，不做离线队列。
- `task_start` 入站会按既有 digest 口径验证移动端 Ed25519 签名后再转发。
- `task_progress_push` 只接受已配对 Gateway 发来的端到端加密定位密文；Server 查找
  iOS `push_token` 后调用 APNs，通知展示固定通用文案。

## Transfer Relay Contract

Transfer API 只做短时中转，不理解图片、视频或文件业务语义。

- `transfer_id` 必须是完整 payload 的 SHA-256。
- `POST /transfers/{transfer_id}/ensure` 只创建或刷新 manifest；如果相同 `transfer_id` 的完整 payload 已存在且大小一致，返回 `needs_upload=false`。
- `PUT /transfers/{transfer_id}` 一次性上传完整 payload；Server 校验 `sha256(body) == transfer_id`。
- `GET /transfers/{transfer_id}` 只允许目标设备下载完整 payload。
- `POST /transfers/{transfer_id}/ack` 表示目标端已有本地副本，Server 刷新 TTL；过期和启动清理负责删除临时文件。

Server 明文字段只包含路由设备、方向、payload 大小、`transfer_id` 和 TTL。文件名、MIME、预览说明、`file_ref`、`file_type` 都属于业务 payload，由 iOS 和 Rust Gateway 处理。

## 部署

`deploy/deploy.sh` 参考同级 Java 项目的部署方式，但不会依赖服务器拉取 Rust crates。脚本会在本机用 `cargo-zigbuild` 交叉编译 `x86_64-unknown-linux-gnu.2.34` 产物，再上传 Linux binary 并注册 `niuma-server.service`。部署目标不写入仓库，通过环境变量或未跟踪的本地配置提供。

本机需要具备：

- Rust target：`rustup target add x86_64-unknown-linux-gnu`。
- Zig：`brew install zig`。
- cargo-zigbuild：`brew install cargo-zigbuild`。

服务器需要具备：

- `NIUMA_DEPLOY_TARGET` 指向的 SSH 目标需要有远端目录和 systemd service 安装权限。
- systemd。
- 本地 `niuma-server/.env`。部署脚本会同步到 `/data/services/niuma-server/.env`，
  至少需要包含 `NIUMA_DATABASE_URL`。

部署命令：

```bash
cd /Users/rainchestnut/research/openai/niuma/niuma-server
NIUMA_DEPLOY_TARGET=user@host ./deploy/deploy.sh
```

也可以创建未跟踪的 `deploy/deploy.local.env`：

```dotenv
NIUMA_DEPLOY_TARGET=user@host
# NIUMA_DEPLOY_REMOTE_DIR=/data/services/niuma-server
```

APNs `.p8` 私钥不随每次部署自动上传。首次配置或轮换密钥时，手动把私钥放到
服务器的服务目录外或服务目录 `keys/` 子目录，并限制权限：

```bash
ssh user@host "mkdir -p /data/services/niuma-server/keys"
scp AuthKey_XXXX.p8 user@host:/tmp/AuthKey_XXXX.p8
ssh user@host "install -m 600 /tmp/AuthKey_XXXX.p8 /data/services/niuma-server/keys/AuthKey_XXXX.p8 && rm /tmp/AuthKey_XXXX.p8"
```

远端 `/data/services/niuma-server/.env` 中的 `NIUMA_APNS_AUTH_KEY_PATH` 应填写服务器上的实际路径，例如：

```dotenv
NIUMA_APNS_AUTH_KEY_PATH=/data/services/niuma-server/keys/AuthKey_XXXX.p8
```

服务注册后可在服务器上用以下命令查看：

```bash
systemctl status niuma-server
journalctl -u niuma-server -n 100 --no-pager
```

## 验证

```bash
cargo fmt --check
cargo check
cargo test
```
