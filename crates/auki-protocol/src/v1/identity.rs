//! Peer identity authority objects for v1.

use super::{
    base64url::{self, Base64UrlError},
    error,
};
use auki_identity::{PublicKey as WalletPublicKey, Signature, VerifyError, Wallet, verify};
use libp2p_identity::PeerId;
use serde_json::{Map, Value};
use std::{fmt, str::FromStr};

/// V1 peer binding object type.
pub const PEER_BINDING_TYPE: &str = "auki.peer_binding.v1";
/// V1 Ed25519 wallet signature scheme label.
pub const WALLET_SIGNATURE_SCHEME_ED25519: &str = "ed25519";

const FIELD_TYPE: &str = "type";
const FIELD_WALLET_SIGNATURE_SCHEME: &str = "wallet_signature_scheme";
const FIELD_WALLET_PUBLIC_KEY: &str = "wallet_public_key";
const FIELD_PEER_ID: &str = "peer_id";
const FIELD_ISSUED_AT: &str = "issued_at";
const FIELD_LABEL: &str = "label";
const FIELD_SIGNATURE: &str = "signature";

/// A v1 wallet-signed binding from a wallet authority key to a libp2p peer id.
#[derive(Debug, Clone, PartialEq)]
pub struct PeerBinding {
    value: Value,
}

/// A successfully verified v1 peer binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedPeerBinding {
    /// Wallet public key that signed the binding.
    pub wallet_public_key: WalletPublicKey,
    /// Parsed libp2p peer id authorized by the wallet.
    pub peer_id: PeerId,
    /// RFC3339 UTC timestamp carried by the binding.
    pub issued_at: String,
    /// Optional operator/application label.
    pub label: Option<String>,
}

/// Local freshness policy for verified peer bindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerBindingFreshnessPolicy {
    /// Maximum accepted age from `issued_at`, in milliseconds.
    pub max_age_ms: Option<u64>,
    /// Maximum accepted future clock skew from `issued_at`, in milliseconds.
    pub future_tolerance_ms: Option<u64>,
}

impl PeerBindingFreshnessPolicy {
    /// Create a policy that does not enforce freshness.
    pub fn disabled() -> Self {
        Self {
            max_age_ms: None,
            future_tolerance_ms: None,
        }
    }

    /// Create the production-profile recommended policy.
    pub fn production_recommended() -> Self {
        Self {
            max_age_ms: Some(60 * 60 * 1000),
            future_tolerance_ms: Some(5 * 60 * 1000),
        }
    }
}

/// Errors produced while creating, parsing, or verifying v1 peer bindings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerBindingError {
    /// Peer binding JSON value was not an object.
    NotObject,
    /// Required field was absent.
    MissingField {
        /// Field name.
        field: &'static str,
    },
    /// Field was present but had the wrong JSON type.
    InvalidFieldType {
        /// Field name.
        field: &'static str,
        /// Expected JSON type.
        expected: &'static str,
    },
    /// `type` was unsupported.
    UnsupportedType {
        /// Actual `type` value.
        actual: String,
    },
    /// `wallet_signature_scheme` was unsupported.
    UnsupportedWalletSignatureScheme {
        /// Actual signature-scheme value.
        actual: String,
    },
    /// A base64url field was malformed or decoded to the wrong length.
    InvalidBase64Url {
        /// Field name.
        field: &'static str,
        /// Base64url decoding error.
        error: Base64UrlError,
    },
    /// Timestamp was not an RFC3339 UTC string with `Z` suffix.
    InvalidTimestamp {
        /// Field name.
        field: &'static str,
        /// Actual timestamp value.
        value: String,
    },
    /// `peer_id` could not be parsed by libp2p.
    InvalidPeerId {
        /// Actual peer id text.
        peer_id: String,
    },
    /// Signature verification failed.
    InvalidSignature,
    /// Signed peer id did not match the transport-authenticated peer id.
    PeerIdMismatch {
        /// Peer id claimed by the binding.
        claimed: Box<PeerId>,
        /// Transport-authenticated peer id.
        authenticated: Box<PeerId>,
    },
}

