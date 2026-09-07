# Durable approval design

The approval ledger is the foundation for human-in-the-loop paid routing. It stores approvals locally in SQLite using WAL mode.

## Approval binding

An approval is bound to:

- A SHA-256 request hash.
- Provider ID.
- Model ID.
- Named routing policy.
- Maximum cost in integer microunits.
- Creation timestamp and expiry timestamp.

It is **single-use**. A request that changes any bound field must receive a new approval. Expired, consumed, unknown, and concurrently consumed approvals are rejected.

## Current integration status

This commit adds the tested durable ledger primitive. The next source integration replaces the prototype `approved: true` transport flag with a required ledger-issued approval ID in the HTTP request path. That integration will also persist provider health and store redacted route-decision metadata alongside the approval.

No prompt text, provider API key, password, browser cookie, or consumer-subscription credential is stored by the ledger.
