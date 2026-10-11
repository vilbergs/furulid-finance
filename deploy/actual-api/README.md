# actual-http-api on Fly.io

Runs [actual-http-api](https://github.com/jhonderson/actual-http-api) as a second Fly app
next to your Actual server, so jevcat can reach the budget over HTTP. Traffic between the
two apps stays on Fly's private network.

## Deploy

Neither `fly.toml` names its app, so the repo does not reveal where your budget lives. Put
the app names in `.env` (see `.env.example`) and load it into your shell first:

```sh
set -a; . ./.env; set +a   # ACTUAL_SERVER_APP, ACTUAL_API_APP
```

The image tag must match the version the Actual server runs (`../actual-server/fly.toml`,
or `curl https://$ACTUAL_SERVER_APP.fly.dev/info`); bump both together.

```sh
fly deploy -c deploy/actual-server/fly.toml -a "$ACTUAL_SERVER_APP"   # the Actual server

cd deploy/actual-api
fly launch --copy-config --no-deploy --name "$ACTUAL_API_APP"   # first time only
fly volumes create actual_api_data --size 1 -a "$ACTUAL_API_APP"  # cache for the downloaded budget
fly secrets set -a "$ACTUAL_API_APP" \
  API_KEY="$(openssl rand -hex 32)" \
  ACTUAL_SERVER_PASSWORD="<your Actual server password>" \
  ACTUAL_SERVER_URL="http://$ACTUAL_SERVER_APP.internal:5006/"
fly deploy -a "$ACTUAL_API_APP"
```

Keep the generated `API_KEY`. It only authenticates callers of this sidecar and is what
jevcat sends in the `x-api-key` header.

## Check it

```sh
curl -s https://$ACTUAL_API_APP.fly.dev/v1/budgets/<sync id>/accounts -H "x-api-key: <API_KEY>"
```

Your accounts should come back as JSON. The Swagger UI is at `/api-docs/`. The first
request after the machine has slept takes a few seconds while it boots and pulls the budget.

## Point jevcat at it

In `jevcat.toml` (gitignored):

```toml
[actual]
base_url = "https://<your api app>.fly.dev"
budget_sync_id = "<sync id>"
```

```sh
export ACTUAL_HTTP_API_KEY="<API_KEY>"
export ACTUAL_ENCRYPTION_PASSWORD="…"   # only for an end-to-end encrypted budget
jevcat list
```

## Keeping it off the public internet

Delete the `[http_service]` block, redeploy, and open a tunnel while you run jevcat:

```sh
fly proxy 5007 -a "$ACTUAL_API_APP"
```

with `base_url = "http://localhost:5007"` in `jevcat.toml`.

## If `.internal` does not resolve

Use your Actual instance's public URL instead:
`fly secrets set -a "$ACTUAL_API_APP" ACTUAL_SERVER_URL="https://$ACTUAL_SERVER_APP.fly.dev/"`.