/// Errors produced while applying local freshness policy to a verified binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerBindingFreshnessError {
    /// Verified binding carried an invalid `issued_at` timestamp.
    InvalidIssuedAtTimestamp {
        /// Actual `issued_at` value.
        issued_at: String,
    },
    /// Local `now` argument was not an RFC3339 UTC string with `Z` suffix.
    InvalidNowTimestamp {
        /// Actual `now` value.
        now: String,
    },
    /// Binding was older than the local maximum age.
    BindingTooOld {
        /// Binding `issued_at` value.
        issued_at: String,
        /// Local verification time.
        now: String,
        /// Actual binding age in milliseconds.
        age_ms: u64,
        /// Maximum accepted age in milliseconds.
        max_age_ms: u64,
    },
    /// Binding was issued too far in the future under local clock-skew policy.
    BindingFromFuture {
        /// Binding `issued_at` value.
        issued_at: String,
        /// Local verification time.
        now: String,
        /// Actual future skew in milliseconds.
        future_ms: u64,
        /// Maximum accepted future skew in milliseconds.
        future_tolerance_ms: u64,
    },
}

impl PeerBindingFreshnessError {
    /// Stable RFC failure code for this freshness error.
    pub fn failure_code(&self) -> &'static str {
        match self {
            Self::BindingTooOld { .. } => error::IDENTITY_BINDING_TOO_OLD,
            Self::BindingFromFuture { .. } => error::IDENTITY_BINDING_FROM_FUTURE,
            Self::InvalidIssuedAtTimestamp { .. } | Self::InvalidNowTimestamp { .. } => {
                error::IDENTITY_INVALID_PEER_BINDING
            }
        }
    }
}

impl fmt::Display for PeerBindingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotObject => write!(f, "peer binding is not a json object"),
            Self::MissingField { field } => write!(f, "peer binding missing field {field}"),
            Self::InvalidFieldType { field, expected } => {
                write!(f, "peer binding field {field} is not {expected}")
            }
            Self::UnsupportedType { actual } => {
                write!(f, "unsupported peer binding type {actual}")
            }
            Self::UnsupportedWalletSignatureScheme { actual } => {
                write!(f, "unsupported wallet signature scheme {actual}")
            }
            Self::InvalidBase64Url { field, error } => {
                write!(f, "invalid base64url in field {field}: {error}")
            }
            Self::InvalidTimestamp { field, value } => {
                write!(f, "invalid timestamp in field {field}: {value}")
            }
            Self::InvalidPeerId { peer_id } => {
                write!(f, "invalid libp2p peer id {peer_id}")
            }
            Self::InvalidSignature => write!(f, "peer binding signature does not verify"),
            Self::PeerIdMismatch {
                claimed,
                authenticated,
            } => {
                write!(
                    f,
                    "peer binding peer id {claimed} does not match authenticated peer id {authenticated}"
                )
            }
        }
    }
}

impl std::error::Error for PeerBindingError {}

impl fmt::Display for PeerBindingFreshnessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidIssuedAtTimestamp { issued_at } => {
                write!(f, "invalid peer binding issued_at timestamp {issued_at}")
            }
            Self::InvalidNowTimestamp { now } => {
                write!(f, "invalid peer binding freshness now timestamp {now}")
            }
            Self::BindingTooOld {
                issued_at,
                now,
                age_ms,
                max_age_ms,
            } => write!(
                f,
                "peer binding issued at {issued_at} is too old at {now}: {age_ms}ms > {max_age_ms}ms"
            ),
            Self::BindingFromFuture {
                issued_at,
                now,
                future_ms,
                future_tolerance_ms,
            } => write!(
                f,
                "peer binding issued at {issued_at} is too far in the future at {now}: {future_ms}ms > {future_tolerance_ms}ms"
            ),
        }
    }
}

