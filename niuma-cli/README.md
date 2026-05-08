# Niuma CLI

`niuma-cli` 是 Niuma 的 Rust 桌面 Gateway。crate 目录名是
`niuma-cli/`，安装后的二进制命令名是 `niuma`。

它负责桌面身份、二维码配对页面、Niuma Server 连接、Codex app-server
子进程、移动端任务转发、审批/输入请求转发，以及文件 transfer 本地物化。

## 安装

当前只支持从本仓库本地安装：

```bash
cargo install --path niuma-cli
```

安装后确认命令可用：

```bash
niuma --help
niuma --version
```

## 命令格式规则

`niuma` 使用 `clap` 解析命令，规则如下：

- 顶层命令必须带子命令：`niuma <COMMAND>`。
- 长参数使用两个 ASCII 短横线：`--help`、`--server-url`。
- 短参数使用一个 ASCII 短横线：`-h`、`-V`。
- 不支持长破折号或混合破折号，例如 `niuma -—help` 不是合法写法。
- 子命令的 help 要写在对应层级后面，例如 `niuma gateway --help`。
- 顶层 `--help` 只显示命令列表；具体参数以子命令 help 为准。

顶层命令：

```bash
niuma gateway [OPTIONS]
niuma service <COMMAND>
niuma status [OPTIONS]
niuma reset --yes
```

## 配置来源

`niuma gateway` 的配置优先级是：

```text
命令行参数 > 环境变量 > ~/.niuma/config.toml > 内置默认值
```

`niuma status` 只读取 dashboard host/port，也遵循同样的参数、环境变量、
配置文件、默认值顺序。

支持的配置文件字段：

```toml
server_url = "http://127.0.0.1:8000"
device_name = "My Mac"
dashboard_host = "127.0.0.1"
dashboard_port = 8765
heartbeat_seconds = 30
```

支持的环境变量：

```bash
NIUMA_SERVER_URL=http://127.0.0.1:8000
NIUMA_DEVICE_NAME="My Mac"
NIUMA_DASHBOARD_HOST=127.0.0.1
NIUMA_DASHBOARD_PORT=8765
NIUMA_HEARTBEAT_SECONDS=30
```

默认值：

- `server_url`: `http://127.0.0.1:8000`
- `dashboard_host`: `127.0.0.1`
- `dashboard_port`: `8765`
- `heartbeat_seconds`: `30`
- `device_name`: 优先取 `HOSTNAME`，其次取 macOS `scutil --get ComputerName`，
  最后回退为 `Niuma Desktop`

## `niuma gateway`

前台启动完整桌面 Gateway runtime：

```bash
niuma gateway
```

启动后会：

- 创建或读取 `~/.niuma/identity` 下的桌面身份。
- 连接 Niuma Server 并注册/auth desktop agent。
- 启动本地 loopback dashboard，默认地址是 `http://127.0.0.1:8765`。
- 默认打开本地配对页面。
- 维护 `/api/pairing/payload` 当前二维码 payload。
- 连接 Codex app-server，处理移动端任务、历史同步、审批、输入请求和文件 transfer。

常用参数：

```bash
niuma gateway --server-url https://example.invalid/niuma-server
niuma gateway --dashboard-host 127.0.0.1 --dashboard-port 8765
niuma gateway --device-name "My Mac"
niuma gateway --no-open
niuma gateway --pairing-page-only
niuma gateway --disable-codex-plugins
```

参数语义：

- `--server-url <URL>`: Niuma Server 地址。
- `--dashboard-host <HOST>`: 本地 dashboard 监听 host。
- `--dashboard-port <PORT>`: 本地 dashboard 监听端口。
- `--device-name <NAME>`: 桌面 agent 展示名。
- `--no-open`: 启动后不自动打开浏览器。
- `--pairing-page-only`: 只启动本地配对/诊断页面，不建立完整 Server WebSocket
  和 Codex runtime。
- `--disable-codex-plugins`: 启动 Codex app-server 时追加 `--disable plugins`。

`gateway` 默认是完整 runtime。如果只想排查二维码页面或本地 HTTP 控制面，
才使用 `--pairing-page-only`。

## `niuma service`

`niuma service` 只管理 macOS LaunchAgent。当前不抽象 Linux systemd 或
Windows Service。

安装后台服务：

```bash
niuma service install
```

安装并立即启动：

```bash
niuma service install --start
```

安装静默后台服务：

```bash
niuma service install --no-open
```

`--no-open` 是安装期参数，会写入 LaunchAgent plist。后续 `niuma service start`
只按已安装 plist 启动，不临时改变是否打开浏览器。

服务生命周期命令：

```bash
niuma service start
niuma service stop
niuma service restart
niuma service status
niuma service uninstall
```

行为规则：

- `install` 写入 `~/Library/LaunchAgents/com.niuma.gateway.plist`。
- plist 内写入当前 `niuma` 二进制的绝对路径，不依赖后台 `PATH`。
- `install` 默认不启动；需要 `install --start` 或单独 `service start`。
- `start` 前会检查 gateway 端口是否已被占用。
- 如果端口已被前台 gateway 或其他进程占用，`start` 会失败并提示占用信息。
- `restart` 等价于 `stop` 后再 `start`。
- `uninstall` 会停止并删除 LaunchAgent plist。
- `status` 同时输出 launchd 状态和 gateway `/api/status` 结果或错误。

后台日志路径：

```text
~/.niuma/logs/gateway.out.log
~/.niuma/logs/gateway.err.log
```

## `niuma status`

读取当前 gateway 的本地 HTTP 状态接口：

```bash
niuma status
niuma status --dashboard-host 127.0.0.1 --dashboard-port 8765
```

该命令只查询：

```text
http://<dashboard-host>:<dashboard-port>/api/status
```

它适合诊断前台 `niuma gateway` 或后台 `niuma service` 管理的同一个
gateway runtime。它不读取 launchd 状态；需要 launchd 信息时使用
`niuma service status`。

## `niuma reset`

`reset` 是破坏性测试命令，必须显式确认：

```bash
niuma reset --yes
```

它会先卸载/停止 LaunchAgent，然后删除本机 Niuma 状态目录：

```text
~/.niuma
```

没有 `--yes` 时会拒绝执行。

## 本地状态目录

Runtime 状态固定存储在 `~/.niuma`：

```text
~/.niuma/
  config.toml
  identity/
  logs/
  runtime/
  transfers/
```

`niuma-cli` 不迁移旧 Python bridge 的 `.niuma-state`，也不读取旧插件状态。

## 常见流程

前台开发：

```bash
cargo install --path niuma-cli
niuma gateway
niuma status
```

后台运行：

```bash
cargo install --path niuma-cli
niuma service install --start
niuma service status
niuma service restart
```

只查看配对页诊断：

```bash
niuma gateway --pairing-page-only
```

清理本机状态重新配对：

```bash
niuma reset --yes
niuma gateway
```

## 验证

```bash
cd niuma-cli
cargo fmt --check
cargo check
cargo test
```
