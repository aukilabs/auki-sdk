//! Binding-free RFC 8785 JSON Canonicalization Scheme implementation.

/// Canonicalize a parsed JSON value into RFC 8785 canonical UTF-8 bytes.
///
/// This is infallible because `serde_json::Value` cannot hold non-finite
/// numbers such as NaN or infinity.
pub fn canonicalize(value: &serde_json::Value) -> Vec<u8> {
    serde_jcs::to_vec(value).expect("serde_json::Value cannot contain non-finite numbers")
}

/// Parse a JSON document and canonicalize it into RFC 8785 canonical UTF-8 bytes.
pub fn canonicalize_json_str(json: &str) -> Result<Vec<u8>, serde_json::Error> {
    let value: serde_json::Value = serde_json::from_str(json)?;
    Ok(canonicalize(&value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn canon(v: serde_json::Value) -> String {
        String::from_utf8(canonicalize(&v)).expect("JCS output is valid UTF-8")
    }

    #[test]
    fn empty_object_and_array() {
        assert_eq!(canon(json!({})), "{}");
        assert_eq!(canon(json!([])), "[]");
    }

    #[test]
    fn primitives() {
        assert_eq!(canon(json!(null)), "null");
        assert_eq!(canon(json!(true)), "true");
        assert_eq!(canon(json!(false)), "false");
        assert_eq!(canon(json!(0)), "0");
        assert_eq!(canon(json!(-1)), "-1");
        assert_eq!(canon(json!(1.5)), "1.5");
    }

    #[test]
    fn object_keys_are_sorted() {
        assert_eq!(
            canon(json!({"b": 1, "a": 2, "c": 3})),
            r#"{"a":2,"b":1,"c":3}"#
        );
        assert_eq!(
            canon(json!({"z": {"b": 1, "a": 2}, "a": []})),
            r#"{"a":[],"z":{"a":2,"b":1}}"#
        );
    }

    #[test]
    fn array_order_is_preserved() {
        assert_eq!(canon(json!([3, 1, 2])), "[3,1,2]");
    }

    #[test]
    fn control_chars_use_lowercase_hex_escapes() {
        assert_eq!(canon(json!("\u{0001}")), r#""\u0001""#);
        assert_eq!(canon(json!("\u{000f}")), r#""\u000f""#);
        assert_eq!(canon(json!("\t")), r#""\t""#);
        assert_eq!(canon(json!("\n")), r#""\n""#);
        assert_eq!(canon(json!("\"\\")), r#""\"\\""#);
    }

    #[test]
    fn non_ascii_passes_through_as_utf8() {
        assert_eq!(canon(json!("€")), "\"€\"");
        assert_eq!(canon(json!("汉")), "\"汉\"");
    }

    #[test]
    fn forward_slash_is_not_escaped() {
        assert_eq!(canon(json!("a/b")), r#""a/b""#);
    }

    #[test]
    fn round_trip_is_stable() {
        let v = json!({
            "z": [3, 2, 1, {"k": "v", "j": null}],
            "a": "hello",
            "m": 3.14,
            "t": true,
            "f": false,
            "n": null,
            "s": "with \"quotes\" and \\ backslash and / slash"
        });
        let first = canonicalize(&v);
        let reparsed: serde_json::Value =
            serde_json::from_slice(&first).expect("canonical bytes must parse as JSON");
        let second = canonicalize(&reparsed);
        assert_eq!(first, second);
    }

    #[test]
    fn canonicalize_json_str_parses_then_canonicalizes() {
        let canonical = canonicalize_json_str(r#"{"b":2,"a":1}"#).unwrap();
        assert_eq!(
            String::from_utf8(canonical).expect("JCS output is valid UTF-8"),
            r#"{"a":1,"b":2}"#
        );
    }

    #[test]
    fn canonicalize_json_str_reports_parse_errors() {
        assert!(canonicalize_json_str("{").is_err());
    }
}