impl std::error::Error for PeerBindingFreshnessError {}

impl VerifiedPeerBinding {
    /// Apply local freshness policy to this already verified binding.
    pub fn validate_freshness(
        &self,
        now: &str,
        policy: PeerBindingFreshnessPolicy,
    ) -> Result<(), PeerBindingFreshnessError> {
        let issued_at_ms = parse_rfc3339_z_timestamp_millis(&self.issued_at).ok_or_else(|| {
            PeerBindingFreshnessError::InvalidIssuedAtTimestamp {
                issued_at: self.issued_at.clone(),
            }
        })?;
        let now_ms = parse_rfc3339_z_timestamp_millis(now).ok_or_else(|| {
            PeerBindingFreshnessError::InvalidNowTimestamp {
                now: now.to_owned(),
            }
        })?;

        if issued_at_ms > now_ms {
            let future_ms = saturating_u64(issued_at_ms - now_ms);
            if let Some(future_tolerance_ms) = policy.future_tolerance_ms
                && future_ms > future_tolerance_ms
            {
                return Err(PeerBindingFreshnessError::BindingFromFuture {
                    issued_at: self.issued_at.clone(),
                    now: now.to_owned(),
                    future_ms,
                    future_tolerance_ms,
                });
            }
            return Ok(());
        }

        let age_ms = saturating_u64(now_ms - issued_at_ms);
        if let Some(max_age_ms) = policy.max_age_ms
            && age_ms > max_age_ms
        {
            return Err(PeerBindingFreshnessError::BindingTooOld {
                issued_at: self.issued_at.clone(),
                now: now.to_owned(),
                age_ms,
                max_age_ms,
            });
        }

        Ok(())
    }
}

impl PeerBinding {
    /// Create and sign a v1 peer binding.
    pub fn create(
        wallet: &Wallet,
        peer_id: &PeerId,
        issued_at: &str,
        label: Option<&str>,
    ) -> Result<Self, PeerBindingError> {
        validate_rfc3339_z_timestamp(FIELD_ISSUED_AT, issued_at)?;

        let mut object = Map::new();
        object.insert(
            FIELD_TYPE.to_owned(),
            Value::String(PEER_BINDING_TYPE.to_owned()),
        );
        object.insert(
            FIELD_WALLET_SIGNATURE_SCHEME.to_owned(),
            Value::String(WALLET_SIGNATURE_SCHEME_ED25519.to_owned()),
        );
        object.insert(
            FIELD_WALLET_PUBLIC_KEY.to_owned(),
            Value::String(base64url::encode(&wallet.public_key().0)),
        );
        object.insert(FIELD_PEER_ID.to_owned(), Value::String(peer_id.to_string()));
        object.insert(
            FIELD_ISSUED_AT.to_owned(),
            Value::String(issued_at.to_owned()),
        );
        if let Some(label) = label {
            object.insert(FIELD_LABEL.to_owned(), Value::String(label.to_owned()));
        }

        let signed_value = Value::Object(object.clone());
        let signed_bytes = auki_jcs::canonicalize(&signed_value);
        let signature = wallet.sign(&signed_bytes);
        object.insert(
            FIELD_SIGNATURE.to_owned(),
            Value::String(base64url::encode(&signature.0)),
        );

        Self::from_value(Value::Object(object))
    }

    /// Parse a v1 peer binding from a JSON value and validate its shape.
    pub fn from_value(value: Value) -> Result<Self, PeerBindingError> {
        let binding = Self { value };
        binding.validate_shape()?;
        Ok(binding)
    }

    /// Borrow the original JSON object, including fields unknown to this crate.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Consume this binding and return the original JSON object.
    pub fn into_value(self) -> Value {
        self.value
    }

