//! V1 JSON frame encoding and decoding.
//!
//! A v1 JSON frame is an unsigned LEB128 body length followed by exactly that
//! many UTF-8 JSON bytes. This module is pure byte-buffer code; stream adapters
//! belong in the runtime crate.

use super::json::{self, JsonError};
use std::{fmt, str};

/// Maximum number of bytes in an unsigned LEB128-encoded `u64`.
pub const MAX_LEB128_U64_BYTES: usize = 10;

/// Errors produced by v1 frame helpers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    /// Length prefix ended before a terminating byte was available.
    UnexpectedEof,
    /// Length prefix used more than ten bytes or kept the continuation bit set
    /// on the tenth byte.
    LengthPrefixTooLong,
    /// Length prefix encodes a value larger than `u64::MAX`.
    LengthOverflow,
    /// Length prefix was valid LEB128 but not the shortest valid encoding.
    NonMinimalLength,
    /// Frame body length exceeds the caller-supplied local frame limit.
    BodyTooLarge {
        /// Length declared by the frame.
        actual: u64,
        /// Local frame body limit.
        max: u64,
    },
    /// Frame body ended before the declared number of bytes was available.
    TruncatedBody {
        /// Length declared by the frame.
        expected: u64,
        /// Body bytes available after the prefix.
        actual: u64,
    },
    /// Frame body is not valid UTF-8.
    InvalidUtf8,
    /// Frame body is not valid JSON.
    InvalidJson(String),
    /// Frame body repeats a JSON object member name.
    DuplicateMemberName {
        /// Repeated member name.
        name: String,
    },
    /// Frame body parsed as JSON but was not one JSON object.
    BodyNotObject,
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEof => write!(f, "unexpected eof while reading length prefix"),
            Self::LengthPrefixTooLong => write!(f, "length prefix exceeds ten bytes"),
            Self::LengthOverflow => write!(f, "length prefix exceeds u64 range"),
            Self::NonMinimalLength => write!(f, "length prefix is not minimally encoded"),
            Self::BodyTooLarge { actual, max } => {
                write!(f, "frame body too large: {actual} bytes (max {max})")
            }
            Self::TruncatedBody { expected, actual } => {
                write!(
                    f,
                    "truncated frame body: expected {expected} bytes, got {actual}"
                )
            }
            Self::InvalidUtf8 => write!(f, "frame body is not valid utf-8"),
            Self::InvalidJson(err) => write!(f, "frame body is not valid json: {err}"),
            Self::DuplicateMemberName { name } => {
                write!(f, "frame body repeats json member name: {name}")
            }
            Self::BodyNotObject => write!(f, "frame body is not a json object"),
        }
    }
}

impl std::error::Error for FrameError {}

impl From<JsonError> for FrameError {
    fn from(error: JsonError) -> Self {
        match error {
            JsonError::InvalidJson(err) => Self::InvalidJson(err),
            JsonError::DuplicateMemberName { name } => Self::DuplicateMemberName { name },
            JsonError::BodyNotObject => Self::BodyNotObject,
        }
    }
}

/// Encode `value` as the shortest unsigned LEB128 `u64` byte sequence.
pub fn encode_length(value: u64) -> Vec<u8> {
    let mut value = value;
    let mut out = Vec::with_capacity(MAX_LEB128_U64_BYTES);
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            return out;
        }
    }
}

/// Decode the unsigned LEB128 `u64` prefix at the beginning of `input`.
///
/// Returns `(value, bytes_consumed)` on success. The decoded value is checked
/// against `max_body_len`, and non-minimal encodings are rejected.
pub fn decode_length(input: &[u8], max_body_len: u64) -> Result<(u64, usize), FrameError> {
    let mut value: u64 = 0;

    for index in 0..MAX_LEB128_U64_BYTES {
        let Some(&byte) = input.get(index) else {
            return Err(FrameError::UnexpectedEof);
        };
        let payload = (byte & 0x7f) as u64;

        if index == MAX_LEB128_U64_BYTES - 1 && payload > 1 {
            return Err(FrameError::LengthOverflow);
        }

        value |= payload << (index * 7);

        if byte & 0x80 == 0 {
            let consumed = index + 1;
            if encode_length(value).len() != consumed {
                return Err(FrameError::NonMinimalLength);
            }
            if value > max_body_len {
                return Err(FrameError::BodyTooLarge {
                    actual: value,
                    max: max_body_len,
                });
            }
            return Ok((value, consumed));
        }
    }

    Err(FrameError::LengthPrefixTooLong)
}

/// Encode a compact v1 JSON frame for `value`.
///
/// The value must serialize to a JSON object. The serialized body length is
/// checked against `max_body_len`.
pub fn encode_json_frame(
    value: &serde_json::Value,
    max_body_len: u64,
) -> Result<Vec<u8>, FrameError> {
    if !value.is_object() {
        return Err(FrameError::BodyNotObject);
    }

    let body = serde_json::to_vec(value).map_err(|err| FrameError::InvalidJson(err.to_string()))?;
    if body.len() as u64 > max_body_len {
        return Err(FrameError::BodyTooLarge {
            actual: body.len() as u64,
            max: max_body_len,
        });
    }

    let mut frame = encode_length(body.len() as u64);
    frame.extend_from_slice(&body);
    Ok(frame)
}

