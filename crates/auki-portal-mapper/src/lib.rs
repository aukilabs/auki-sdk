//! Batteries-included Portal Mapper integration for Auki's reference QR
//! detector and Portal Service.
//!
//! The mapping algorithm remains in `auki-mappers` and accepts any detector
//! and Portal resolver. This crate is the optional product integration layer:
//! it normalizes `auki-qr-detector` output and resolves `https://r8.hr/...`
//! payloads through the authenticated DDS lighthouse API.

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use auki_datatypes::detection::DetectionFrame;
use auki_mappers::{
    ImagePoint, MapperInput, MapperInputError, PortalCandidate, PortalDefinition,
    PortalDetectionBatch, PortalResolver, PortalResolverError, TimedSdkSample,
};
use auki_qr_detector::{QR_DETECTION_TYPE, QrDetections};
use futures::{FutureExt, StreamExt, future::BoxFuture};
use reqwest::{Client, StatusCode, Url, header};
use serde::Deserialize;

/// Convert one reference QR detector envelope into the detector-neutral shape
/// consumed by [`auki_mappers::PortalMapperRunner`].
///
/// Refined subpixel corners are preferred when QR Lab produced them. Both QR
/// Lab corner representations already use the mapper's required
/// `TL, TR, BR, BL` order.
pub fn adapt_qr_detection_frame(
    frame: DetectionFrame,
) -> Result<PortalDetectionBatch, QrPortalAdapterError> {
    if frame.r#type != QR_DETECTION_TYPE {
        return Err(QrPortalAdapterError::UnexpectedDetectionType {
            expected: QR_DETECTION_TYPE,
            received: frame.r#type,
        });
    }
    let detections = QrDetections::decode(&frame.data)
        .map_err(|error| QrPortalAdapterError::InvalidQrPayload(error.to_string()))?;
    Ok(PortalDetectionBatch {
        sensor_hash: frame.sensor_hash,
        detections: detections
            .codes
            .into_iter()
            .map(|code| {
                let corners = code.refined_corners_px.unwrap_or(code.corners_px);
                PortalCandidate {
                    payload: code.payload,
                    corners_px: corners.map(|corner| ImagePoint {
                        x: corner.x,
                        y: corner.y,
                    }),
                }
            })
            .collect(),
    })
}

/// Adapt an opened QR Detection Log without changing its source identity,
/// clock, timestamps, or sequence numbers.
pub fn adapt_qr_detection_input(
    input: MapperInput<DetectionFrame>,
) -> MapperInput<PortalDetectionBatch> {
    let MapperInput {
        log_ref,
        clock,
        samples,
    } = input;
    let samples = samples.map(|sample| {
        sample.and_then(|sample| {
            let TimedSdkSample {
                sequence,
                timestamp_ns,
                payload,
            } = sample;
            adapt_qr_detection_frame(payload)
                .map(|payload| TimedSdkSample {
                    sequence,
                    timestamp_ns,
                    payload,
                })
                .map_err(|error| MapperInputError::new(error.to_string()))
        })
    });
    MapperInput::new(log_ref, clock, Box::pin(samples))
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum QrPortalAdapterError {
    #[error("expected DetectionFrame.type {expected:?}, received {received:?}")]
    UnexpectedDetectionType {
        expected: &'static str,
        received: String,
    },
    #[error("invalid QR detector payload: {0}")]
    InvalidQrPayload(String),
}

/// Runtime configuration for the Auki DDS Portal Service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DdsPortalServiceConfig {
    /// DDS origin, for example `https://dds.posemesh.org/`.
    pub service_base_url: String,
    /// Full HTTP Authorization value used by the host application.
    pub authorization: String,
    /// Value for DDS's `posemesh-client-id` header.
    pub client_id: String,
    /// QR URL origin. Production Portals use `https://r8.hr/`.
    pub portal_url_base: String,
    /// How long a canonical definition may be reused before DDS is queried
    /// again. This bounds staleness when a Portal's physical size changes.
    pub cache_ttl: Duration,
}

