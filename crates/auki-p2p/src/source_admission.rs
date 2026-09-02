use std::{collections::HashMap, fmt, sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use futures::{
    future::{select, Either},
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    lock::Mutex as AsyncMutex,
    pin_mut, Future,
};
use futures_timer::Delay;
use libp2p::{swarm::ConnectionId, PeerId, StreamProtocol};
use parking_lot::Mutex;
use serde::{
    de::{self, MapAccess, Visitor},
    ser::SerializeStruct,
    Deserialize, Deserializer, Serialize, Serializer,
};
use uuid::Uuid;
use web_time::Instant;

use crate::{
    token::{ensure_token_peer, DdsTokenVerifier, SignedP2pCredential, TokenStore},
    Error, Result,
};

pub(crate) const PROTOCOL: StreamProtocol = StreamProtocol::new("/auki-p2p/relay-auth/1");
pub(crate) const REQUEST_MAX_BYTES: usize = 64 * 1024;
pub(crate) const RESPONSE_MAX_BYTES: usize = 4 * 1024;
pub(crate) const TIMEOUT: Duration = Duration::from_secs(10);
pub(crate) const MAX_ACCEPTED_TTL: Duration = Duration::from_secs(30);
const REUSE_SAFETY_MARGIN: chrono::TimeDelta = chrono::TimeDelta::seconds(1);

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct CacheKey {
    relay_peer_id: PeerId,
    relay_connection: ConnectionId,
    target_peer_id: PeerId,
    domain_id: Uuid,
    credential_issued_at: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct CachedAdmission {
    pub(crate) expires_at: DateTime<Utc>,
    lease_id: u64,
    credential_issued_at: u64,
}

impl CachedAdmission {
    pub(crate) fn uncached(expires_at: DateTime<Utc>) -> Self {
        Self {
            expires_at,
            lease_id: 0,
            credential_issued_at: 0,
        }
    }
}

/// Single-flight cache matching the relay's short-lived source admission key.
///
/// One admitted source may open multiple circuits until `accepted_until`.
/// Serializing cache misses prevents a composed protocol flow from repeating
/// the same JWT verification for every application substream.
#[derive(Default)]
pub(crate) struct AdmissionCache {
    entries: Mutex<HashMap<CacheKey, Arc<AdmissionEntry>>>,
    next_lease_id: Mutex<u64>,
}

#[derive(Default)]
struct AdmissionEntry {
    gate: AsyncMutex<()>,
    cached: Mutex<Option<AdmissionLease>>,
}

#[derive(Clone, Copy)]
struct AdmissionLease {
    expires_at: DateTime<Utc>,
    reuse_until: Instant,
    lease_id: u64,
}

impl AdmissionCache {
    pub(crate) async fn authorize<F, Fut>(
        &self,
        relay_peer_id: PeerId,
        relay_connection: ConnectionId,
        target_peer_id: PeerId,
        domain_id: Uuid,
        credential_issued_at: u64,
        authorize: F,
    ) -> Result<CachedAdmission>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<DateTime<Utc>>>,
    {
        let key = CacheKey {
            relay_peer_id,
            relay_connection,
            target_peer_id,
            domain_id,
            credential_issued_at,
        };
        let wall_now = Utc::now();
        let monotonic_now = Instant::now();
        let entry = {
            let mut entries = self.entries.lock();
            entries.retain(|_, entry| {
                Arc::strong_count(entry) > 1
                    || entry
                        .cached
                        .lock()
                        .is_some_and(|lease| lease.reusable(wall_now, monotonic_now))
            });
            Arc::clone(entries.entry(key).or_default())
        };
        let _gate = entry.gate.lock().await;
        if let Some(lease) = *entry.cached.lock() {
            if lease.reusable(Utc::now(), Instant::now()) {
                return Ok(CachedAdmission {
                    expires_at: lease.expires_at,
                    lease_id: lease.lease_id,
                    credential_issued_at,
                });
            }
        }

        *entry.cached.lock() = None;
        let expires_at = authorize().await?;
        let lease_id = {
            let mut next_lease_id = self.next_lease_id.lock();
            *next_lease_id = next_lease_id.wrapping_add(1).max(1);
            *next_lease_id
        };
        if let Some(reuse_until) = reuse_deadline(expires_at, Utc::now(), Instant::now()) {
            *entry.cached.lock() = Some(AdmissionLease {
                expires_at,
                reuse_until,
                lease_id,
            });
        }
        Ok(CachedAdmission {
            expires_at,
            lease_id,
            credential_issued_at,
        })
    }

    pub(crate) async fn invalidate(
        &self,
        relay_peer_id: PeerId,
        relay_connection: ConnectionId,
        target_peer_id: PeerId,
        domain_id: Uuid,
        admission: &CachedAdmission,
    ) {
        let key = CacheKey {
            relay_peer_id,
            relay_connection,
            target_peer_id,
            domain_id,
            credential_issued_at: admission.credential_issued_at,
        };
        let entry = self.entries.lock().get(&key).cloned();
        if let Some(entry) = entry {
            let _gate = entry.gate.lock().await;
            let mut cached = entry.cached.lock();
            if cached
                .as_ref()
                .is_some_and(|lease| lease.lease_id == admission.lease_id)
            {
                *cached = None;
            }
        }
    }
}

impl AdmissionLease {
    fn reusable(&self, wall_now: DateTime<Utc>, monotonic_now: Instant) -> bool {
        self.expires_at > wall_now + REUSE_SAFETY_MARGIN && monotonic_now < self.reuse_until
    }
}

fn reuse_deadline(
    expires_at: DateTime<Utc>,
    wall_now: DateTime<Utc>,
    monotonic_now: Instant,
) -> Option<Instant> {
    let remaining = expires_at.signed_duration_since(wall_now).to_std().ok()?;
    let reusable_for = remaining.checked_sub(REUSE_SAFETY_MARGIN.to_std().ok()?)?;
    Some(monotonic_now + reusable_for)
}

pub(crate) struct Request<'a> {
    pub(crate) domain_id: Uuid,
    pub(crate) target_peer_id: PeerId,
    pub(crate) p2p_access_token: &'a str,
}

impl fmt::Debug for Request<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Request")
            .field("version", &1)
            .field("domain_id", &self.domain_id)
            .field("target_peer_id", &self.target_peer_id)
            .field("p2p_access_token", &"[REDACTED]")
            .finish()
    }
}