/// Decode one complete v1 JSON frame from the beginning of `input`.
///
/// Returns `(body, bytes_consumed)` on success. The body must be valid UTF-8,
/// valid JSON, and exactly one JSON object.
pub fn decode_json_frame(
    input: &[u8],
    max_body_len: u64,
) -> Result<(serde_json::Value, usize), FrameError> {
    let (body_len, prefix_len) = decode_length(input, max_body_len)?;
    let body_start = prefix_len;
    let available_body_len = input.len().saturating_sub(body_start) as u64;
    if available_body_len < body_len {
        return Err(FrameError::TruncatedBody {
            expected: body_len,
            actual: available_body_len,
        });
    }
    let body_len = usize::try_from(body_len).map_err(|_| FrameError::LengthOverflow)?;
    let body_end = body_start + body_len;

    let body = &input[body_start..body_end];
    let body = str::from_utf8(body).map_err(|_| FrameError::InvalidUtf8)?;
    let value = json::parse_json_object(body)?;
    Ok((value, body_end))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn length_encoding_uses_single_byte_for_small_values() {
        assert_eq!(encode_length(0), vec![0x00]);
        assert_eq!(encode_length(1), vec![0x01]);
        assert_eq!(encode_length(127), vec![0x7f]);
    }

    #[test]
    fn length_encoding_uses_little_endian_groups() {
        assert_eq!(encode_length(128), vec![0x80, 0x01]);
        assert_eq!(encode_length(16_384), vec![0x80, 0x80, 0x01]);
        assert_eq!(
            decode_length(&[0x80, 0x80, 0x01], 20_000).unwrap(),
            (16_384, 3)
        );
    }

    #[test]
    fn length_round_trips_u64_max() {
        let encoded = encode_length(u64::MAX);
        assert_eq!(encoded.len(), MAX_LEB128_U64_BYTES);
        assert_eq!(decode_length(&encoded, u64::MAX).unwrap(), (u64::MAX, 10));
    }

    #[test]
    fn decode_rejects_non_minimal_prefix() {
        assert_eq!(
            decode_length(&[0x80, 0x00], 1024),
            Err(FrameError::NonMinimalLength)
        );
        assert_eq!(
            decode_length(&[0x81, 0x00], 1024),
            Err(FrameError::NonMinimalLength)
        );
    }

    #[test]
    fn decode_rejects_unterminated_prefix() {
        assert_eq!(decode_length(&[0x80], 1024), Err(FrameError::UnexpectedEof));
    }

    #[test]
    fn decode_rejects_over_ten_byte_prefix() {
        let prefix = [0x80; 10];
        assert_eq!(
            decode_length(&prefix, u64::MAX),
            Err(FrameError::LengthPrefixTooLong)
        );
    }

    #[test]
    fn decode_rejects_u64_overflow_prefix() {
        let mut prefix = [0x80; 10];
        prefix[9] = 0x02;
        assert_eq!(
            decode_length(&prefix, u64::MAX),
            Err(FrameError::LengthOverflow)
        );
    }

    #[test]
    fn decode_rejects_body_larger_than_limit() {
        assert_eq!(
            decode_length(&encode_length(1025), 1024),
            Err(FrameError::BodyTooLarge {
                actual: 1025,
                max: 1024
            })
        );
    }

    #[test]
    fn json_frame_round_trips_object_and_reports_consumed_bytes() {
        let value = json!({"type": "example.v1", "n": 1});
        let mut frame = encode_json_frame(&value, 1024).unwrap();
        let frame_len = frame.len();
        frame.extend_from_slice(b"tail");

        let (decoded, consumed) = decode_json_frame(&frame, 1024).unwrap();
        assert_eq!(decoded, value);
        assert_eq!(consumed, frame_len);
    }

    #[test]
    fn json_frame_rejects_non_object_body() {
        let mut frame = encode_length(4);
        frame.extend_from_slice(b"true");
        assert_eq!(
            decode_json_frame(&frame, 1024),
            Err(FrameError::BodyNotObject)
        );
        assert_eq!(
            encode_json_frame(&json!(["not", "object"]), 1024),
            Err(FrameError::BodyNotObject)
        );
    }

    #[test]
    fn json_frame_rejects_invalid_utf8_body() {
        let frame = [0x01, 0xff];
        assert_eq!(
            decode_json_frame(&frame, 1024),
            Err(FrameError::InvalidUtf8)
        );
    }

    #[test]
    fn json_frame_rejects_non_standard_numeric_values() {
        let mut frame = encode_length(9);
        frame.extend_from_slice(br#"{"n":NaN}"#);
        assert!(matches!(
            decode_json_frame(&frame, 1024),
            Err(FrameError::InvalidJson(_))
        ));
    }

    #[test]
    fn json_frame_rejects_duplicate_member_names() {
        let body = br#"{"a":1,"a":2}"#;
        let mut frame = encode_length(body.len() as u64);
        frame.extend_from_slice(body);

        assert_eq!(
            decode_json_frame(&frame, 1024),
            Err(FrameError::DuplicateMemberName {
                name: "a".to_owned()
            })
        );
    }

    #[test]
    fn json_frame_rejects_nested_duplicate_member_names() {
        let body = br#"{"items":[{"a":1,"a":2}]}"#;
        let mut frame = encode_length(body.len() as u64);
        frame.extend_from_slice(body);

        assert_eq!(
            decode_json_frame(&frame, 1024),
            Err(FrameError::DuplicateMemberName {
                name: "a".to_owned()
            })
        );
    }

    #[test]
    fn json_frame_rejects_truncated_body() {
        let mut frame = encode_length(5);
        frame.extend_from_slice(b"{\"a\"");
        assert_eq!(
            decode_json_frame(&frame, 1024),
            Err(FrameError::TruncatedBody {
                expected: 5,
                actual: 4
            })
        );
    }
}