    /// Return the signed peer id string.
    pub fn peer_id_str(&self) -> Result<&str, PeerBindingError> {
        required_string(self.object()?, FIELD_PEER_ID)
    }

    /// Return the signed `issued_at` timestamp string.
    pub fn issued_at(&self) -> Result<&str, PeerBindingError> {
        required_string(self.object()?, FIELD_ISSUED_AT)
    }

    /// Return the optional label string.
    pub fn label(&self) -> Result<Option<&str>, PeerBindingError> {
        optional_string(self.object()?, FIELD_LABEL)
    }

    /// Recompute signed bytes for this binding using RFC-0003 rules.
    pub fn signed_bytes(&self) -> Result<Vec<u8>, PeerBindingError> {
        Ok(auki_jcs::canonicalize(&self.signed_value()?))
    }

    /// Verify this binding against the transport-authenticated libp2p peer id.
    pub fn verify_for_peer_id(
        &self,
        authenticated_peer_id: &PeerId,
    ) -> Result<VerifiedPeerBinding, PeerBindingError> {
        self.validate_shape()?;

        let object = self.object()?;
        let wallet_public_key = decode_wallet_public_key(object)?;
        let signature = decode_signature(object)?;
        let signed_bytes = self.signed_bytes()?;

        verify(&wallet_public_key, &signed_bytes, &signature).map_err(map_verify_error)?;

        let claimed_peer_id = parse_peer_id(required_string(object, FIELD_PEER_ID)?)?;
        if claimed_peer_id != *authenticated_peer_id {
            return Err(PeerBindingError::PeerIdMismatch {
                claimed: Box::new(claimed_peer_id),
                authenticated: Box::new(*authenticated_peer_id),
            });
        }

        Ok(VerifiedPeerBinding {
            wallet_public_key,
            peer_id: claimed_peer_id,
            issued_at: required_string(object, FIELD_ISSUED_AT)?.to_owned(),
            label: optional_string(object, FIELD_LABEL)?.map(ToOwned::to_owned),
        })
    }

    fn validate_shape(&self) -> Result<(), PeerBindingError> {
        let object = self.object()?;

        let type_value = required_string(object, FIELD_TYPE)?;
        if type_value != PEER_BINDING_TYPE {
            return Err(PeerBindingError::UnsupportedType {
                actual: type_value.to_owned(),
            });
        }

        let scheme = required_string(object, FIELD_WALLET_SIGNATURE_SCHEME)?;
        if scheme != WALLET_SIGNATURE_SCHEME_ED25519 {
            return Err(PeerBindingError::UnsupportedWalletSignatureScheme {
                actual: scheme.to_owned(),
            });
        }

        decode_wallet_public_key(object)?;
        parse_peer_id(required_string(object, FIELD_PEER_ID)?)?;
        validate_rfc3339_z_timestamp(FIELD_ISSUED_AT, required_string(object, FIELD_ISSUED_AT)?)?;
        optional_string(object, FIELD_LABEL)?;
        decode_signature(object)?;

        Ok(())
    }

    fn object(&self) -> Result<&Map<String, Value>, PeerBindingError> {
        self.value.as_object().ok_or(PeerBindingError::NotObject)
    }

    fn signed_value(&self) -> Result<Value, PeerBindingError> {
        let mut object = self.object()?.clone();
        object
            .remove(FIELD_SIGNATURE)
            .ok_or(PeerBindingError::MissingField {
                field: FIELD_SIGNATURE,
            })?;
        Ok(Value::Object(object))
    }
}

fn decode_wallet_public_key(
    object: &Map<String, Value>,
) -> Result<WalletPublicKey, PeerBindingError> {
    let value = required_string(object, FIELD_WALLET_PUBLIC_KEY)?;
    base64url::decode_exact::<32>(value)
        .map(WalletPublicKey)
        .map_err(|error| PeerBindingError::InvalidBase64Url {
            field: FIELD_WALLET_PUBLIC_KEY,
            error,
        })
}