impl Serialize for Request<'_> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("RelayAdmissionRequest", 4)?;
        state.serialize_field("version", &1_u8)?;
        state.serialize_field("domain_id", &self.domain_id.to_string())?;
        state.serialize_field("target_peer_id", &self.target_peer_id.to_string())?;
        state.serialize_field("p2p_access_token", self.p2p_access_token)?;
        state.end()
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Response {
    accepted: bool,
    accepted_until: Option<String>,
}

impl<'de> Deserialize<'de> for Response {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ResponseVisitor;

        impl<'de> Visitor<'de> for ResponseVisitor {
            type Value = Response;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an exact relay admission response object")
            }

            fn visit_map<A>(self, mut map: A) -> std::result::Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut accepted = None;
                let mut accepted_until = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "accepted" => {
                            if accepted.is_some() {
                                return Err(de::Error::duplicate_field("accepted"));
                            }
                            accepted = Some(map.next_value::<bool>()?);
                        }
                        "accepted_until" => {
                            if accepted_until.is_some() {
                                return Err(de::Error::duplicate_field("accepted_until"));
                            }
                            accepted_until = Some(map.next_value::<String>()?);
                        }
                        _ => return Err(de::Error::unknown_field(&key, RESPONSE_FIELDS)),
                    }
                }
                let accepted = accepted.ok_or_else(|| de::Error::missing_field("accepted"))?;
                match (accepted, accepted_until) {
                    (true, Some(accepted_until)) => Ok(Response {
                        accepted,
                        accepted_until: Some(accepted_until),
                    }),
                    (false, None) => Ok(Response {
                        accepted,
                        accepted_until: None,
                    }),
                    _ => Err(de::Error::custom(
                        "accepted_until is required only for accepted responses",
                    )),
                }
            }
        }

        deserializer.deserialize_map(ResponseVisitor)
    }
}

const RESPONSE_FIELDS: &[&str] = &["accepted", "accepted_until"];

