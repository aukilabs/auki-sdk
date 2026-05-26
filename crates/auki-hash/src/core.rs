//! Binding-free XXH3-128 content hash implementation.

/// Hash already-canonical bytes (typically `auki_jcs::canonicalize` output)
/// to a 32-character lowercase hex string.
///
/// Uses XXH3-128 with seed 0; the seed is fixed because content addressing
/// must not vary per-instance.
pub fn hash_jcs_bytes(bytes: &[u8]) -> String {
    let h = xxhash_rust::xxh3::xxh3_128(bytes);
    format!("{h:032x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_lower_hex(s: &str) -> bool {
        s.chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
    }

    #[test]
    fn length_is_always_32() {
        // XXH3-128 is 128 bits, so the output is exactly 32 hex chars.
        assert_eq!(hash_jcs_bytes(b"").len(), 32);
        assert_eq!(hash_jcs_bytes(b"a").len(), 32);
        let kib: Vec<u8> = (0..1024u32).map(|i| (i & 0xff) as u8).collect();
        assert_eq!(hash_jcs_bytes(&kib).len(), 32);
    }

    #[test]
    fn lowercase_hex_only() {
        assert!(is_lower_hex(&hash_jcs_bytes(b"")));
        assert!(is_lower_hex(&hash_jcs_bytes(b"hello world")));
        assert!(is_lower_hex(&hash_jcs_bytes(&[0xff; 64])));
    }

    #[test]
    fn deterministic() {
        assert_eq!(hash_jcs_bytes(b"hello"), hash_jcs_bytes(b"hello"));
        let a = hash_jcs_bytes(b"the quick brown fox jumps over the lazy dog");
        let b = hash_jcs_bytes(b"the quick brown fox jumps over the lazy dog");
        assert_eq!(a, b);
    }

    #[test]
    fn different_inputs_differ() {
        assert_ne!(hash_jcs_bytes(b"hello"), hash_jcs_bytes(b"hellp"));
        assert_ne!(hash_jcs_bytes(b""), hash_jcs_bytes(b" "));
        assert_ne!(hash_jcs_bytes(b"abc"), hash_jcs_bytes(b"abd"));
    }

    #[test]
    fn known_vector_empty() {
        assert_eq!(hash_jcs_bytes(b""), "99aa06d3014798d86001c324468d497f");
    }

    #[test]
    fn known_vector_abc() {
        assert_eq!(hash_jcs_bytes(b"abc"), "06b05ab6733a618578af5f94892f3950");
    }
}