impl DdsPortalServiceConfig {
    pub fn production(
        service_base_url: impl Into<String>,
        authorization: impl Into<String>,
        client_id: impl Into<String>,
    ) -> Self {
        Self {
            service_base_url: service_base_url.into(),
            authorization: authorization.into(),
            client_id: client_id.into(),
            portal_url_base: "https://r8.hr/".into(),
            cache_ttl: Duration::from_secs(300),
        }
    }
}

/// Authenticated, caching resolver for Auki Portal QR payloads.
///
/// Non-Portal QR payloads resolve to `Ok(None)`. A recognized Portal URL is
/// looked up by its short ID using `GET /api/v1/lighthouses/{short_id}`.
/// DDS stores `size` in centimetres; definitions exposed to the PnP mapper are
/// converted to metres.
pub struct DdsPortalResolver {
    http: Client,
    service_base_url: Url,
    portal_url_base: Url,
    authorization: header::HeaderValue,
    client_id: header::HeaderValue,
    cache_ttl: Duration,
    cache: Mutex<HashMap<String, CachedPortalDefinition>>,
}

impl DdsPortalResolver {
    pub fn new(config: DdsPortalServiceConfig) -> Result<Self, DdsPortalResolverConfigError> {
        Self::with_http(config, Client::new())
    }

    /// Construct with an application-configured client (timeouts, proxy,
    /// certificate roots) while preserving the resolver's wire contract.
    pub fn with_http(
        config: DdsPortalServiceConfig,
        http: Client,
    ) -> Result<Self, DdsPortalResolverConfigError> {
        let service_base_url = parse_base_url("service_base_url", &config.service_base_url)?;
        let portal_url_base = parse_base_url("portal_url_base", &config.portal_url_base)?;
        let authorization = header::HeaderValue::from_str(&config.authorization)
            .map_err(|_| DdsPortalResolverConfigError::InvalidAuthorizationHeader)?;
        let client_id = header::HeaderValue::from_str(&config.client_id)
            .map_err(|_| DdsPortalResolverConfigError::InvalidClientIdHeader)?;
        if authorization.is_empty() {
            return Err(DdsPortalResolverConfigError::InvalidAuthorizationHeader);
        }
        if client_id.is_empty() {
            return Err(DdsPortalResolverConfigError::InvalidClientIdHeader);
        }
        Ok(Self {
            http,
            service_base_url,
            portal_url_base,
            authorization,
            client_id,
            cache_ttl: config.cache_ttl,
            cache: Mutex::new(HashMap::new()),
        })
    }

    pub fn clear_cache(&self) {
        self.cache.lock().expect("Portal cache poisoned").clear();
    }

    async fn resolve_payload(
        &self,
        payload: &str,
    ) -> Result<Option<PortalDefinition>, PortalResolverError> {
        let Some(short_id) = portal_short_id(payload, &self.portal_url_base) else {
            return Ok(None);
        };
        {
            let mut cache = self
                .cache
                .lock()
                .map_err(|_| PortalResolverError::new("Portal cache poisoned"))?;
            if let Some(cached) = cache.get(short_id)
                && cached.inserted_at.elapsed() < self.cache_ttl
            {
                return Ok(Some(cached.definition.clone()));
            }
            cache.remove(short_id);
        }

        let endpoint = lighthouse_endpoint(&self.service_base_url, short_id)
            .map_err(|error| PortalResolverError::new(error.to_string()))?;
        let response = self
            .http
            .get(endpoint)
            .header(header::AUTHORIZATION, self.authorization.clone())
            .header("posemesh-client-id", self.client_id.clone())
            .send()
            .await
            .map_err(|error| {
                PortalResolverError::new(format!("Portal Service request: {error}"))
            })?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let response = response.error_for_status().map_err(|error| {
            PortalResolverError::new(format!("Portal Service response: {error}"))
        })?;
        let lighthouse: DdsLighthouse = response.json().await.map_err(|error| {
            PortalResolverError::new(format!("Portal Service payload: {error}"))
        })?;
        let definition = lighthouse
            .into_definition()
            .map_err(PortalResolverError::new)?;
        self.cache
            .lock()
            .map_err(|_| PortalResolverError::new("Portal cache poisoned"))?
            .insert(
                short_id.to_owned(),
                CachedPortalDefinition {
                    inserted_at: Instant::now(),
                    definition: definition.clone(),
                },
            );
        Ok(Some(definition))
    }
}