fn decode_signature(object: &Map<String, Value>) -> Result<Signature, PeerBindingError> {
    let value = required_string(object, FIELD_SIGNATURE)?;
    base64url::decode_exact::<64>(value)
        .map(Signature)
        .map_err(|error| PeerBindingError::InvalidBase64Url {
            field: FIELD_SIGNATURE,
            error,
        })
}

fn required_string<'a>(
    object: &'a Map<String, Value>,
    field: &'static str,
) -> Result<&'a str, PeerBindingError> {
    object
        .get(field)
        .ok_or(PeerBindingError::MissingField { field })?
        .as_str()
        .ok_or(PeerBindingError::InvalidFieldType {
            field,
            expected: "a string",
        })
}

fn optional_string<'a>(
    object: &'a Map<String, Value>,
    field: &'static str,
) -> Result<Option<&'a str>, PeerBindingError> {
    object
        .get(field)
        .map(|value| {
            value.as_str().ok_or(PeerBindingError::InvalidFieldType {
                field,
                expected: "a string",
            })
        })
        .transpose()
}

fn parse_peer_id(peer_id: &str) -> Result<PeerId, PeerBindingError> {
    PeerId::from_str(peer_id).map_err(|_| PeerBindingError::InvalidPeerId {
        peer_id: peer_id.to_owned(),
    })
}

fn map_verify_error(_error: VerifyError) -> PeerBindingError {
    PeerBindingError::InvalidSignature
}

fn validate_rfc3339_z_timestamp(field: &'static str, value: &str) -> Result<(), PeerBindingError> {
    if is_rfc3339_z_timestamp(value) {
        Ok(())
    } else {
        Err(PeerBindingError::InvalidTimestamp {
            field,
            value: value.to_owned(),
        })
    }
}

fn is_rfc3339_z_timestamp(value: &str) -> bool {
    let Some(value) = value.strip_suffix('Z') else {
        return false;
    };
    let Some((date, time)) = value.split_once('T') else {
        return false;
    };

    let Some((year, month, day)) = parse_date(date) else {
        return false;
    };
    if year > 9999 || month == 0 || month > 12 {
        return false;
    }
    if day == 0 || day > days_in_month(year, month) {
        return false;
    }

    let Some((hour, minute, second)) = parse_time(time) else {
        return false;
    };
    hour <= 23 && minute <= 59 && second <= 60
}

fn parse_rfc3339_z_timestamp_millis(value: &str) -> Option<i128> {
    let value = value.strip_suffix('Z')?;
    let (date, time) = value.split_once('T')?;
    let (year, month, day) = parse_date(date)?;
    if year > 9999 || month == 0 || month > 12 {
        return None;
    }
    if day == 0 || day > days_in_month(year, month) {
        return None;
    }

    let (hour, minute, second, fraction_ms) = parse_time_millis(time)?;
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }

    let days = days_from_civil(year, month, day);
    let seconds =
        days * 86_400 + i128::from(hour) * 3_600 + i128::from(minute) * 60 + i128::from(second);
    Some(seconds * 1000 + i128::from(fraction_ms))
}

fn parse_time_millis(value: &str) -> Option<(u32, u32, u32, u32)> {
    let (base, fraction) = match value.split_once('.') {
        Some((base, fraction)) => {
            if fraction.is_empty() || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            (base, Some(fraction))
        }
        None => (value, None),
    };

    if base.len() != 8 {
        return None;
    }
    if base.as_bytes().get(2) != Some(&b':') || base.as_bytes().get(5) != Some(&b':') {
        return None;
    }
    let hour = parse_fixed_digits(&base[0..2])?;
    let minute = parse_fixed_digits(&base[3..5])?;
    let second = parse_fixed_digits(&base[6..8])?;
    Some((
        hour,
        minute,
        second,
        fraction_millis(fraction.unwrap_or("")),
    ))
}

