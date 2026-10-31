#!/bin/sh
# Builds jevcat on the Pi and schedules `jevcat sync` weekly with a systemd user timer.
# Run from this repo: deploy/pi/install.sh [user@host]
# The host defaults to JEVCAT_SYNC_HOST from the gitignored .env, so no hostname lives here.
# Copies the repo (with the gitignored jevcat.toml and .env) to ~/sites/furulid-finance on the Pi.
set -eu
HERE="$(cd "$(dirname "$0")/../.." && pwd)"
[ -f "$HERE/.env" ] && JEVCAT_SYNC_HOST="$(sed -n 's/^JEVCAT_SYNC_HOST=//p' "$HERE/.env" | tr -d '"')"
HOST="${1:-${JEVCAT_SYNC_HOST:-}}"
[ -n "$HOST" ] || { echo "usage: $0 user@host   (or set JEVCAT_SYNC_HOST in .env)" >&2; exit 2; }

echo "== copying repo to $HOST:~/sites/furulid-finance"
rsync -az --delete --exclude target --exclude .git "$HERE/" "$HOST:sites/furulid-finance/"

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
