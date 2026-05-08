# Rust Server Replaces FastAPI

`niuma-server/` is now a standalone Rust crate rather than a Python/FastAPI project. The crate keeps the existing HTTP and WebSocket wire protocol, continues to use PostgreSQL for control-plane state, stores transfer payloads on the server filesystem, and remains payload-blind for Codex business content.

**Status:** accepted

**Consequences:** Python source, Python tests, virtualenv setup, audit service, and Python project configuration are removed from `niuma-server/`. The Rust implementation uses `axum`, `tokio`, `sqlx`, `serde`, and `tracing`; startup creates the current control-plane schema. Database URLs use standard `postgresql://` or `postgres://`.