fn fraction_millis(fraction: &str) -> u32 {
    let mut millis = 0;
    for index in 0..3 {
        millis *= 10;
        if let Some(byte) = fraction.as_bytes().get(index) {
            millis += u32::from(byte - b'0');
        }
    }
    millis
}

fn days_from_civil(year: u32, month: u32, day: u32) -> i128 {
    let year = i128::from(year) - if month <= 2 { 1 } else { 0 };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month = i128::from(month);
    let day = i128::from(day);
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn saturating_u64(value: i128) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn parse_date(value: &str) -> Option<(u32, u32, u32)> {
    if value.len() != 10 {
        return None;
    }
    if value.as_bytes().get(4) != Some(&b'-') || value.as_bytes().get(7) != Some(&b'-') {
        return None;
    }
    let year = parse_fixed_digits(&value[0..4])?;
    let month = parse_fixed_digits(&value[5..7])?;
    let day = parse_fixed_digits(&value[8..10])?;
    Some((year, month, day))
}

fn parse_time(value: &str) -> Option<(u32, u32, u32)> {
    let (base, fraction) = match value.split_once('.') {
        Some((base, fraction)) => {
            if fraction.is_empty() || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            (base, Some(fraction))
        }
        None => (value, None),
    };

    if base.len() != 8 {
        return None;
    }
    if base.as_bytes().get(2) != Some(&b':') || base.as_bytes().get(5) != Some(&b':') {
        return None;
    }
    let hour = parse_fixed_digits(&base[0..2])?;
    let minute = parse_fixed_digits(&base[3..5])?;
    let second = parse_fixed_digits(&base[6..8])?;
    let _ = fraction;
    Some((hour, minute, second))
}

fn parse_fixed_digits(value: &str) -> Option<u32> {
    if value.bytes().all(|byte| byte.is_ascii_digit()) {
        value.parse().ok()
    } else {
        None
    }
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: u32) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const PEER_ID: &str = "12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar";
    const OTHER_PEER_ID: &str = "12D3KooWFU1bqozGMWdqN2Ckh2YHNbr9n5Lypw6uJrNkbm2ptVbF";
    const ISSUED_AT: &str = "2026-05-26T12:00:00Z";

    fn wallet() -> std::sync::Arc<Wallet> {
        Wallet::from_seed(vec![3u8; 32]).expect("32-byte seed")
    }

    fn peer_id() -> PeerId {
        PeerId::from_str(PEER_ID).expect("valid peer id")
    }

    fn other_peer_id() -> PeerId {
        PeerId::from_str(OTHER_PEER_ID).expect("valid peer id")
    }

    fn signed_binding_value(extra: Option<(&str, Value)>) -> Value {
        let wallet = wallet();
        let mut object = Map::new();
        object.insert(
            FIELD_TYPE.to_owned(),
            Value::String(PEER_BINDING_TYPE.to_owned()),
        );
        object.insert(
            FIELD_WALLET_SIGNATURE_SCHEME.to_owned(),
            Value::String(WALLET_SIGNATURE_SCHEME_ED25519.to_owned()),
        );
        object.insert(
            FIELD_WALLET_PUBLIC_KEY.to_owned(),
            Value::String(base64url::encode(&wallet.public_key().0)),
        );
        object.insert(FIELD_PEER_ID.to_owned(), Value::String(PEER_ID.to_owned()));
        object.insert(
            FIELD_ISSUED_AT.to_owned(),
            Value::String(ISSUED_AT.to_owned()),
        );
        if let Some((field, value)) = extra {
            object.insert(field.to_owned(), value);
        }

        let signed_value = Value::Object(object.clone());
        let signed_bytes = auki_jcs::canonicalize(&signed_value);
        object.insert(
            FIELD_SIGNATURE.to_owned(),
            Value::String(base64url::encode(&wallet.sign(&signed_bytes).0)),
        );
        Value::Object(object)
    }

    #[test]
    fn create_and_verify_peer_binding() {
        let wallet = wallet();
        let binding = PeerBinding::create(&wallet, &peer_id(), ISSUED_AT, Some("robot-a")).unwrap();
        let verified = binding.verify_for_peer_id(&peer_id()).unwrap();

        assert_eq!(verified.wallet_public_key, wallet.public_key());
        assert_eq!(verified.peer_id, peer_id());
        assert_eq!(verified.issued_at, ISSUED_AT);
        assert_eq!(verified.label.as_deref(), Some("robot-a"));
        assert_eq!(
            binding.value()[FIELD_TYPE],
            Value::String(PEER_BINDING_TYPE.to_owned())
        );
    }

    #[test]
    fn verify_accepts_unknown_fields_when_they_were_signed() {
        let binding = PeerBinding::from_value(signed_binding_value(Some((
            "unknown_extension",
            json!({"kept": true}),
        ))))
        .unwrap();

        assert!(binding.verify_for_peer_id(&peer_id()).is_ok());
    }

    #[test]
    fn verify_rejects_unknown_field_added_after_signing() {
        let mut binding = PeerBinding::create(&wallet(), &peer_id(), ISSUED_AT, None)
            .unwrap()
            .into_value();
        binding
            .as_object_mut()
            .unwrap()
            .insert("unknown_extension".to_owned(), json!({"tampered": true}));
        let binding = PeerBinding::from_value(binding).unwrap();

        assert_eq!(
            binding.verify_for_peer_id(&peer_id()),
            Err(PeerBindingError::InvalidSignature)
        );
    }

    #[test]
    fn verify_rejects_peer_id_mismatch() {
        let binding = PeerBinding::create(&wallet(), &peer_id(), ISSUED_AT, None).unwrap();

        assert_eq!(
            binding.verify_for_peer_id(&other_peer_id()),
            Err(PeerBindingError::PeerIdMismatch {
                claimed: Box::new(peer_id()),
                authenticated: Box::new(other_peer_id())
            })
        );
    }

    #[test]
    fn from_value_rejects_missing_required_field() {
        assert_eq!(
            PeerBinding::from_value(json!({})),
            Err(PeerBindingError::MissingField { field: FIELD_TYPE })
        );
    }

    #[test]
    fn from_value_rejects_unsupported_type() {
        let mut value = signed_binding_value(None);
        value.as_object_mut().unwrap().insert(
            FIELD_TYPE.to_owned(),
            Value::String("auki.other_binding.v1".to_owned()),
        );

        assert_eq!(
            PeerBinding::from_value(value),
            Err(PeerBindingError::UnsupportedType {
                actual: "auki.other_binding.v1".to_owned()
            })
        );
    }

    #[test]
    fn from_value_rejects_malformed_wallet_public_key() {
        let mut value = signed_binding_value(None);
        value.as_object_mut().unwrap().insert(
            FIELD_WALLET_PUBLIC_KEY.to_owned(),
            Value::String("Zg=".to_owned()),
        );

        assert!(matches!(
            PeerBinding::from_value(value),
            Err(PeerBindingError::InvalidBase64Url {
                field: FIELD_WALLET_PUBLIC_KEY,
                ..
            })
        ));
    }

    #[test]
    fn from_value_rejects_malformed_peer_id() {
        let mut value = signed_binding_value(None);
        value.as_object_mut().unwrap().insert(
            FIELD_PEER_ID.to_owned(),
            Value::String("not-a-peer-id".to_owned()),
        );

        assert_eq!(
            PeerBinding::from_value(value),
            Err(PeerBindingError::InvalidPeerId {
                peer_id: "not-a-peer-id".to_owned()
            })
        );
    }

    #[test]
    fn from_value_rejects_malformed_issued_at() {
        let mut value = signed_binding_value(None);
        value.as_object_mut().unwrap().insert(
            FIELD_ISSUED_AT.to_owned(),
            Value::String("2026-05-26T12:00:00+07:00".to_owned()),
        );

        assert_eq!(
            PeerBinding::from_value(value),
            Err(PeerBindingError::InvalidTimestamp {
                field: FIELD_ISSUED_AT,
                value: "2026-05-26T12:00:00+07:00".to_owned()
            })
        );
    }

    #[test]
    fn create_rejects_malformed_issued_at() {
        assert_eq!(
            PeerBinding::create(&wallet(), &peer_id(), "2026-02-29T12:00:00Z", None),
            Err(PeerBindingError::InvalidTimestamp {
                field: FIELD_ISSUED_AT,
                value: "2026-02-29T12:00:00Z".to_owned()
            })
        );
    }

    #[test]
    fn verify_rejects_tampered_signature_field() {
        let mut value = signed_binding_value(None);
        value.as_object_mut().unwrap().insert(
            FIELD_SIGNATURE.to_owned(),
            Value::String(base64url::encode(&[0u8; 64])),
        );
        let binding = PeerBinding::from_value(value).unwrap();

        assert_eq!(
            binding.verify_for_peer_id(&peer_id()),
            Err(PeerBindingError::InvalidSignature)
        );
    }

    #[test]
    fn signed_bytes_remove_only_signature_field() {
        let binding =
            PeerBinding::from_value(signed_binding_value(Some(("z_extra", json!("included")))))
                .unwrap();
        let signed: Value = serde_json::from_slice(&binding.signed_bytes().unwrap()).unwrap();

        assert!(signed.get(FIELD_SIGNATURE).is_none());
        assert_eq!(signed["z_extra"], json!("included"));
    }

    #[test]
    fn timestamp_validator_accepts_fractional_seconds_and_leap_year() {
        assert!(is_rfc3339_z_timestamp("2024-02-29T00:00:00.123Z"));
    }

    #[test]
    fn peer_binding_freshness_accepts_policy_window() {
        let binding = PeerBinding::create(&wallet(), &peer_id(), ISSUED_AT, None).unwrap();
        let verified = binding.verify_for_peer_id(&peer_id()).unwrap();
        let policy = PeerBindingFreshnessPolicy::production_recommended();

        verified
            .validate_freshness("2026-05-26T13:00:00Z", policy)
            .unwrap();
        verified
            .validate_freshness("2026-05-26T11:55:00Z", policy)
            .unwrap();
    }

    #[test]
    fn peer_binding_freshness_rejects_old_or_future_binding() {
        let binding = PeerBinding::create(&wallet(), &peer_id(), ISSUED_AT, None).unwrap();
        let verified = binding.verify_for_peer_id(&peer_id()).unwrap();
        let policy = PeerBindingFreshnessPolicy::production_recommended();

        let too_old = verified
            .validate_freshness("2026-05-26T13:00:01Z", policy)
            .unwrap_err();
        assert_eq!(
            too_old,
            PeerBindingFreshnessError::BindingTooOld {
                issued_at: ISSUED_AT.to_owned(),
                now: "2026-05-26T13:00:01Z".to_owned(),
                age_ms: 3_601_000,
                max_age_ms: 3_600_000,
            }
        );
        assert_eq!(too_old.failure_code(), error::IDENTITY_BINDING_TOO_OLD);

        let from_future = verified
            .validate_freshness("2026-05-26T11:54:59Z", policy)
            .unwrap_err();
        assert_eq!(
            from_future,
            PeerBindingFreshnessError::BindingFromFuture {
                issued_at: ISSUED_AT.to_owned(),
                now: "2026-05-26T11:54:59Z".to_owned(),
                future_ms: 301_000,
                future_tolerance_ms: 300_000,
            }
        );
        assert_eq!(
            from_future.failure_code(),
            error::IDENTITY_BINDING_FROM_FUTURE
        );
    }
}
