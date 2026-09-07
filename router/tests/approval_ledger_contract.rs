#[path = "../src/approval.rs"]
mod approval;

use approval::{ApprovalBinding, ApprovalLedger};
use serde_json::json;

fn binding(provider: &str, model: &str) -> ApprovalBinding {
    ApprovalBinding::canonical_hash(&json!({"messages":[{"role":"user","content":"hello"}]}), provider, model, "coding", 25_000_000).unwrap()
}

#[test]
fn approval_is_single_use_and_bound_to_one_route() {
    let ledger = ApprovalLedger::open(":memory:").unwrap();
    let original = binding("openai", "example-model");
    let issued = ledger.issue(&original, 120, 1_000).unwrap();

    ledger.consume(&issued.id, &original, 1_001).unwrap();
    assert!(ledger.consume(&issued.id, &original, 1_002).is_err());
}

#[test]
fn approval_rejects_changed_provider_model_or_request() {
    let ledger = ApprovalLedger::open(":memory:").unwrap();
    let original = binding("openai", "example-model");
    let issued = ledger.issue(&original, 120, 1_000).unwrap();

    assert!(ledger.consume(&issued.id, &binding("anthropic", "example-model"), 1_001).is_err());
    assert!(ledger.consume(&issued.id, &binding("openai", "other-model"), 1_001).is_err());
}

#[test]
fn approval_expires() {
    let ledger = ApprovalLedger::open(":memory:").unwrap();
    let route = binding("openai", "example-model");
    let issued = ledger.issue(&route, 5, 1_000).unwrap();
    assert!(ledger.consume(&issued.id, &route, 1_006).is_err());
}