pub(crate) async fn authorize<S, F>(
    stream: &mut S,
    request: Request<'_>,
    now: F,
) -> Result<DateTime<Utc>>
where
    S: AsyncRead + AsyncWrite + Unpin,
    F: FnOnce() -> DateTime<Utc>,
{
    let payload = serde_json::to_vec(&request).map_err(|_| Error::RelayAdmissionMalformed)?;
    timeout(TIMEOUT, write_frame(stream, &payload, REQUEST_MAX_BYTES)).await?;
    let response = timeout(TIMEOUT, async {
        let response = read_frame(stream, RESPONSE_MAX_BYTES).await?;
        require_eof(stream).await?;
        Ok(response)
    })
    .await?;
    let response = decode_response(&response)?;
    let now = now();
    if !response.accepted {
        return Err(Error::RelayAdmissionDenied);
    }
    let raw_deadline = response
        .accepted_until
        .ok_or(Error::RelayAdmissionMalformed)?;
    let parsed =
        DateTime::parse_from_rfc3339(&raw_deadline).map_err(|_| Error::RelayAdmissionMalformed)?;
    if parsed.offset().local_minus_utc() != 0 {
        return Err(Error::RelayAdmissionMalformed);
    }
    let accepted_until = parsed.with_timezone(&Utc);
    if now >= accepted_until {
        return Err(Error::RelayAdmissionExpired);
    }
    let maximum = now
        + chrono::Duration::from_std(MAX_ACCEPTED_TTL)
            .map_err(|_| Error::RelayAdmissionMalformed)?;
    if accepted_until > maximum {
        return Err(Error::RelayAdmissionMalformed);
    }
    Ok(accepted_until)
}

pub(crate) struct PreparedAuthorization {
    domain_id: Uuid,
    target_peer_id: PeerId,
    token: SignedP2pCredential,
    token_expiration: DateTime<Utc>,
    issued_at: u64,
}

impl PreparedAuthorization {
    pub(crate) fn issued_at(&self) -> u64 {
        self.issued_at
    }
}

pub(crate) async fn prepare_authorization(
    local_peer_id: PeerId,
    target_peer_id: PeerId,
    domain_id: Uuid,
    tokens: &TokenStore,
    verifier: &DdsTokenVerifier,
) -> Result<PreparedAuthorization> {
    let token = tokens.snapshot().await.ok_or(Error::MissingToken)?;
    let claims = verifier.verify(token.as_str())?;
    ensure_token_peer(&claims, local_peer_id)?;
    if !claims
        .domain_ids
        .iter()
        .filter_map(|domain| Uuid::parse_str(domain).ok())
        .any(|domain| domain == domain_id)
    {
        return Err(Error::RemoteDomainMismatch(domain_id.to_string()));
    }
    let token_expiration = i64::try_from(claims.exp)
        .ok()
        .and_then(|seconds| DateTime::<Utc>::from_timestamp(seconds, 0))
        .ok_or(Error::RelayAdmissionMalformed)?;

    Ok(PreparedAuthorization {
        domain_id,
        target_peer_id,
        token,
        token_expiration,
        issued_at: claims.iat,
    })
}

pub(crate) async fn authorize_prepared<S, F>(
    stream: &mut S,
    prepared: PreparedAuthorization,
    now: F,
) -> Result<DateTime<Utc>>
where
    S: AsyncRead + AsyncWrite + Unpin,
    F: FnOnce() -> DateTime<Utc>,
{
    let accepted_until = authorize(
        stream,
        Request {
            domain_id: prepared.domain_id,
            target_peer_id: prepared.target_peer_id,
            p2p_access_token: prepared.token.as_str(),
        },
        now,
    )
    .await?;
    if accepted_until > prepared.token_expiration {
        return Err(Error::RelayAdmissionMalformed);
    }
    Ok(accepted_until)
}

async fn timeout<T>(duration: Duration, future: impl Future<Output = Result<T>>) -> Result<T> {
    let delay = Delay::new(duration);
    pin_mut!(future, delay);
    match select(future, delay).await {
        Either::Left((result, _)) => result,
        Either::Right(((), _)) => Err(Error::RelayAdmissionTimeout),
    }
}

async fn write_frame<S>(stream: &mut S, payload: &[u8], maximum: usize) -> Result<()>
where
    S: AsyncWrite + Unpin,
{
    if payload.is_empty() || payload.len() > maximum {
        return Err(Error::RelayAdmissionFrameTooLarge { maximum });
    }
    let length =
        u32::try_from(payload.len()).map_err(|_| Error::RelayAdmissionFrameTooLarge { maximum })?;
    stream.write_all(&length.to_be_bytes()).await?;
    stream.write_all(payload).await?;
    stream.flush().await?;
    Ok(())
}

