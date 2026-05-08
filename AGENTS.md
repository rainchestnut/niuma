# Niuma Repo Notes

## Workspace Shape
- This root is a container for the iOS app, Rust desktop gateway, Rust server, and design docs. It is not a buildable monorepo. There is no root manifest, lockfile, CI workflow, or shared task runner checked in.
- `design/*.md` holds the intended architecture. Use it for product intent, but trust code and project config over prose when they differ.
- The root directory is a git repo for this checkout, but it is still only a container workspace. Do not assume root-level build or test commands exist.

## Project Boundaries
- `niuma-server/`: Rust control-plane server crate. App entrypoint is `src/main.rs`; HTTP and WebSocket routes are composed in `src/routes.rs`.
- `niuma-cli/`: Rust desktop gateway installed as the `niuma` command. `niuma gateway` runs the foreground bridge and pairing page; `niuma service` manages the macOS LaunchAgent.
- `niuma-ios/`: Xcode iOS app. App entrypoint is `niuma/App/NiumaApp.swift`; dependency wiring is in `niuma/App/AppContainer.swift`.

## Verified Commands
- Server setup: `cd niuma-server && python3 -m venv .venv && source .venv/bin/activate && pip install -e .`
- Server dev server: `cd niuma-server && cargo run`
- iOS schemes: `cd niuma-ios && xcodebuild -list -project "niuma.xcodeproj"`
- iOS build: `cd niuma-ios && xcodebuild -scheme "niuma" -project "niuma.xcodeproj" -destination "platform=iOS Simulator,name=iPhone 17" build`

## Current Verification Reality
- Test suites and standalone verification scripts were intentionally removed for the current restructuring pass. Recreate tests after the code structure settles.

## Server-Specific Gotchas
- Settings are loaded by `src/config.rs` from built-in defaults plus `niuma-server/.env`; local `.env` wins over shell env when both exist. Deployment environments without `.env` can still provide `NIUMA_` env vars.
- Runtime server startup requires a standard `postgresql://` or `postgres://` `NIUMA_DATABASE_URL` and initializes the PostgreSQL schema.
- Server 不再暴露 `/projects`、`/sessions`、`/threads`、`/approvals` 明文 metadata 读接口；列表数据由 Rust Gateway 通过 WebSocket 增量推送，移动端落 SwiftData。
- WebSocket auth is query-string based: `/ws/mobile?...&session_token=...` and `/ws/agent?...&session_token=...`.
- There is no `/dev/mock-agent` route in the current server source. Development pairing now goes through the Rust Gateway dashboard / pairing payload endpoint.
- The server is intentionally payload-blind control-plane code. Do not add assumptions about persisted task payloads.
- Session detail sync is `resume_thread -> task_update* -> thread_sync_completed`. iOS must not infer completion from metadata refreshes, WebSocket quiet time, or local timers.
- iOS 走真实 server 时，`X-Session-Token` 由 `LiveNiumaController.updateSessionToken` 注入；WS `task_start` 的 `signature` 用 `DeviceIdentityService.makeTaskStartSignature` 计算，与 server `build_task_start_digest` 同口径。新任务可不带 `session_id`，由桌面 Codex/app-server 创建真实 thread id 后同步回移动端；已有会话发送仍带 `session_id`。`task_update` 不发 `event_type`，移动端不生成 canonical message id。

## Gateway-Specific Gotchas
- The Rust gateway owns `/ws/agent`, the loopback pairing dashboard, Codex app-server JSONL process management, metadata projection, task forwarding, approvals, request-user-input, and transfer materialization.
- Codex app-server is the only history source. Do not reintroduce gateway-side message-history caches for replay.
- Runtime state is persisted under `~/.niuma`; do not restore `.niuma-state` or removed Python bridge state.
- Useful env overrides: `NIUMA_SERVER_URL`, `NIUMA_DEVICE_NAME`, `NIUMA_DASHBOARD_HOST`, `NIUMA_DASHBOARD_PORT`, `NIUMA_HEARTBEAT_SECONDS`.

## iOS-Specific Gotchas
- The app always uses `LiveNiumaController`; the Niuma Server URL is a persisted mobile setting exposed in pairing and Settings. There is no iOS `NIUMA_SERVER_URL` runtime override; a fresh install has no default server and requires manual entry before pairing.
- The iOS project is a plain Xcode project (`niuma.xcodeproj`) with one app target (`niuma`); there is no SwiftPM manifest.
- `LiveNiumaController` mirrors the current server skeleton exactly. If you change server contracts, update the matching request or response models in `niuma-ios/niuma/Core/Models/APIModels.swift` and transport code in `niuma-ios/niuma/Core/Networking/LiveNiumaController.swift` together.
- 配对入口是 **二维码扫描**：`Features/Pairing/PairingView.swift` 用 `QRScannerView`（AVFoundation）解码 Rust Gateway 打印的完整 JSON payload，包含桌面长期签名/加密公钥、本次配对公钥、指纹、签名和 `pair_token`。移动端通过 `AppModel.pairWithScannedPayload` 走 `/pair/confirm`，并在该请求内提交 iOS 设备名和长期签名公钥。相机权限说明通过 `INFOPLIST_KEY_NSCameraUsageDescription`（pbxproj Debug+Release 双份）注入，没有物理 Info.plist。
- `#if DEBUG` 下 `PairingView` 暴露“模拟扫码当前桌面二维码”按钮，调用 `AppModel.pairWithDesktopGateway()` → `LiveNiumaController.fetchDesktopPairingPayload()`，读取本地 Rust Gateway dashboard 的当前配对 JSON，再走真实 `/pair/confirm`。

## Missing Tooling
- No root lint, formatter, or typecheck config is checked in. Use project-local Rust/iOS commands, and do not invent root-level task runners unless you add and verify them first.
