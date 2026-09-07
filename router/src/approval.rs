use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalBinding {
    pub request_hash: String,
    pub provider: String,
    pub model: String,
    pub policy: String,
    pub max_cost_microunits: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Approval {
    pub id: String,
    pub binding: ApprovalBinding,
    pub created_at_unix: i64,
    pub expires_at_unix: i64,
}

pub struct ApprovalLedger {
    conn: Connection,
}

impl ApprovalBinding {
    pub fn canonical_hash(request_json: &serde_json::Value, provider: &str, model: &str, policy: &str, max_cost_microunits: i64) -> Result<Self> {
        let canonical = serde_json::to_vec(request_json).context("serialize canonical request")?;
        let mut hasher = Sha256::new();
        hasher.update(&canonical);
        hasher.update([0]);
        hasher.update(provider.as_bytes());
        hasher.update([0]);
        hasher.update(model.as_bytes());
        hasher.update([0]);
        hasher.update(policy.as_bytes());
        hasher.update([0]);
        hasher.update(max_cost_microunits.to_be_bytes());
        Ok(Self { request_hash: format!("{:x}", hasher.finalize()), provider: provider.to_owned(), model: model.to_owned(), policy: policy.to_owned(), max_cost_microunits })
    }
}

impl ApprovalLedger {
    pub fn open(path: &str) -> Result<Self> {
        let conn = Connection::open(path).with_context(|| format!("open approval ledger {path}"))?;
        conn.execute_batch("
            PRAGMA journal_mode = WAL;
            PRAGMA foreign_keys = ON;
            CREATE TABLE IF NOT EXISTS approvals (
                id TEXT PRIMARY KEY NOT NULL,
                request_hash TEXT NOT NULL,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                policy TEXT NOT NULL,
                max_cost_microunits INTEGER NOT NULL,
                created_at_unix INTEGER NOT NULL,
                expires_at_unix INTEGER NOT NULL,
                consumed_at_unix INTEGER
            );
            CREATE INDEX IF NOT EXISTS approvals_usable ON approvals (id, expires_at_unix, consumed_at_unix);
        ")?;
        Ok(Self { conn })
    }

    pub fn issue(&self, binding: &ApprovalBinding, ttl_seconds: i64, now_unix: i64) -> Result<Approval> {
        if ttl_seconds <= 0 { bail!("approval TTL must be positive") }
        if binding.provider.is_empty() || binding.model.is_empty() || binding.policy.is_empty() { bail!("approval binding requires provider, model, and policy") }
        let approval = Approval { id: Uuid::new_v4().to_string(), binding: binding.clone(), created_at_unix: now_unix, expires_at_unix: now_unix.saturating_add(ttl_seconds) };
        self.conn.execute("INSERT INTO approvals (id, request_hash, provider, model, policy, max_cost_microunits, created_at_unix, expires_at_unix) VALUES (?, ?, ?, ?, ?, ?, ?, ?)", params![approval.id, approval.binding.request_hash, approval.binding.provider, approval.binding.model, approval.binding.policy, approval.binding.max_cost_microunits, approval.created_at_unix, approval.expires_at_unix])?;
        Ok(approval)
    }

    pub fn consume(&self, id: &str, binding: &ApprovalBinding, now_unix: i64) -> Result<()> {
        let found: Option<(String, String, String, String, i64, i64, Option<i64>)> = self.conn.query_row("SELECT request_hash, provider, model, policy, max_cost_microunits, expires_at_unix, consumed_at_unix FROM approvals WHERE id = ?", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?))).optional()?;
        let Some((request_hash, provider, model, policy, max_cost, expires, consumed)) = found else { bail!("unknown approval") };
        if consumed.is_some() { bail!("approval already consumed") }
        if now_unix > expires { bail!("approval expired") }
        if request_hash != binding.request_hash || provider != binding.provider || model != binding.model || policy != binding.policy || max_cost != binding.max_cost_microunits { bail!("approval binding does not match request") }
        let changed = self.conn.execute("UPDATE approvals SET consumed_at_unix = ? WHERE id = ? AND consumed_at_unix IS NULL", params![now_unix, id])?;
        if changed != 1 { bail!("approval was consumed concurrently") }
        Ok(())
    }
}

pub fn now_unix() -> i64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64 }