async fn read_frame<S>(stream: &mut S, maximum: usize) -> Result<Vec<u8>>
where
    S: AsyncRead + Unpin,
{
    let mut header = [0_u8; 4];
    stream.read_exact(&mut header).await?;
    let length = u32::from_be_bytes(header) as usize;
    if length == 0 || length > maximum {
        return Err(Error::RelayAdmissionFrameTooLarge { maximum });
    }
    let mut payload = vec![0; length];
    stream.read_exact(&mut payload).await?;
    Ok(payload)
}

async fn require_eof<S>(stream: &mut S) -> Result<()>
where
    S: AsyncRead + Unpin,
{
    let mut trailing = [0_u8; 1];
    match stream.read(&mut trailing).await? {
        0 => Ok(()),
        _ => Err(Error::RelayAdmissionMalformed),
    }
}

fn decode_response(payload: &[u8]) -> Result<Response> {
    std::str::from_utf8(payload).map_err(|_| Error::RelayAdmissionMalformed)?;
    let mut stream = serde_json::Deserializer::from_slice(payload).into_iter::<Response>();
    let response = stream
        .next()
        .ok_or(Error::RelayAdmissionMalformed)?
        .map_err(|_| Error::RelayAdmissionMalformed)?;
    if stream.byte_offset() != payload.len() {
        return Err(Error::RelayAdmissionMalformed);
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use std::{
        io,
        pin::Pin,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
        task::{Context, Poll},
    };

    use futures::{
        future::join,
        io::{AsyncRead, AsyncWrite, Cursor},
    };

    use super::*;

    const PEER_ID: &str = "12D3KooWBMyph6PCuP6GUJkwFdR7bLUPZ3exLvgEPpR93J52GaJg";
    const DOMAIN_ID: &str = "11111111-2222-3333-4444-555555555555";
    const TOKEN: &str = "header.payload.signature";
    const REQUEST_JSON: &str = "{\"version\":1,\"domain_id\":\"11111111-2222-3333-4444-555555555555\",\"target_peer_id\":\"12D3KooWBMyph6PCuP6GUJkwFdR7bLUPZ3exLvgEPpR93J52GaJg\",\"p2p_access_token\":\"header.payload.signature\"}";

    #[tokio::test]
    async fn admission_cache_single_flights_and_isolates_authority_keys() {
        let cache = AdmissionCache::default();
        let relay = PEER_ID.parse().unwrap();
        let target = PeerId::random();
        let domain = Uuid::parse_str(DOMAIN_ID).unwrap();
        let connection = ConnectionId::new_unchecked(7);
        let calls = Arc::new(AtomicUsize::new(0));
        let expires_at = Utc::now() + chrono::TimeDelta::seconds(20);

        let authorize = || {
            let calls = Arc::clone(&calls);
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Delay::new(Duration::from_millis(20)).await;
                Ok(expires_at)
            }
        };
        let (first, second) = join(
            cache.authorize(relay, connection, target, domain, 10, authorize),
            cache.authorize(relay, connection, target, domain, 10, authorize),
        )
        .await;
        let first = first.unwrap();
        second.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        cache
            .authorize(relay, connection, PeerId::random(), domain, 10, authorize)
            .await
            .unwrap();
        cache
            .authorize(
                relay,
                ConnectionId::new_unchecked(8),
                target,
                domain,
                10,
                authorize,
            )
            .await
            .unwrap();
        cache
            .authorize(relay, connection, target, domain, 11, authorize)
            .await
            .unwrap();
        cache
            .authorize(relay, connection, target, Uuid::new_v4(), 10, authorize)
            .await
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 5);

        cache
            .invalidate(relay, connection, target, domain, &first)
            .await;
        let replacement = cache
            .authorize(relay, connection, target, domain, 10, authorize)
            .await
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 6);

        cache
            .invalidate(relay, connection, target, domain, &first)
            .await;
        cache
            .authorize(relay, connection, target, domain, 10, authorize)
            .await
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 6);
        assert_ne!(first.lease_id, replacement.lease_id);
    }

    #[tokio::test]
    async fn admission_cache_does_not_reuse_a_near_expiry_grant() {
        let cache = AdmissionCache::default();
        let relay = PEER_ID.parse().unwrap();
        let target = PeerId::random();
        let domain = Uuid::parse_str(DOMAIN_ID).unwrap();
        let connection = ConnectionId::new_unchecked(10);
        let calls = Arc::new(AtomicUsize::new(0));

        for _ in 0..2 {
            cache
                .authorize(relay, connection, target, domain, 13, || {
                    let calls = Arc::clone(&calls);
                    async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        Ok(Utc::now() + chrono::TimeDelta::milliseconds(500))
                    }
                })
                .await
                .unwrap();
        }
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn admission_cache_does_not_serialize_different_keys_or_cache_failures() {
        let cache = AdmissionCache::default();
        let relay = PEER_ID.parse().unwrap();
        let domain = Uuid::parse_str(DOMAIN_ID).unwrap();
        let connection = ConnectionId::new_unchecked(9);
        let in_flight = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let expires_at = Utc::now() + chrono::TimeDelta::seconds(20);
        let authorize = || {
            let in_flight = Arc::clone(&in_flight);
            let maximum = Arc::clone(&maximum);
            async move {
                let active = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                maximum.fetch_max(active, Ordering::SeqCst);
                Delay::new(Duration::from_millis(20)).await;
                in_flight.fetch_sub(1, Ordering::SeqCst);
                Ok(expires_at)
            }
        };
        let (left, right) = join(
            cache.authorize(relay, connection, PeerId::random(), domain, 12, authorize),
            cache.authorize(relay, connection, PeerId::random(), domain, 12, authorize),
        )
        .await;
        left.unwrap();
        right.unwrap();
        assert_eq!(maximum.load(Ordering::SeqCst), 2);

        let failed_target = PeerId::random();
        assert!(cache
            .authorize(relay, connection, failed_target, domain, 12, || async {
                Err(Error::RelayAdmissionDenied)
            })
            .await
            .is_err());
        let attempts = Arc::new(AtomicUsize::new(0));
        cache
            .authorize(relay, connection, failed_target, domain, 12, || {
                let attempts = Arc::clone(&attempts);
                async move {
                    attempts.fetch_add(1, Ordering::SeqCst);
                    Ok(expires_at)
                }
            })
            .await
            .unwrap();
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn request_frame_vector_is_stable_and_redacted() {
        let request = request();
        let payload = serde_json::to_vec(&request).unwrap();
        assert_eq!(payload, REQUEST_JSON.as_bytes());
        assert_eq!((payload.len() as u32).to_be_bytes(), [0, 0, 0, 182]);
        let debug = format!("{request:?}");
        assert!(!debug.contains(TOKEN));
        assert!(debug.contains("[REDACTED]"));
    }

    #[test]
    fn response_decoder_is_exact() {
        let valid = br#"{"accepted":true,"accepted_until":"2030-01-02T03:04:05Z"}"#;
        assert_eq!(
            decode_response(valid).unwrap(),
            Response {
                accepted: true,
                accepted_until: Some("2030-01-02T03:04:05Z".into()),
            }
        );
        assert_eq!(
            decode_response(br#"{"accepted":false}"#).unwrap(),
            Response {
                accepted: false,
                accepted_until: None,
            }
        );

        for invalid in [
            br#"{}"#.as_slice(),
            br#"null"#,
            br#"{"accepted":null}"#,
            br#"{"Accepted":true,"accepted_until":"2030-01-02T03:04:05Z"}"#,
            br#"{"accepted":true,"accepted":false}"#,
            br#"{"accepted":true}"#,
            br#"{"accepted":false,"accepted_until":"2030-01-02T03:04:05Z"}"#,
            br#"{"accepted":true,"accepted_until":null}"#,
            br#"{"accepted":true,"accepted_until":"2030-01-02T03:04:05Z","extra":1}"#,
            br#"{"accepted":true,"accepted_until":"2030-01-02T03:04:05Z"} true"#,
            br#"{"accepted":true,"accepted_until":"2030-01-02T03:04:05Z"} "#,
            &[0xff],
        ] {
            assert!(
                decode_response(invalid).is_err(),
                "accepted invalid response: {invalid:?}"
            );
        }
    }

    #[tokio::test]
    async fn authorization_round_trip_enforces_denial_and_deadline() {
        let now = DateTime::parse_from_rfc3339("2030-01-02T03:04:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut accepted = ScriptedStream::with_response(frame(
            br#"{"accepted":true,"accepted_until":"2030-01-02T03:04:05Z"}"#,
        ));
        let deadline = authorize(&mut accepted, request(), || now).await.unwrap();
        assert_eq!(deadline.to_rfc3339(), "2030-01-02T03:04:05+00:00");
        assert_eq!(&accepted.written[4..], REQUEST_JSON.as_bytes());

        let mut denied = ScriptedStream::with_response(frame(br#"{"accepted":false}"#));
        assert!(matches!(
            authorize(&mut denied, request(), || now).await,
            Err(Error::RelayAdmissionDenied)
        ));

        let mut expired = ScriptedStream::with_response(frame(
            br#"{"accepted":true,"accepted_until":"2030-01-02T03:04:00Z"}"#,
        ));
        assert!(matches!(
            authorize(&mut expired, request(), || now).await,
            Err(Error::RelayAdmissionExpired)
        ));

        let mut overlong = ScriptedStream::with_response(frame(
            br#"{"accepted":true,"accepted_until":"2030-01-02T03:04:31Z"}"#,
        ));
        assert!(matches!(
            authorize(&mut overlong, request(), || now).await,
            Err(Error::RelayAdmissionMalformed)
        ));

        let completed_at = now + chrono::Duration::seconds(10);
        let mut slow_but_fresh = ScriptedStream::with_response(frame(
            br#"{"accepted":true,"accepted_until":"2030-01-02T03:04:31Z"}"#,
        ));
        assert_eq!(
            authorize(&mut slow_but_fresh, request(), || completed_at)
                .await
                .unwrap()
                .to_rfc3339(),
            "2030-01-02T03:04:31+00:00"
        );

        let mut extra_after_frame =
            frame(br#"{"accepted":true,"accepted_until":"2030-01-02T03:04:05Z"}"#);
        extra_after_frame.extend_from_slice(&[0]);
        let mut extra_after_frame = ScriptedStream::with_response(extra_after_frame);
        assert!(matches!(
            authorize(&mut extra_after_frame, request(), || now).await,
            Err(Error::RelayAdmissionMalformed)
        ));
    }

    #[tokio::test]
    async fn frame_bounds_are_enforced() {
        let mut sink = ScriptedStream::default();
        assert!(matches!(
            write_frame(&mut sink, &[], REQUEST_MAX_BYTES).await,
            Err(Error::RelayAdmissionFrameTooLarge { .. })
        ));
        let oversized = (RESPONSE_MAX_BYTES as u32 + 1).to_be_bytes().to_vec();
        let mut source = Cursor::new(oversized);
        assert!(matches!(
            read_frame(&mut source, RESPONSE_MAX_BYTES).await,
            Err(Error::RelayAdmissionFrameTooLarge { .. })
        ));
    }

    fn request() -> Request<'static> {
        Request {
            domain_id: Uuid::parse_str(DOMAIN_ID).unwrap(),
            target_peer_id: PEER_ID.parse().unwrap(),
            p2p_access_token: TOKEN,
        }
    }

    fn frame(payload: &[u8]) -> Vec<u8> {
        let mut framed = (payload.len() as u32).to_be_bytes().to_vec();
        framed.extend_from_slice(payload);
        framed
    }

    #[derive(Default)]
    struct ScriptedStream {
        response: Cursor<Vec<u8>>,
        written: Vec<u8>,
    }

    impl ScriptedStream {
        fn with_response(response: Vec<u8>) -> Self {
            Self {
                response: Cursor::new(response),
                written: Vec::new(),
            }
        }
    }

    impl AsyncRead for ScriptedStream {
        fn poll_read(
            mut self: Pin<&mut Self>,
            context: &mut Context<'_>,
            buffer: &mut [u8],
        ) -> Poll<io::Result<usize>> {
            Pin::new(&mut self.response).poll_read(context, buffer)
        }
    }

    impl AsyncWrite for ScriptedStream {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _context: &mut Context<'_>,
            buffer: &[u8],
        ) -> Poll<io::Result<usize>> {
            self.written.extend_from_slice(buffer);
            Poll::Ready(Ok(buffer.len()))
        }

        fn poll_flush(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_close(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }
}
