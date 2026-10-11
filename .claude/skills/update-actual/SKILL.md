---
name: update-actual
description: Upgrade the Actual server and actual-http-api on Fly.io to the newest version both have released, keeping the two on the same version. Use when asked to update, upgrade or bump Actual, actual-server, actual-http-api or the budget server, or to check whether a new Actual version is out.
---

# Update Actual and actual-http-api

actual-http-api is versioned after the `@actual-app/api` it bundles, which is versioned
after Actual itself: actual-http-api `X.Y.Z` talks to an Actual server `X.Y.Z`. An API newer
than the server can fail to open the budget, so the two images always carry the same tag and
the server is upgraded first.

The pinned tags live in `deploy/actual-server/fly.toml` and `deploy/actual-api/fly.toml`. The
Fly app names are not in the repo; they come from `.env` (`ACTUAL_SERVER_APP`,
`ACTUAL_API_APP`).

## 1. Pick the target version

```sh
.claude/skills/update-actual/versions.sh
```

`target` is the newest `X.Y.Z` published by both images. If it equals the pinned tags,
report that everything is current and stop. If `latest server` is newer than `target`,
actual-http-api has not caught up yet: upgrade to `target` anyway (or stop if that is the
pinned version) and say which server version is waiting on the API.

Read the Actual release notes for every version between pinned and target
(https://github.com/actualbudget/actual/releases, also https://actualbudget.org/docs/releases)
and the actual-http-api releases (https://github.com/jhonderson/actual-http-api/releases).
Stop and ask before going on if they mention breaking changes, required manual steps, or a
change to how the server is started (the server `fly.toml` overrides the command with
`node --max-old-space-size=180 app.js`).

## 2. Check the live state

```sh
set -a; . ./.env; set +a
curl -s "https://$ACTUAL_SERVER_APP.fly.dev/info"   # build.version is what the server runs
fly status -a "$ACTUAL_SERVER_APP"
fly status -a "$ACTUAL_API_APP"
```

If the running server version differs from the pinned tag, someone deployed outside this
repo. Report it and ask before continuing.

## 3. Snapshot the budget volume

The server migrates its data on the first start of a new version. Take a snapshot so it can
be rolled back:

```sh
fly volumes list -a "$ACTUAL_SERVER_APP"                 # the actual_data volume id
fly volumes snapshots create <volume id> -a "$ACTUAL_SERVER_APP"
```

Do not continue if the snapshot fails.

## 4. Upgrade the server, then the API

This step changes production, so confirm with the user before starting it unless they
already asked for the upgrade to be deployed.

Set the `image` tag in both `fly.toml` files to the target, then:

```sh
fly deploy -c deploy/actual-server/fly.toml -a "$ACTUAL_SERVER_APP"
curl -s "https://$ACTUAL_SERVER_APP.fly.dev/info"        # must report the target version
fly deploy -c deploy/actual-api/fly.toml -a "$ACTUAL_API_APP"
```

Only deploy the API once `/info` reports the target. If the server deploy fails or `/info`
does not come back, revert both tags, redeploy the old server tag and report; do not deploy
the API.

## 5. Verify

```sh
cargo run --quiet -- list
```

This loads the budget through actual-http-api with the keys in `.env` and the URL in
`jevcat.toml`, and calls nothing else. It must print transactions (or an empty list) without
errors. The first request after a deploy can take a few seconds while the API pulls the
budget. On a version mismatch error, check `fly logs -a "$ACTUAL_API_APP"`.

## 6. Commit

Commit only the two `fly.toml` changes, e.g. `Bump Actual and actual-http-api to 26.11.0`,
and push. Never commit `.env`, `jevcat.toml` or app names.

Report the old and new versions, the snapshot id, and anything from the release notes worth
knowing.
