# Real API Router

A Rust service that consumes this repository's derived pricing/capability snapshot and chooses an eligible **official API or local endpoint**. It does not turn consumer subscriptions into APIs, scrape browser sessions, or bypass provider quotas.

## Guarantees and boundaries

- `subscription` rows are recommendation/analysis signals. They are never invoked unless you separately configure a provider-supported API endpoint.
- Secrets are read only from environment variables. Do not commit `.env`, OAuth refresh tokens, browser cookies, or account passwords.
- A routing response reports the snapshot freshness, score breakdown, rejected candidates, and whether a provider is in a cooldown.
- `require_approval: true` makes the router propose a route without calling a paid external provider. A caller can present that proposal to a human/agent UI and re-submit with `approved: true`.
- Provider authentication is API-key based in this first version. OAuth can be added only per provider's documented authorization flow and requires that provider's client ID, redirect URI, scopes, and token endpoint. Consumer subscription login is intentionally unsupported.

## Run

```bash
cd router
cp router.example.toml router.toml
cp .env.example .env
# Edit router.toml and put supported API keys in .env
cargo run -- --config router.toml serve
```

Read [OPERATOR.md](OPERATOR.md) before exposing the service beyond your machine.

## Ask for a route

```bash
curl -s http://127.0.0.1:8787/v1/route \
  -H 'content-type: application/json' \
  -d '{
    "mode": "coding",
    "minimum_capability": 50,
    "allowed_regions": ["US", "GLOBAL", "LOCAL"],
    "require_approval": true
  }'
```

The response is a proposal only. Set `approved: true` and call `/v1/chat/completions` with `messages` to allow an actual provider call. Pre-built policy examples live in `policy.example.json`.

## Endpoints

- `GET /v1/health` — router and snapshot health
- `GET /v1/catalog` — normalized candidates, price snapshot, and provider state
- `POST /v1/route` — rank candidates without spending funds
- `POST /v1/chat/completions` — choose then call an official configured API/local endpoint

## Snapshot freshness

This service uses the repository checkout as a snapshot. Pull upstream changes on a schedule, validate changes in CI, and restart/reload the process after the importer receives a new valid snapshot. It does not claim real-time provider pricing unless you add a verified provider-specific price adapter.
