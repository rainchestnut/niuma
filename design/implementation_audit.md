# Niuma Current Implementation Audit

## Scope

This audit records the current architecture after the transfer cleanup pass and
the Rust desktop gateway decision. It is the practical reference for keeping
the implementation and design documents aligned.

## Current Boundary

Niuma remains a mobile channel for the desktop Codex app, not a separate chat
system.

- Codex desktop local state is the source of truth for workspace roots and projectless thread ids; Codex app-server is the source of truth for sessions, threads, and message bodies.
- The desktop Rust gateway is the only layer that adapts Codex app-server records into the Niuma mobile protocol.
- The server is payload-blind. It persists only devices, pairings, online state, and temporary content-addressed transfer payloads; project/session/thread/approval metadata is forwarded over WebSocket and stored by iOS SwiftData.
- The iOS app persists mobile display state in SwiftData, one message entry per row, and uses the desktop replay cursor to request more history.

Project projection is intentionally narrow: workspace projects come from
`~/.codex/.codex-global-state.json`, projectless conversations come from
`projectless-thread-ids`, and thread metadata is read with scoped
`thread/list(cwd=<workspace_root>, archived=...)`. The gateway must not recreate
the removed cwd/global-scan/project-derivation branches.

## Current Session Detail Protocol

The session-detail sync path is intentionally narrow:

1. iOS sends `resume_thread` with `thread_id`, current integer `cursor`, and optional `checkpoint`.
2. Server validates the mobile-agent pairing and forwards the envelope to the connected desktop gateway.
3. Gateway reads Codex app-server history through `thread/read(includeTurns=true)`.
4. Gateway sends one `task_update` per mobile-visible message record.
5. Gateway sends one `thread_sync_completed` as the authoritative end-of-batch marker.
6. iOS persists each entry and advances its SwiftData cursor only from received records and the completion marker.

The mobile app must not infer completion from WebSocket quiet time, metadata refresh completion, or an idle timeout.
Ordering must come from Codex/app-server thread order. The gateway may project that order into the mobile protocol, but it must not mint an independent sequence that can diverge from `thread/read` replay.
Folding must remain a mobile UI concern. The gateway/server protocol carries Codex item `type` and `phase` only; it must not introduce Niuma-specific message display categories.

## Current Task Start Protocol

Mobile-originated task start is also narrowed:

- iOS does not generate canonical message ids.
- iOS sends route fields, selected model, and signed payload ciphertext. New tasks omit `thread_id`; existing-thread sends include the real Codex thread id.
- Desktop Codex/app-server generates the durable thread/turn/item identifiers for new tasks.
- Existing-thread sends may show a local-only transient row in memory. That row is never sent as protocol identity, never persisted, and is removed when the desktop-confirmed user entry for the same content arrives.
- Gateway sends the original mobile `content_parts` envelope back on the initial desktop-confirmed user `task_update`. Later replay comes from Codex `thread/read`; file refs that only carry `transfer_id` are materialized by the Rust transfer store before entering Codex input.
- iOS does not persist client-generated sessions or message ids.

`idempotency_key`, `message_id`, `session_title`, and client-generated message identity are not part of the current wire contract.

## Media Boundary

Media and files must not expand the realtime control frame.

- iOS sends images, videos, and files as `file_ref` content parts with a `file_type` and content-hash `transfer_id`.
- Server sees only transfer routing metadata, payload size, `transfer_id`, and TTL. `transfer_id` is the SHA-256 of the complete transfer payload, so there is no separate content-check field or chunk state.
- Gateway stores inbound payloads under `~/.niuma/transfers` before materializing them as real Codex-readable file inputs. Codex-originated inline raster images are rewritten into `agent_to_ios` server transfers before `task_update` leaves the gateway.
- Codex-originated inline raster image data must not be replayed as a large `task_update` frame. The gateway converts stable `item/completed` or `turn/completed` images into `agent_to_ios` transfers only after MIME and file-signature validation, then sends only transfer-backed `file_ref` metadata in `task_update`.

## Remaining Architecture Risks

- The realtime message contracts are duplicated across Swift, server Pydantic models, and gateway Rust structs.
- `AppModel` still owns too many responsibilities: pairing, authentication, websocket lifecycle, timeline merge, transfer download, metadata refresh, and settings.
- The gateway runtime must keep orchestration, replay failure handling, and app-server restart policy in separate modules as the Rust port matures.
- Pairing now owns key exchange: server remains payload-blind, relays encrypted handshake to an online gateway, and creates bindings only after signed gateway ack.
- A small transfer contract test set exists again, but broader protocol coverage should still focus on projection boundaries instead of broad unit coverage.

## Required Direction

1. Keep the server payload-blind and prevent it from learning message, media, filename, MIME, or display metadata semantics.
2. Keep `resume_thread -> task_update* -> thread_sync_completed` as the only session-detail sync protocol.
3. Move all Codex history and media adaptation into the gateway adapter/projection boundary.
4. Move iOS timeline/cursor mutation into SwiftData-facing helpers so `AppModel` becomes an observable facade rather than the owner of sync invariants.
5. Expand tests around the settled boundaries: protocol projection tests, SwiftData timeline merge tests, and one installed-gateway visible E2E path.
