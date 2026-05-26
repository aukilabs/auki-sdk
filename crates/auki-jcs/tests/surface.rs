use auki_jcs::{canonicalize, canonicalize_json_str};
use serde_json::json;

#[test]
fn rust_root_api_remains_source_compatible() {
    assert_eq!(canonicalize(&json!({"b": 2, "a": 1})), br#"{"a":1,"b":2}"#);
}

#[test]
fn json_string_adapter_matches_root_api() {
    assert_eq!(
        canonicalize_json_str(r#"{"b":2,"a":1}"#).unwrap(),
        canonicalize(&json!({"b": 2, "a": 1}))
    );
}
