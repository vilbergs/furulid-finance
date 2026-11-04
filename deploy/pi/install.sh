#!/bin/sh
# Builds jevcat on the Pi and schedules the weekly sync with a systemd user timer. The
# service runs the Swamp workflow `sync` (workflows/workflow-sync.yaml) and falls back to
# plain `jevcat sync` when swamp cannot run.
# Run from this repo: deploy/pi/install.sh [user@host]
# The host defaults to JEVCAT_SYNC_HOST from the gitignored .env, so no hostname lives here.
# Copies the repo (with the gitignored jevcat.toml and .env, without the local .swamp/ runtime
# data) to ~/sites/furulid-finance on the Pi. Swamp needs an account: put an API key from
# swamp-club.com in ~/.config/swamp/api-key on the Pi (chmod 600); the script fills the Pi's
# vault from .env once that file exists.
set -eu
HERE="$(cd "$(dirname "$0")/../.." && pwd)"
[ -f "$HERE/.env" ] && JEVCAT_SYNC_HOST="$(sed -n 's/^JEVCAT_SYNC_HOST=//p' "$HERE/.env" | tr -d '"')"
HOST="${1:-${JEVCAT_SYNC_HOST:-}}"
[ -n "$HOST" ] || { echo "usage: $0 user@host   (or set JEVCAT_SYNC_HOST in .env)" >&2; exit 2; }

echo "== copying repo to $HOST:~/sites/furulid-finance"
rsync -az --delete --exclude target --exclude .git --exclude .swamp --exclude .claude/settings.local.json "$HERE/" "$HOST:sites/furulid-finance/"

ssh "$HOST" 'sh -s' <<'REMOTE'
set -eu
cd ~/sites/furulid-finance
if ! command -v cargo >/dev/null 2>&1 && [ ! -x "$HOME/.cargo/bin/cargo" ]; then
  echo "== installing rust"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
fi
. "$HOME/.cargo/env"
command -v cc >/dev/null 2>&1 || { echo "== installing build tools"; sudo apt-get update -q && sudo apt-get install -y -q build-essential; }
echo "== building jevcat (a few minutes the first time)"
cargo install --path . --locked 2>&1 | tail -2
if [ ! -x "$HOME/.local/bin/swamp" ]; then
  echo "== installing swamp"
  mkdir -p "$HOME/.local/bin"
  curl -fsSL https://swamp-club.com/install.sh | sh -s -- -d "$HOME/.local/bin"
fi
export PATH="$HOME/.local/bin:$PATH"
KEY_FILE="$HOME/.config/swamp/api-key"
if [ -s "$KEY_FILE" ]; then
  export SWAMP_API_KEY_FILE="$KEY_FILE" SWAMP_NO_INBOX_NOTICE=1
  swamp config set telemetry.collection disabled >/dev/null 2>&1 || true
  echo "== filling the swamp vault from .env"
  set -a; . ./.env; set +a
  for k in ACTUAL_HTTP_API_KEY TYPESAFE_API_KEY ACTUAL_SERVER_APP ACTUAL_API_APP; do
    eval "v=\${$k:-}"
    [ -n "$v" ] || { echo "   $k: not set in .env"; continue; }
    printf '%s' "$v" | swamp vault put default "$k" --yes >/dev/null && echo "   $k: stored"
  done
  swamp workflow validate sync --json >/dev/null && echo "== workflow sync validates"
else
  echo "== no $KEY_FILE: the service will fall back to plain jevcat sync until it exists"
fi
echo "== installing systemd user timer"
mkdir -p ~/.config/systemd/user
cp deploy/pi/jevcat-sync.service deploy/pi/jevcat-sync.timer ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now jevcat-sync.timer
# Keep the user manager alive without a login session, or the timer never fires.
sudo loginctl enable-linger "$USER"
echo "== first run"
systemctl --user start jevcat-sync.service
journalctl --user -u jevcat-sync.service -n 15 --no-pager
systemctl --user list-timers jevcat-sync.timer --no-pager
REMOTE