impl PortalResolver for DdsPortalResolver {
    fn resolve<'a>(
        &'a self,
        payload: &'a str,
    ) -> BoxFuture<'a, Result<Option<PortalDefinition>, PortalResolverError>> {
        self.resolve_payload(payload).boxed()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DdsPortalResolverConfigError {
    #[error("{field} must be an absolute HTTP(S) base URL ending in /")]
    InvalidBaseUrl { field: &'static str },
    #[error("authorization must be a non-empty valid HTTP header value")]
    InvalidAuthorizationHeader,
    #[error("client_id must be a non-empty valid HTTP header value")]
    InvalidClientIdHeader,
}

fn parse_base_url(field: &'static str, value: &str) -> Result<Url, DdsPortalResolverConfigError> {
    let url =
        Url::parse(value).map_err(|_| DdsPortalResolverConfigError::InvalidBaseUrl { field })?;
    if !matches!(url.scheme(), "http" | "https")
        || url.cannot_be_a_base()
        || !url.path().ends_with('/')
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(DdsPortalResolverConfigError::InvalidBaseUrl { field });
    }
    Ok(url)
}

fn portal_short_id<'a>(payload: &'a str, portal_base: &Url) -> Option<&'a str> {
    let payload_url = Url::parse(payload).ok()?;
    if payload_url.scheme() != portal_base.scheme()
        || payload_url.host_str()? != portal_base.host_str()?
        || payload_url.port_or_known_default() != portal_base.port_or_known_default()
        || payload_url.query().is_some()
        || payload_url.fragment().is_some()
    {
        return None;
    }
    let base_segments: Vec<_> = portal_base
        .path_segments()?
        .filter(|s| !s.is_empty())
        .collect();
    let payload_segments: Vec<_> = payload_url
        .path_segments()?
        .filter(|s| !s.is_empty())
        .collect();
    if payload_segments.len() != base_segments.len() + 1
        || payload_segments[..base_segments.len()] != base_segments
    {
        return None;
    }
    let short_id = payload.rsplit('/').next()?;
    (!short_id.is_empty()
        && short_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')))
    .then_some(short_id)
}

fn lighthouse_endpoint(base: &Url, short_id: &str) -> Result<Url, url::ParseError> {
    base.join(&format!("api/v1/lighthouses/{short_id}"))
}

#[derive(Debug, Deserialize)]
struct DdsLighthouse {
    id: String,
    size: f64,
}

struct CachedPortalDefinition {
    inserted_at: Instant,
    definition: PortalDefinition,
}

