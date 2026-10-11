#!/usr/bin/env bash
# Prints the pinned, newest and newest-common release of the Actual server and
# actual-http-api images. The two projects share version numbers, so the target
# is the newest X.Y.Z published by both.
set -euo pipefail
root="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"

tags() {
  curl -fsS "https://hub.docker.com/v2/repositories/$1/tags?page_size=100&ordering=last_updated" |
    python3 -c 'import json,re,sys
for t in json.load(sys.stdin)["results"]:
    if re.fullmatch(r"\d+\.\d+\.\d+", t["name"]): print(t["name"])' |
    sort -t. -k1,1n -k2,2n -k3,3n
}
pinned() { sed -nE "s/^image = ['\"][^:]+:([^'\"]+)['\"].*/\1/p" "$root/$1"; }

server=$(tags actualbudget/actual-server)
api=$(tags jhonderson/actual-http-api)
common=$(comm -12 <(echo "$server" | sort) <(echo "$api" | sort) | sort -t. -k1,1n -k2,2n -k3,3n | tail -1)

echo "pinned server: $(pinned deploy/actual-server/fly.toml)"
echo "pinned api:    $(pinned deploy/actual-api/fly.toml)"
echo "latest server: $(echo "$server" | tail -1)"
echo "latest api:    $(echo "$api" | tail -1)"
echo "target:        $common"
