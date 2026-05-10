#!/bin/bash
set -euo pipefail

# Deploys niuma-server to a configured host and registers it as a systemd
# service. The Linux binary is cross-compiled locally with cargo-zigbuild, so
# the remote host does not need Rust or crate registry access.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
LOCAL_CONFIG="$SCRIPT_DIR/deploy.local.env"

if [ -f "$LOCAL_CONFIG" ]; then
  set -a
  # shellcheck source=/dev/null
  . "$LOCAL_CONFIG"
  set +a
fi

SERVER="${NIUMA_DEPLOY_TARGET:-}"
REMOTE_DIR="${NIUMA_DEPLOY_REMOTE_DIR:-/data/services/niuma-server}"
SERVICE_NAME="${NIUMA_DEPLOY_SERVICE_NAME:-niuma-server}"
SERVICE_FILE="${NIUMA_DEPLOY_SERVICE_FILE:-deploy/niuma-server.service}"
REMOTE_SERVICE_FILE="${NIUMA_DEPLOY_REMOTE_SERVICE_FILE:-/etc/systemd/system/$SERVICE_NAME.service}"
REMOTE_ENV_FILE="${NIUMA_DEPLOY_REMOTE_ENV_FILE:-$REMOTE_DIR/.env}"
TARGET_TRIPLE="${NIUMA_DEPLOY_TARGET_TRIPLE:-x86_64-unknown-linux-gnu}"
TARGET_GLIBC="${NIUMA_DEPLOY_TARGET_GLIBC:-2.34}"
ZIGBUILD_TARGET="$TARGET_TRIPLE.$TARGET_GLIBC"
LOCAL_BINARY="target/$TARGET_TRIPLE/release/$SERVICE_NAME"
REMOTE_BINARY_TMP="${NIUMA_DEPLOY_REMOTE_BINARY_TMP:-/tmp/$SERVICE_NAME}"
REMOTE_SERVICE_TMP="${NIUMA_DEPLOY_REMOTE_SERVICE_TMP:-/tmp/$SERVICE_NAME.service}"

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

log()  { echo -e "${GREEN}[deploy]${NC} $1"; }
warn() { echo -e "${YELLOW}[warn]${NC}  $1"; }
fail() { echo -e "${RED}[error]${NC} $1"; exit 1; }

cd "$PROJECT_DIR"

[ -n "$SERVER" ] || fail "Missing NIUMA_DEPLOY_TARGET; set it to user@host or create deploy/deploy.local.env"
[ -f "$SERVICE_FILE" ] || fail "Missing $SERVICE_FILE"
command -v cargo >/dev/null 2>&1 || fail "cargo is required"
command -v cargo-zigbuild >/dev/null 2>&1 || fail "cargo-zigbuild is required; install it with 'brew install cargo-zigbuild'"
command -v zig >/dev/null 2>&1 || fail "zig is required; install it with 'brew install zig'"
command -v ssh >/dev/null 2>&1 || fail "ssh is required"
command -v scp >/dev/null 2>&1 || fail "scp is required"

if ! rustup target list --installed | grep -qx "$TARGET_TRIPLE"; then
  fail "Rust target $TARGET_TRIPLE is not installed; run 'rustup target add $TARGET_TRIPLE'"
fi

log "Cross-compiling $SERVICE_NAME for $ZIGBUILD_TARGET ..."
cargo zigbuild --release --target "$ZIGBUILD_TARGET"
[ -x "$LOCAL_BINARY" ] || fail "Missing cross-compiled binary at $LOCAL_BINARY"

log "Preparing remote directory $SERVER:$REMOTE_DIR ..."
ssh "$SERVER" "mkdir -p '$REMOTE_DIR' '$REMOTE_DIR/transfers'"

log "Uploading binary..."
scp "$LOCAL_BINARY" "$SERVER:$REMOTE_BINARY_TMP"

log "Uploading systemd unit..."
scp "$SERVICE_FILE" "$SERVER:$REMOTE_SERVICE_TMP"

if ssh "$SERVER" "test -f '$REMOTE_ENV_FILE'"; then
  log "Remote env exists at $REMOTE_ENV_FILE; keeping existing server configuration."
else
  [ -f .env ] || fail "Remote env is missing and local .env was not found; create one before first deploy"
  log "Remote env missing; uploading local .env to $REMOTE_ENV_FILE ..."
  scp .env "$SERVER:$REMOTE_ENV_FILE"
fi

log "Installing binary and systemd service..."
ssh "$SERVER" "install -m 755 '$REMOTE_BINARY_TMP' '$REMOTE_DIR/$SERVICE_NAME' && install -m 644 '$REMOTE_SERVICE_TMP' '$REMOTE_SERVICE_FILE' && rm '$REMOTE_BINARY_TMP' '$REMOTE_SERVICE_TMP' && systemctl daemon-reload && systemctl enable '$SERVICE_NAME' && systemctl restart '$SERVICE_NAME'"

sleep 2
STATUS=$(ssh "$SERVER" "systemctl is-active '$SERVICE_NAME'")
if [ "$STATUS" != "active" ]; then
  warn "Service status: $STATUS"
  ssh "$SERVER" "journalctl -u '$SERVICE_NAME' -n 40 --no-pager"
  fail "Service did not start correctly."
fi

if ssh "$SERVER" "command -v curl >/dev/null 2>&1 && curl -fsS http://127.0.0.1:8000/healthz >/dev/null"; then
  log "Done! Service is active and /healthz passed."
else
  warn "Service is active, but /healthz could not be verified with remote curl."
fi