impl DdsLighthouse {
    fn into_definition(self) -> Result<PortalDefinition, String> {
        if self.id.is_empty() {
            return Err("Portal Service returned an empty lighthouse id".into());
        }
        if !self.size.is_finite() || self.size <= 0.0 {
            return Err("Portal Service returned an invalid lighthouse size".into());
        }
        Ok(PortalDefinition {
            portal_id: self.id,
            physical_size_m: self.size / 100.0,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use auki_qr_detector::{PixelCorner, QR_DETECTION_SCHEMA_VERSION, QrDetection, QrDetections};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use super::*;

    fn qr_frame() -> DetectionFrame {
        QrDetections {
            schema_version: QR_DETECTION_SCHEMA_VERSION,
            codes: vec![QrDetection {
                payload: "HTTPS://R8.HR/Ab_12".into(),
                version: 1,
                ecc: 'M',
                mirrored: false,
                inverted: false,
                corners_px: [
                    PixelCorner { x: 1.0, y: 2.0 },
                    PixelCorner { x: 3.0, y: 4.0 },
                    PixelCorner { x: 5.0, y: 6.0 },
                    PixelCorner { x: 7.0, y: 8.0 },
                ],
                refined_corners_px: Some([
                    PixelCorner { x: 1.5, y: 2.5 },
                    PixelCorner { x: 3.5, y: 4.5 },
                    PixelCorner { x: 5.5, y: 6.5 },
                    PixelCorner { x: 7.5, y: 8.5 },
                ]),
                scanner_stage: 0,
            }],
        }
        .into_detection_frame("camera-hash")
    }

    #[test]
    fn qr_adapter_preserves_sensor_payload_and_refined_corner_order() {
        let batch = adapt_qr_detection_frame(qr_frame()).unwrap();
        assert_eq!(batch.sensor_hash, "camera-hash");
        assert_eq!(batch.detections[0].payload, "HTTPS://R8.HR/Ab_12");
        assert_eq!(
            batch.detections[0].corners_px[0],
            ImagePoint { x: 1.5, y: 2.5 }
        );
        assert_eq!(
            batch.detections[0].corners_px[3],
            ImagePoint { x: 7.5, y: 8.5 }
        );
    }

    #[test]
    fn only_exact_portal_origin_and_one_short_id_are_recognized() {
        let base = Url::parse("https://r8.hr/").unwrap();
        assert_eq!(portal_short_id("HTTPS://R8.HR/Ab_12", &base), Some("Ab_12"));
        assert_eq!(portal_short_id("https://r8.hr/a/b", &base), None);
        assert_eq!(portal_short_id("https://r8.hr/a?x=1", &base), None);
        assert_eq!(portal_short_id("https://evil.example/a", &base), None);
        assert_eq!(portal_short_id("ordinary QR", &base), None);
    }

    #[tokio::test]
    async fn resolver_uses_dds_contract_converts_cm_to_m_and_caches() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(AtomicUsize::new(0));
        let server_requests = requests.clone();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = vec![0; 4096];
            let read = socket.read(&mut bytes).await.unwrap();
            let request = String::from_utf8_lossy(&bytes[..read]);
            assert!(request.starts_with("GET /api/v1/lighthouses/Ab_12 HTTP/1.1"));
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("authorization: bearer test")
            );
            assert!(request.contains("posemesh-client-id: park"));
            server_requests.fetch_add(1, Ordering::SeqCst);
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 32\r\nconnection: close\r\n\r\n{\"id\":\"portal-uuid\",\"size\":20.0}",
                )
                .await
                .unwrap();
        });
        let resolver = DdsPortalResolver::new(DdsPortalServiceConfig {
            service_base_url: format!("http://{address}/"),
            authorization: "Bearer test".into(),
            client_id: "park".into(),
            portal_url_base: "https://r8.hr/".into(),
            cache_ttl: Duration::from_secs(300),
        })
        .unwrap();

        let first = resolver
            .resolve("HTTPS://R8.HR/Ab_12")
            .await
            .unwrap()
            .unwrap();
        let second = resolver
            .resolve("HTTPS://R8.HR/Ab_12")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            first,
            PortalDefinition {
                portal_id: "portal-uuid".into(),
                physical_size_m: 0.2
            }
        );
        assert_eq!(second, first);
        assert_eq!(requests.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn non_portal_payload_does_not_touch_service() {
        let resolver = DdsPortalResolver::new(DdsPortalServiceConfig::production(
            "https://dds.example/",
            "Bearer test",
            "park",
        ))
        .unwrap();
        assert_eq!(resolver.resolve("hello").await.unwrap(), None);
    }
}
