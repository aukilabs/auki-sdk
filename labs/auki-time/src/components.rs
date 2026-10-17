//! Fixed session-clock anchors as ordinary retained Component Products.
//!
//! Creating this helper neither starts a session nor exports a network service.
//! The host serves the clock definitions and explicitly exports `product()` on
//! its existing Component protocol endpoint. See the crate README for the profile.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use auki_components::*;
use auki_registry::{ClockBody, ClockMeta, ClockRegistryEntry, Scope};
use serde::{Deserialize, Serialize};

use crate::{Clock, SessionClock, tick};

pub const ANCHOR_SCHEMA: &str = "auki.session-utc-anchor/v1";
pub const CONTRACT_SCHEMA: &str = "auki.session-utc-contract/v1";

/// Immutable relationship declaration. Full definitions resolve each endpoint's
/// exact registry reference through `from_reference` / `to_reference`.
///
/// This profile is a held offset measured once near session startup, not a
/// continuously disciplined UTC clock. Sampling uncertainty is in the payload;
/// unknown UTC accuracy and oscillator drift must not be interpreted as zero.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionUtcContract {
    pub from_clock: ClockRegistryEntry,
    pub to_clock: ClockRegistryEntry,
    pub utc_accuracy_ns: Option<u64>,
}

impl SessionUtcContract {
    pub fn from_reference(&self) -> ClockReference {
        reference(&self.from_clock)
    }

    pub fn to_reference(&self) -> ClockReference {
        reference(&self.to_clock)
    }

    /// Content-derived identity binds both definitions and the accuracy claim.
    pub fn output_id(&self) -> String {
        format!("session-utc-{}", manifest_hash(self))
    }

    pub fn validate(&self) -> Result<(), SessionClockError> {
        for entry in [&self.from_clock, &self.to_clock] {
            clock::validate_clock(&reference(entry)).map_err(SessionClockError::Invalid)?;
            if entry.session_id.trim().is_empty() || entry.session_id.len() > 256 {
                return Err(invalid("clock requires a bounded, nonempty session ID"));
            }
        }
        if self.from_clock.peer_id != self.to_clock.peer_id
            || self.from_clock.session_id != self.to_clock.session_id
            || self.from_clock.clock_id == self.to_clock.clock_id
        {
            return Err(invalid(
                "session UTC endpoints require one owner/session and distinct IDs",
            ));
        }
        if !matches!(&self.from_clock.body, ClockBody::MonotonicClock(meta)
            if meta.monotonic && meta.epoch.is_none() && meta.scope == Scope::DeviceLocal
            && matches!(meta.unit.as_str(), "ns" | "nanoseconds"))
        {
            return Err(invalid(
                "source must be a device-local session-monotonic nanosecond clock",
            ));
        }
        if !matches!(&self.to_clock.body, ClockBody::UtcClock(meta)
            if !meta.monotonic && meta.epoch.as_deref() == Some("1970-01-01T00:00:00Z")
            && meta.scope == Scope::DeviceLocal && matches!(meta.unit.as_str(), "ns" | "nanoseconds"))
        {
            return Err(invalid(
                "destination must be a device-local Unix UTC nanosecond clock",
            ));
        }
        Ok(())
    }

    pub fn validate_output(&self, output: &OutputManifest) -> Result<(), SessionClockError> {
        self.validate()?;
        output.validate().map_err(SessionClockError::Invalid)?;
        if output.peer_id != self.from_clock.peer_id
            || output.clock != self.from_reference()
            || output.output_id != self.output_id()
            || output.payload != payload_contract()
            || output.spatial_frame.is_some()
            || output.spatial_frame_id.is_some()
        {
            return Err(invalid(
                "session UTC Output does not match its exact contract",
            ));
        }
        Ok(())
    }
}

fn reference(entry: &ClockRegistryEntry) -> ClockReference {
    ClockReference {
        peer_id: entry.peer_id.clone(),
        id: entry.clock_id.clone(),
        hash: entry.hash(),
    }
}

fn payload_contract() -> PayloadContract {
    PayloadContract::Structured(StructuredPayloadContract {
        modality: "time_transform".into(),
        datatype: ANCHOR_SCHEMA.into(),
        schema: ANCHOR_SCHEMA.into(),
        observes: "session_monotonic_to_local_utc".into(),
        unit: Some("nanoseconds".into()),
    })
}

/// A failed startup attempt is discoverable but never a usable transform.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UtcUnavailable {
    ReadFailed,
    OutOfRange,
    InvalidSample,
}

