use serde_json::Value;

#[test]
fn policy_examples_are_valid_and_safe() {
    let value: Value = serde_json::from_str(include_str!("../policy.example.json"))
        .expect("policy.example.json must be valid JSON");
    let object = value.as_object().expect("policy file must be an object");

    for required in ["interactive_coding", "private_local", "budget_limited"] {
        assert!(object.contains_key(required), "missing {required} policy");
    }

    let coding = &object["interactive_coding"];
    assert_eq!(coding["mode"], "coding");
    assert_eq!(coding["require_approval"], true);

    let local = &object["private_local"];
    assert_eq!(local["mode"], "local_only");
    assert_eq!(local["local_only"], true);

    let budget = &object["budget_limited"];
    assert_eq!(budget["mode"], "cheapest");
    assert_eq!(budget["require_approval"], true);
    assert!(budget["maximum_cost"].as_f64().is_some_and(|v| v > 0.0));
}
