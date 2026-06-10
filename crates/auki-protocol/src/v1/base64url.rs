//! Canonical base64url-without-padding helpers for v1 JSON fields.

use std::fmt;

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Errors produced by v1 base64url-without-padding helpers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Base64UrlError {
    /// Input contains `=` padding, which v1 does not allow.
    ContainsPadding {
        /// Byte offset of the padding character.
        index: usize,
    },
    /// Input contains a character outside the base64url alphabet.
    InvalidAlphabet {
        /// Byte offset of the invalid character.
        index: usize,
        /// Invalid byte value.
        byte: u8,
    },
    /// Input length is impossible for unpadded base64.
    InvalidLength {
        /// Input length in bytes.
        len: usize,
    },
    /// Unused trailing bits were non-zero, so the spelling is non-canonical.
    NonCanonicalTrailingBits,
    /// Decoded byte length differed from the required field length.
    DecodedLengthMismatch {
        /// Required decoded byte length.
        expected: usize,
        /// Actual decoded byte length.
        actual: usize,
    },
}

impl fmt::Display for Base64UrlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ContainsPadding { index } => {
                write!(f, "base64url padding is not allowed at byte {index}")
            }
            Self::InvalidAlphabet { index, byte } => {
                write!(f, "invalid base64url byte 0x{byte:02x} at byte {index}")
            }
            Self::InvalidLength { len } => {
                write!(f, "invalid unpadded base64url length {len}")
            }
            Self::NonCanonicalTrailingBits => {
                write!(f, "non-canonical base64url trailing bits")
            }
            Self::DecodedLengthMismatch { expected, actual } => {
                write!(
                    f,
                    "decoded base64url length mismatch: expected {expected}, got {actual}"
                )
            }
        }
    }
}

impl std::error::Error for Base64UrlError {}

