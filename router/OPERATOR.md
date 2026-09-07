# Router operator runbook

## Scope

`real-api-router` is a local policy engine and provider gateway. It reads the checked-in research snapshot, recommends a route, and can call only explicitly configured official APIs or local endpoints. It never converts a consumer chat subscription into unattended API capacity.

## Threat model

Treat the router as a boundary that can spend API credits and receive sensitive prompts. Keep it bound to loopback unless it is behind a mutually authenticated private network. Do not expose port 8787 directly to the public internet.

## Local macOS install

```bash
git clone --branch feature/rust-router git@github.com:colombianh/real-api-pricing.git
cd real-api-pricing/router
cp router.example.toml router.toml
cp .env.example .env
chmod 600 .env router.toml
# Put only official API keys in .env, then:
cargo run -- --config router.toml validate
cargo run -- --config router.toml serve
```

Use `ROUTER_BEARER_TOKEN` whenever another process connects to the server. Send it in `Authorization: Bearer <token>`.

## Enrollment workflow

1. Add a provider block to local `router.toml`. Give it an immutable ID, endpoint URL, supported-region list, and the environment-variable name that holds its API key.
2. Add the API key only to local `.env` or a system keychain/secret manager injected into the service environment.
3. Call `GET /v1/catalog` and confirm the provider/model candidates are visible.
4. Call `POST /v1/route` with `require_approval: true`. Review the recommendation, alternatives, snapshot age, estimated cost, and eligibility reasons.
5. Use an explicit approval action in your UI/agent. Only then reissue the chat request with `approved: true`.
6. Review `router-audit.jsonl` for provider/model/latency/failure metadata. Prompt text is intentionally not written there.

## Human-in-the-loop policy

For paid remote providers, keep `require_approval` true during early operation. A capable agent can ask: "The private local model is available but lower capability. The recommended paid API route is Provider/Model X based on this snapshot and health score. Estimated unit price is Y. Proceed?"

Approval must belong to the caller/UI, not the model. Store request ID, selected provider/model, allowed maximum cost, and approval expiry in your application before making a paid request. The router's first version exposes the proposal gate; a durable approval ledger belongs in the next SQLite/LibSQL persistence push.

## Freshness and updates

The upstream project is a research snapshot, not a guaranteed live-pricing feed. Pull updates deliberately, validate them, inspect the diff, and restart the service only after it loads successfully:

```bash
git fetch origin
git log --oneline HEAD..origin/main
git merge --ff-only origin/main
cd router
cargo run -- --config router.toml validate
```

Set `max_snapshot_age_hours` conservatively. If the data is old, treat the rank as an advisory estimate rather than a price quote.

## Provider failure response

- A failed request increases the provider failure count and starts a temporary cooldown.
- Do not automatically retry non-idempotent tool/action requests unless your caller has an idempotency key and records the outcome.
- Prefer local-only mode for sensitive inputs.
- If the provider returns rate limiting, lower concurrency and let the cooldown expire; do not attempt to evade quotas.

## Do not add

- Consumer-account passwords, browser cookies, session tokens, or unofficial subscription gateways.
- API keys in TOML, source files, shell history, Git commits, issues, or logs.
- Public unauthenticated access to the router.
