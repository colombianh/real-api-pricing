# Router testing contract

The router test suite is intentionally small at this stage, but it protects non-negotiable operating rules while the codebase is modularized.

## Current checks

- `policy_contract.rs` verifies that shipped policy presets remain valid JSON, retain their intended modes, keep paid/budget policies approval-gated, and keep the private preset local-only.
- `secret_template_contract.rs` verifies that `.env.example` contains no secret values and that `.gitignore` excludes actual environment/config/audit files.

Run locally:

```bash
cd router
cargo test --all-targets
```

## Next test layers

1. Fixture-based tests against the exact current `derived/points.json` schema.
2. Routing property tests: no blocked provider, region, cost cap, local-only, or capability constraint may be violated.
3. Approval binding tests: an approval must be single-use, time-limited, and bound to one canonical request hash plus provider/model/budget.
4. Mock-server tests for OpenAI-compatible, Anthropic, 429, timeout, and failover behavior.
5. SQLite/LibSQL migration and audit-ledger tests.