/// Capture time belongs to the enclosing SDK Observation in `from_clock`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum SessionUtcAnchor {
    Available {
        /// `UTC_ns = session_ns + offset_ns` (checked arithmetic required).
        offset_ns: i64,
        /// Full before/after read window, NOT UTC accuracy or a drift bound.
        uncertainty_ns: u32,
    },
    Unavailable {
        reason: UtcUnavailable,
    },
}

impl ContractType for SessionUtcAnchor {
    const DATATYPE: &'static str = ANCHOR_SCHEMA;
}

#[derive(Debug, thiserror::Error)]
pub enum SessionClockError {
    #[error("invalid session UTC contract: {0}")]
    Invalid(String),
    #[error("the session UTC startup attempt is immutable and has already been published")]
    AlreadyPublished,
    #[error("no session UTC startup attempt has been published")]
    NotPublished,
    #[error("component: {0}")]
    Component(Box<ComponentBuildError>),
    #[error("capture: {0}")]
    Capture(#[from] ProductCaptureError),
    #[error("publication: {0}")]
    Publish(#[from] PublishError),
    #[error("product access: {0}")]
    Access(#[from] ProductAccessError),
}

fn invalid(message: impl Into<String>) -> SessionClockError {
    SessionClockError::Invalid(message.into())
}

impl From<ComponentBuildError> for SessionClockError {
    fn from(error: ComponentBuildError) -> Self {
        Self::Component(Box::new(error))
    }
}

/// One local publisher for the peer's *existing* session clock. No timers,
/// network calls, independent epochs or background sampler threads.
pub struct SessionClockPublisher {
    session: SessionClock,
    contract: SessionUtcContract,
    _component: Component,
    output: ConfiguredObservable<SessionUtcAnchor>,
    capture: BufferProductCapture<SessionUtcAnchor>,
    attempted: bool,
}

impl SessionClockPublisher {
    pub fn new(
        runtime: &ComponentRuntime,
        session: &SessionClock,
    ) -> Result<Self, SessionClockError> {
        let from = session.registry_entry();
        let contract = SessionUtcContract {
            to_clock: ClockRegistryEntry {
                peer_id: from.peer_id.clone(),
                session_id: from.session_id.clone(),
                clock_id: format!("{}/{}/utc", from.peer_id, from.session_id),
                body: ClockBody::UtcClock(ClockMeta {
                    unit: "ns".into(),
                    monotonic: false,
                    epoch: Some("1970-01-01T00:00:00Z".into()),
                    scope: Scope::DeviceLocal,
                }),
            },
            from_clock: from,
            utc_accuracy_ns: None,
        };
        contract.validate()?;
        if runtime.peer_id() != contract.from_clock.peer_id {
            return Err(invalid("runtime does not own the supplied session clock"));
        }
        let id = contract.output_id();
        let component =
            runtime.component(ComponentSpec::new(&id).observable(ObservableContract {
                name: "anchor".into(),
                datatype: ANCHOR_SCHEMA.into(),
                schema: ANCHOR_SCHEMA.into(),
                access: vec![ObservationAccess::FollowNew],
                exposure: Exposure::Cluster,
            }))?;
        let output = component.configured_observable(ConfiguredObservableSpec::new(
            "anchor",
            &id,
            contract.from_reference(),
            payload_contract(),
        ))?;
        contract.validate_output(output.manifest())?;
        component.expose()?;
        let descriptor = contract.clone();
        let capture = runtime.capture_buffer_with_metadata(
            id,
            &output,
            BufferLimits {
                max_entries: Some(1),
                max_bytes: Some(1024),
                target_duration: None,
            },
            |_| std::mem::size_of::<SessionUtcAnchor>(),
            move |observation| {
                validate_anchor(observation).map_err(|e| e.to_string())?;
                Ok(Some(CatalogProductMetadata {
                    schema: CONTRACT_SCHEMA.into(),
                    source_sequence: observation.sequence,
                    value: serde_json::to_value(&descriptor).map_err(|e| e.to_string())?,
                }))
            },
        )?;
        Ok(Self {
            session: session.clone(),
            contract,
            _component: component,
            output,
            capture,
            attempted: false,
        })
    }

    pub fn contract(&self) -> &SessionUtcContract {
        &self.contract
    }

    pub fn product(&self) -> RetainedProduct<SessionUtcAnchor> {
        self.capture.product()
    }

    /// Call once at session startup, before capture workloads. Wall-clock
    /// corrections after this call cannot move the retained anchor.
    pub fn publish_startup_anchor(
        &mut self,
    ) -> Result<Observation<SessionUtcAnchor>, SessionClockError> {
        self.publish_startup_anchor_with_utc(|| {
            let duration = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| UtcUnavailable::OutOfRange)?;
            i64::try_from(duration.as_nanos()).map_err(|_| UtcUnavailable::OutOfRange)
        })
    }

    /// Inject only the UTC reader, not a capture timestamp or a second monotonic
    /// clock. It must read this host's current Unix UTC in nanoseconds. Failures
    /// publish a typed unavailable outcome. A second call fails without reading.
    pub fn publish_startup_anchor_with_utc(
        &mut self,
        utc_now: impl FnOnce() -> Result<i64, UtcUnavailable>,
    ) -> Result<Observation<SessionUtcAnchor>, SessionClockError> {
        if self.attempted {
            return Err(SessionClockError::AlreadyPublished);
        }
        let before = self.session.now_i64_ns();
        let utc = utc_now();
        let after = self.session.now_i64_ns();
        let (timestamp, anchor) = sample(before, utc, after);
        // Even a failed retention/publication must not permit a second anchor.
        self.attempted = true;
        self.output.publish(timestamp, Arc::new(anchor))?;
        if !self.capture.errors().is_empty() {
            return Err(invalid("session UTC anchor retention failed"));
        }
        read_session_anchor(&self.product(), &self.contract)
    }
}

/// Validate/read the same local or imported SDK Product. Sequence zero is the
/// only accepted anchor; neither a replacement nor a latest-time guess is valid.
pub fn read_session_anchor(
    product: &RetainedProduct<SessionUtcAnchor>,
    contract: &SessionUtcContract,
) -> Result<Observation<SessionUtcAnchor>, SessionClockError> {
    contract.validate_output(&product.producer)?;
    if product.manifest.producer != product.producer.reference()
        || product.manifest.peer_id != product.producer.peer_id
        || product.manifest.hash() != product.manifest_hash
        || product.manifest.form != ProductForm::Buffer
        || product.manifest.schema != "auki.product-manifest/v1"
    {
        return Err(invalid("anchor Product provenance mismatch"));
    }
    let observation = product
        .latest_existing()?
        .ok_or(SessionClockError::NotPublished)?;
    let range = product.buffer().range();
    if range.entries != 1 || range.first_sequence != Some(0) || range.last_sequence != Some(0) {
        return Err(SessionClockError::AlreadyPublished);
    }
    if observation.output != product.producer.reference() {
        return Err(invalid("anchor observation provenance mismatch"));
    }
    validate_anchor(&observation)?;
    Ok(observation)
}

fn validate_anchor(observation: &Observation<SessionUtcAnchor>) -> Result<(), SessionClockError> {
    if observation.sequence != 0 {
        return Err(SessionClockError::AlreadyPublished);
    }
    let timestamp = i64::try_from(observation.timestamp_ns)
        .map_err(|_| invalid("anchor timestamp overflow"))?;
    if let SessionUtcAnchor::Available { offset_ns, .. } = *observation.payload
        && timestamp.checked_add(offset_ns).is_none_or(|utc| utc < 0)
    {
        return Err(invalid("anchor UTC timestamp out of range"));
    }
    Ok(())
}

// Validate before using tick: its legacy saturating arithmetic must not invent
// an offset or understate a sampling window for this strict profile.
fn sample(before: i64, utc: Result<i64, UtcUnavailable>, after: i64) -> (u64, SessionUtcAnchor) {
    let invalid_sample = || {
        (
            before.max(0) as u64,
            SessionUtcAnchor::Unavailable {
                reason: UtcUnavailable::InvalidSample,
            },
        )
    };
    if before < 0 || after < before || after - before > i64::from(u32::MAX) {
        return invalid_sample();
    }
    let midpoint = before + (after - before) / 2;
    let utc = match utc {
        Ok(utc) if utc >= 0 && utc.checked_sub(midpoint).is_some() => utc,
        Ok(_) => {
            return (
                midpoint as u64,
                SessionUtcAnchor::Unavailable {
                    reason: UtcUnavailable::OutOfRange,
                },
            );
        }
        Err(reason) => return (midpoint as u64, SessionUtcAnchor::Unavailable { reason }),
    };
    struct Readings {
        first: std::sync::atomic::AtomicBool,
        before: i64,
        after: i64,
        utc: i64,
    }
    impl Clock for Readings {
        fn read_from_ns(&self) -> i64 {
            if self.first.swap(false, std::sync::atomic::Ordering::Relaxed) {
                self.before
            } else {
                self.after
            }
        }
        fn read_to_ns(&self) -> i64 {
            self.utc
        }
    }
    let (timestamp, entry) = tick(&Readings {
        first: std::sync::atomic::AtomicBool::new(true),
        before,
        after,
        utc,
    });
    (
        timestamp as u64,
        SessionUtcAnchor::Available {
            offset_ns: entry.offset_ns,
            uncertainty_ns: entry.uncertainty_ns,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sampling_preserves_sdk_midpoint_offset_and_full_read_window() {
        assert_eq!(
            sample(100, Ok(1_000), 140),
            (
                120,
                SessionUtcAnchor::Available {
                    offset_ns: 880,
                    uncertainty_ns: 40,
                }
            )
        );
        assert_eq!(
            sample(100, Ok(50), 100),
            (
                100,
                SessionUtcAnchor::Available {
                    offset_ns: -50,
                    uncertainty_ns: 0,
                }
            )
        );
    }

    #[test]
    fn invalid_sampling_never_clamps_a_valid_offset_or_uncertainty() {
        for (before, after) in [(-1, 1), (2, 1), (0, i64::from(u32::MAX) + 1), (0, i64::MAX)] {
            assert!(matches!(
                sample(before, Ok(1_000), after).1,
                SessionUtcAnchor::Unavailable {
                    reason: UtcUnavailable::InvalidSample
                }
            ));
        }
        assert!(matches!(
            sample(100, Ok(i64::MIN), 100).1,
            SessionUtcAnchor::Unavailable {
                reason: UtcUnavailable::OutOfRange
            }
        ));
        assert_eq!(
            sample(i64::MAX - 1, Ok(i64::MAX), i64::MAX),
            (
                (i64::MAX - 1) as u64,
                SessionUtcAnchor::Available {
                    offset_ns: 1,
                    uncertainty_ns: 1
                }
            )
        );
        assert!(matches!(
            sample(0, Ok(1_000), i64::from(u32::MAX)).1,
            SessionUtcAnchor::Available {
                uncertainty_ns: u32::MAX,
                ..
            }
        ));
    }

    #[test]
    fn unavailable_read_keeps_measurement_time_not_a_zero_offset() {
        assert_eq!(
            sample(100, Err(UtcUnavailable::ReadFailed), 140),
            (
                120,
                SessionUtcAnchor::Unavailable {
                    reason: UtcUnavailable::ReadFailed
                }
            )
        );
    }

    #[test]
    fn reader_rejects_replacement_and_malformed_imported_anchor() {
        let runtime = ComponentRuntime::new("peer");
        let session = SessionClock::new("peer", "session", "capture");
        let mut publisher = SessionClockPublisher::new(&runtime, &session).unwrap();
        let original = publisher
            .publish_startup_anchor_with_utc(|| Ok(1_000_000_000))
            .unwrap();
        let source = publisher.product();
        for case in 0..6 {
            let imported = RetainedProduct::imported_buffer(
                source.manifest.clone(),
                source.manifest_hash.clone(),
                source.producer.clone(),
                BufferLimits::entries(1),
                |_| 64,
            )
            .unwrap();
            let mut observation = original.clone();
            let mut envelope_sequence = 0;
            match case {
                0 => {
                    observation.sequence = 1;
                    envelope_sequence = 1;
                }
                1 => observation.output.manifest_hash = "0".repeat(32),
                2 => observation.timestamp_ns = u64::MAX,
                3 => {
                    observation.payload = Arc::new(SessionUtcAnchor::Available {
                        offset_ns: i64::MAX,
                        uncertainty_ns: 0,
                    })
                }
                4 => {
                    observation.payload = Arc::new(SessionUtcAnchor::Available {
                        offset_ns: i64::MIN,
                        uncertainty_ns: 0,
                    })
                }
                5 => envelope_sequence = 1,
                _ => unreachable!(),
            }
            imported
                .buffer()
                .append_shared(Arc::new(Envelope::new(
                    envelope_sequence,
                    observation.timestamp_ns,
                    observation,
                )))
                .unwrap();
            assert!(
                read_session_anchor(&imported, publisher.contract()).is_err(),
                "case {case}"
            );
        }
    }
}