/// Encode bytes as base64url without padding.
pub fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    let mut chunks = bytes.chunks_exact(3);

    for chunk in &mut chunks {
        let b0 = chunk[0];
        let b1 = chunk[1];
        let b2 = chunk[2];
        out.push(ALPHABET[(b0 >> 2) as usize] as char);
        out.push(ALPHABET[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        out.push(ALPHABET[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        out.push(ALPHABET[(b2 & 0x3f) as usize] as char);
    }

    match chunks.remainder() {
        [] => {}
        [b0] => {
            out.push(ALPHABET[(b0 >> 2) as usize] as char);
            out.push(ALPHABET[((b0 & 0x03) << 4) as usize] as char);
        }
        [b0, b1] => {
            out.push(ALPHABET[(b0 >> 2) as usize] as char);
            out.push(ALPHABET[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
            out.push(ALPHABET[((b1 & 0x0f) << 2) as usize] as char);
        }
        _ => unreachable!("chunks_exact(3) remainder has at most two bytes"),
    }

    out
}

/// Decode canonical base64url without padding.
pub fn decode(input: &str) -> Result<Vec<u8>, Base64UrlError> {
    if input.len() % 4 == 1 {
        return Err(Base64UrlError::InvalidLength { len: input.len() });
    }

    let sextets = input
        .bytes()
        .enumerate()
        .map(|(index, byte)| decode_sextet(index, byte))
        .collect::<Result<Vec<_>, _>>()?;

    let mut out = Vec::with_capacity((input.len() / 4) * 3 + 2);
    let full_groups = sextets.len() / 4;

    for group_index in 0..full_groups {
        let group = &sextets[group_index * 4..group_index * 4 + 4];
        out.push((group[0] << 2) | (group[1] >> 4));
        out.push(((group[1] & 0x0f) << 4) | (group[2] >> 2));
        out.push(((group[2] & 0x03) << 6) | group[3]);
    }

    match sextets.len() % 4 {
        0 => {}
        2 => {
            let group = &sextets[full_groups * 4..];
            if group[1] & 0x0f != 0 {
                return Err(Base64UrlError::NonCanonicalTrailingBits);
            }
            out.push((group[0] << 2) | (group[1] >> 4));
        }
        3 => {
            let group = &sextets[full_groups * 4..];
            if group[2] & 0x03 != 0 {
                return Err(Base64UrlError::NonCanonicalTrailingBits);
            }
            out.push((group[0] << 2) | (group[1] >> 4));
            out.push(((group[1] & 0x0f) << 4) | (group[2] >> 2));
        }
        _ => return Err(Base64UrlError::InvalidLength { len: input.len() }),
    }

    Ok(out)
}

/// Decode canonical base64url without padding into an exact-length byte array.
pub fn decode_exact<const N: usize>(input: &str) -> Result<[u8; N], Base64UrlError> {
    let bytes = decode(input)?;
    bytes
        .try_into()
        .map_err(|bytes: Vec<u8>| Base64UrlError::DecodedLengthMismatch {
            expected: N,
            actual: bytes.len(),
        })
}

fn decode_sextet(index: usize, byte: u8) -> Result<u8, Base64UrlError> {
    match byte {
        b'A'..=b'Z' => Ok(byte - b'A'),
        b'a'..=b'z' => Ok(byte - b'a' + 26),
        b'0'..=b'9' => Ok(byte - b'0' + 52),
        b'-' => Ok(62),
        b'_' => Ok(63),
        b'=' => Err(Base64UrlError::ContainsPadding { index }),
        _ => Err(Base64UrlError::InvalidAlphabet { index, byte }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locked_encode_vectors() {
        let cases: &[(&[u8], &str)] = &[
            (b"", ""),
            (b"f", "Zg"),
            (b"fo", "Zm8"),
            (b"foo", "Zm9v"),
            (b"foob", "Zm9vYg"),
            (b"fooba", "Zm9vYmE"),
            (b"foobar", "Zm9vYmFy"),
            (&[0xfb, 0xff, 0xff], "-___"),
            (&[0x00; 32], "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
            (
                &[0x00; 64],
                "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            ),
        ];

        for (input, expected) in cases {
            assert_eq!(encode(input), *expected);
            assert_eq!(decode(expected).unwrap(), *input);
        }
    }

    #[test]
    fn decode_exact_returns_array_for_required_lengths() {
        let decoded: [u8; 32] =
            decode_exact("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA").unwrap();
        assert_eq!(decoded, [0u8; 32]);
    }

    #[test]
    fn decode_exact_rejects_wrong_decoded_length() {
        assert_eq!(
            decode_exact::<32>("Zm9v"),
            Err(Base64UrlError::DecodedLengthMismatch {
                expected: 32,
                actual: 3
            })
        );
    }

    #[test]
    fn decode_rejects_padding() {
        assert_eq!(
            decode("Zg="),
            Err(Base64UrlError::ContainsPadding { index: 2 })
        );
    }

    #[test]
    fn decode_rejects_standard_base64_alphabet() {
        assert_eq!(
            decode("+___"),
            Err(Base64UrlError::InvalidAlphabet {
                index: 0,
                byte: b'+'
            })
        );
        assert_eq!(
            decode("/___"),
            Err(Base64UrlError::InvalidAlphabet {
                index: 0,
                byte: b'/'
            })
        );
    }

    #[test]
    fn decode_rejects_impossible_unpadded_length() {
        assert_eq!(decode("A"), Err(Base64UrlError::InvalidLength { len: 1 }));
    }

    #[test]
    fn decode_rejects_non_canonical_one_byte_tail() {
        assert_eq!(decode("Zh"), Err(Base64UrlError::NonCanonicalTrailingBits));
    }

    #[test]
    fn decode_rejects_non_canonical_two_byte_tail() {
        assert_eq!(decode("Zm9"), Err(Base64UrlError::NonCanonicalTrailingBits));
    }
}
