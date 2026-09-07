#!/usr/bin/env bash
# Live E2E smoke: real crys web server + real luoshu tunnel client.
# Usage: bash scripts/e2e_tunnel_smoke.sh  (from the luoshu repo root)
set -euo pipefail

CRYS=${CRYS:-/root/workspace/crys}
E2E=/tmp/luoshu-e2e
# Pick a definitely-free port (the server also auto-shifts when occupied,
# which would silently desync the checks below).
PORT=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')
export PATH="$HOME/.local/bin:$HOME/.cargo/bin:$PATH"

rm -rf "$E2E" && mkdir -p "$E2E"
export CRAN_DATABASE_URL="sqlite+aiosqlite:///$E2E/cran.db"
export CRAN_JWT_SECRET="e2e-secret"

cleanup() { [ -n "${SRV:-}" ] && kill "$SRV" 2>/dev/null || true; [ -n "${TUN:-}" ] && kill "$TUN" 2>/dev/null || true; }
trap cleanup EXIT

echo "== prebuilding tunnel client =="
cargo build -q -p luoshu-bridge --example tunnel_standalone
TUNNEL_BIN=$(find target/debug/examples -name tunnel_standalone -type f | head -1)

echo "== starting crys web on 127.0.0.1:$PORT =="
(cd "$CRYS" && uv run cran-code web --port "$PORT" --host 127.0.0.1 >"$E2E/server.log" 2>&1) &
SRV=$!
for i in $(seq 1 60); do
  curl -sf "http://127.0.0.1:$PORT/api/v2/health" >/dev/null 2>&1 && break
  curl -sf "http://127.0.0.1:$PORT/" >/dev/null 2>&1 && break
  sleep 1
done

echo "== register user + create device =="
TOKEN=$(curl -sf -X POST "http://127.0.0.1:$PORT/api/v2/auth/register" \
  -H 'Content-Type: application/json' \
  -d '{"email":"e2e@example.com","username":"e2e","password":"e2e-pass-12345"}' \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["access_token"])')
DEVICE=$(curl -sf -X POST "http://127.0.0.1:$PORT/api/v2/devices" \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"name":"e2e-rig"}')
echo "$DEVICE"
DEV_ID=$(echo "$DEVICE" | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')
DEV_TOKEN=$(echo "$DEVICE" | python3 -c 'import json,sys; print(json.load(sys.stdin)["token"])')

cat > "$E2E/device.json" <<EOF
{"cloud_url": "http://127.0.0.1:$PORT", "device_id": "$DEV_ID", "device_token": "$DEV_TOKEN", "name": "e2e-rig"}
EOF
chmod 600 "$E2E/device.json"

echo "== starting luoshu tunnel client =="
LUOSHU_HOME="$E2E" "$TUNNEL_BIN" >"$E2E/tunnel.log" 2>&1 &
TUN=$!

ONLINE=""
for i in $(seq 1 20); do
  ONLINE=$(curl -sf "http://127.0.0.1:$PORT/api/v2/devices" -H "Authorization: Bearer $TOKEN" \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)[0]["online"])' 2>/dev/null || true)
  [ "$ONLINE" = "True" ] && break
  sleep 1
done

echo "== device list (expect online: true) =="
curl -sf "http://127.0.0.1:$PORT/api/v2/devices" -H "Authorization: Bearer $TOKEN" | python3 -m json.tool
[ "$ONLINE" = "True" ] || { echo "DEVICE NEVER CAME ONLINE"; exit 1; }

echo "== revoke device (tunnel should get 4401 and stop) =="
curl -sf -X DELETE "http://127.0.0.1:$PORT/api/v2/devices/$DEV_ID" -H "Authorization: Bearer $TOKEN"
echo
sleep 3
STOPPED=""
for i in $(seq 1 10); do
  kill -0 "$TUN" 2>/dev/null || { STOPPED=1; break; }
  sleep 1
done
if [ -z "$STOPPED" ]; then echo "TUNNEL STILL RUNNING (unexpected)"; exit 1; else echo "tunnel stopped after revoke: OK"; fi
echo "E2E SMOKE OK"
