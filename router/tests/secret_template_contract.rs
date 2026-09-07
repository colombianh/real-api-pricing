#[test]
fn environment_template_contains_only_empty_secret_values() {
    let template = include_str!("../.env.example");
    for line in template.lines().filter(|line| !line.trim_start().starts_with('#')) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (_, value) = line.split_once('=').expect("env template lines need KEY=VALUE");
        assert!(value.is_empty(), "example environment files must not carry secrets");
    }
}

#[test]
fn gitignore_excludes_local_router_secrets_and_state() {
    let ignore = include_str!("../.gitignore");
    for required in [".env", "router.toml", "router-audit.jsonl"] {
        assert!(ignore.lines().any(|line| line.trim() == required), ".gitignore must include {required}");
    }
}
