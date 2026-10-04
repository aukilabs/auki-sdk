//! Explicitly framed odometry observations and queries over standard SDK Buffers.
use auki_components::*;
use auki_registry::FrameRegistryEntry;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub const POSE_SCHEMA: &str = "auki.odometry-pose/v1";
pub const METADATA_SCHEMA: &str = "auki.odometry-product/v1";

/// One continuous physical reference frame, not just an axis convention.
/// Destination frame IDs MUST be new after an odometry reset. Hosts retain and
/// serve these registry definitions and the referenced clock definition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoseContract {
    pub from_frame: FrameRegistryEntry,
    pub to_frame: FrameRegistryEntry,
    pub clock: ClockReference,
    pub session_id: String,
}
impl PoseContract {
    pub fn validate(&self) -> Result<(), PoseError> {
        clock::validate_clock(&self.clock).map_err(PoseError::Invalid)?;
        if self.session_id.trim().is_empty() || self.session_id.len() > 256 {
            return Err(PoseError::Invalid("invalid odometry session ID".into()));
        }
        for f in [&self.from_frame, &self.to_frame] {
            if f.peer_id.trim().is_empty() {
                return Err(PoseError::Invalid("frame owner required".into()));
            }
            FrameRegistryEntry::validate_id(&f.frame_id)
                .map_err(|e| PoseError::Invalid(e.to_string()))?;
            auki_geometry::convention_matrix(f, f)
                .map_err(|e| PoseError::Invalid(e.to_string()))?;
        }
        if self.from_frame.peer_id == self.to_frame.peer_id
            && self.from_frame.frame_id == self.to_frame.frame_id
        {
            return Err(PoseError::Invalid(
                "odometry endpoints must be distinct".into(),
            ));
        }
        // A rigid quaternion cannot encode a handedness change or a unit scale.
        if self.from_frame.handedness != self.to_frame.handedness
            || self.from_frame.units != self.to_frame.units
        {
            return Err(PoseError::Invalid(
                "rigid pose endpoints require matching units and handedness".into(),
            ));
        }
        Ok(())
    }
}

/// Translation is in destination-frame units. Rotation is Hamilton (x,y,z,w),
/// actively transforming source-frame coordinates into destination coordinates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pose {
    pub translation: [f64; 3],
    pub rotation_xyzw: [f64; 4],
}
impl Pose {
    pub fn validate(&self) -> Result<(), PoseError> {
        let norm = self.rotation_xyzw.iter().map(|x| x * x).sum::<f64>();
        if !self
            .translation
            .iter()
            .chain(self.rotation_xyzw.iter())
            .all(|x| x.is_finite())
            || (norm - 1.0).abs() > 1e-6
        {
            return Err(PoseError::Invalid(
                "pose requires finite translation and unit quaternion".into(),
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Tracking {
    Valid { pose: Pose },
    Lost,
}
/// Self-describing payload; enclosing Observation timestamp is measurement time.
/// The payload clock must equal the exact Output Manifest clock.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PoseUpdate {
    pub contract: PoseContract,
    pub tracking: Tracking,
}
impl ContractType for PoseUpdate {
    const DATATYPE: &'static str = "auki.odometry-pose";
}
impl PoseUpdate {
    pub fn validate(&self) -> Result<(), PoseError> {
        self.contract.validate()?;
        if let Tracking::Valid { pose } = &self.tracking {
            pose.validate()?;
        }
        Ok(())
    }
}
#[derive(Debug, thiserror::Error)]
pub enum PoseError {
    #[error("invalid pose contract: {0}")]
    Invalid(String),
    #[error("pose frame, clock, session or producer mismatch")]
    ContractMismatch,
    #[error("measurement timestamp must strictly increase")]
    NonMonotonic,
    #[error("pose query is outside retained history; extrapolation is forbidden")]
    OutsideHistory,
    #[error("tracking is lost at a bounding observation")]
    TrackingLost,
    #[error("interpolation interval exceeds the requested maximum gap")]
    GapTooLarge,
    #[error("component: {0}")]
    Component(Box<ComponentBuildError>),
    #[error("publication: {0}")]
    Publish(#[from] PublishError),
    #[error("buffer: {0}")]
    Buffer(#[from] BufferError),
}

impl From<ComponentBuildError> for PoseError {
    fn from(error: ComponentBuildError) -> Self {
        Self::Component(Box::new(error))
    }
}

/// Driver-neutral publisher. No vendor polling, history queue or map alignment.
/// Create a fresh component/output for each odometry session; end the old output
/// before resetting. Never reuse its destination frame ID for a reset origin.
pub struct OdometryComponent {
    _component: Component,
    output: ConfiguredObservable<PoseUpdate>,
    contract: PoseContract,
    last_timestamp_ns: Option<u64>,
}
impl OdometryComponent {
    pub fn new(
        runtime: &ComponentRuntime,
        id: &str,
        contract: PoseContract,
    ) -> Result<Self, PoseError> {
        contract.validate()?;
        let component =
            runtime.component(ComponentSpec::new(id).observable(ObservableContract {
                name: "poses".into(),
                datatype: PoseUpdate::DATATYPE.into(),
                schema: POSE_SCHEMA.into(),
                access: vec![ObservationAccess::FollowNew],
                exposure: Exposure::Cluster,
            }))?;
        let output = component.configured_observable(
            ConfiguredObservableSpec::new(
                "poses",
                format!("poses-{}", contract.session_id),
                contract.clock.clone(),
                PayloadContract::Structured(StructuredPayloadContract {
                    modality: "pose".into(),
                    datatype: PoseUpdate::DATATYPE.into(),
                    schema: POSE_SCHEMA.into(),
                    observes: "robot_base_pose_in_odometry".into(),
                    unit: None,
                }),
            )
            .in_registered_frame(contract.to_frame.clone()),
        )?;
        component.expose()?;
        Ok(Self {
            _component: component,
            output,
            contract,
            last_timestamp_ns: None,
        })
    }
    pub fn output(&self) -> &ConfiguredObservable<PoseUpdate> {
        &self.output
    }
    pub fn contract(&self) -> &PoseContract {
        &self.contract
    }
    pub fn publish(
        &mut self,
        timestamp_ns: u64,
        tracking: Tracking,
    ) -> Result<Observation<PoseUpdate>, PoseError> {
        if self
            .last_timestamp_ns
            .is_some_and(|last| timestamp_ns <= last)
        {
            return Err(PoseError::NonMonotonic);
        }
        let update = PoseUpdate {
            contract: self.contract.clone(),
            tracking,
        };
        update.validate()?;
        let (observation, _) = self.output.publish(timestamp_ns, Arc::new(update))?;
        self.last_timestamp_ns = Some(timestamp_ns);
        Ok(observation)
    }
    pub fn end(
        &mut self,
        timestamp_ns: u64,
        reason: ObservationEndReason,
    ) -> Result<ObservationEnd, PoseError> {
        if self
            .last_timestamp_ns
            .is_some_and(|last| timestamp_ns < last)
        {
            return Err(PoseError::NonMonotonic);
        }
        Ok(self.output.end(timestamp_ns, reason)?)
    }
}

/// Standard Buffer Product capture, with discoverable frame/clock declarations.
/// Retention is entirely owned by auki-components; attach before publishing.
/// Metadata appears with the first accepted observation and pins its source sequence.
pub fn capture_pose_history(
    runtime: &ComponentRuntime,
    id: &str,
    source: &OdometryComponent,
    limits: BufferLimits,
) -> Result<BufferProductCapture<PoseUpdate>, ProductCaptureError> {
    let expected = source.contract.clone();
    runtime.capture_buffer_with_metadata(
        id,
        source.output(),
        limits,
        |o| serde_json::to_vec(o).map_or(usize::MAX, |v| v.len()),
        move |o| {
            o.payload.validate().map_err(|e| e.to_string())?;
            if o.payload.contract != expected {
                return Err("pose contract changed within output".into());
            }
            Ok(Some(CatalogProductMetadata {
                schema: METADATA_SCHEMA.into(),
                source_sequence: o.sequence,
                value: serde_json::to_value(&expected).map_err(|e| e.to_string())?,
            }))
        },
    )
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PoseQuery {
    pub contract: PoseContract,
    pub timestamp_ns: u64,
    pub max_gap_ns: u64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum Derivation {
    Exact {
        sequence: u64,
    },
    Interpolated {
        before_sequence: u64,
        after_sequence: u64,
        fraction: f64,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PoseAtTime {
    pub product: ProductReference,
    pub contract: PoseContract,
    pub timestamp_ns: u64,
    pub pose: Pose,
    pub derivation: Derivation,
}

/// Queries the actual standard SDK Buffer, including imported remote Products.
/// No private history, latest-pose substitution, cross-clock conversion or extrapolation.
pub fn pose_at(
    product: &RetainedProduct<PoseUpdate>,
    query: &PoseQuery,
) -> Result<PoseAtTime, PoseError> {
    query.contract.validate()?;
    if product.producer.clock != query.contract.clock
        || product
            .producer
            .spatial_frame
            .as_ref()
            .map(|f| &f.definition)
            != Some(&query.contract.to_frame)
        || product.producer.payload.datatype() != PoseUpdate::DATATYPE
        || product.producer.payload.schema() != POSE_SCHEMA
    {
        return Err(PoseError::ContractMismatch);
    }
    let bracket = product.buffer().bracket_time_ns(query.timestamp_ns)?;
    let before = bracket.before.ok_or(PoseError::OutsideHistory)?;
    let after = bracket.after.ok_or(PoseError::OutsideHistory)?;
    for e in [&before, &after] {
        let o = &e.payload;
        if o.output != product.producer.reference()
            || o.payload.contract != query.contract
            || e.timestamp_ns != o.timestamp_ns
            || e.sequence != o.sequence
        {
            return Err(PoseError::ContractMismatch);
        }
        o.payload.validate()?;
    }
    let (Tracking::Valid { pose: a }, Tracking::Valid { pose: b }) = (
        &before.payload.payload.tracking,
        &after.payload.payload.tracking,
    ) else {
        return Err(PoseError::TrackingLost);
    };
    let (pose, derivation) = if before.sequence == after.sequence {
        (
            a.clone(),
            Derivation::Exact {
                sequence: before.sequence,
            },
        )
    } else {
        let gap = after.timestamp_ns - before.timestamp_ns;
        if gap > query.max_gap_ns {
            return Err(PoseError::GapTooLarge);
        }
        let t = (query.timestamp_ns - before.timestamp_ns) as f64 / gap as f64;
        (
            Pose {
                translation: std::array::from_fn(|i| {
                    (1.0 - t) * a.translation[i] + t * b.translation[i]
                }),
                rotation_xyzw: slerp(a.rotation_xyzw, b.rotation_xyzw, t),
            },
            Derivation::Interpolated {
                before_sequence: before.sequence,
                after_sequence: after.sequence,
                fraction: t,
            },
        )
    };
    pose.validate()?;
    Ok(PoseAtTime {
        product: product.reference(),
        contract: query.contract.clone(),
        timestamp_ns: query.timestamp_ns,
        pose,
        derivation,
    })
}

// Shortest-arc SLERP; inputs are validated unit quaternions. Renormalize only the
// computed interpolation result, never the published measurements.
fn slerp(a: [f64; 4], mut b: [f64; 4], t: f64) -> [f64; 4] {
    let mut dot: f64 = a.iter().zip(b).map(|(a, b)| a * b).sum();
    if dot < 0.0 {
        b = b.map(|v| -v);
        dot = -dot;
    }
    let (u, v) = if dot > 0.9995 {
        (1.0 - t, t)
    } else {
        let theta = dot.clamp(-1.0, 1.0).acos();
        (
            ((1.0 - t) * theta).sin() / theta.sin(),
            (t * theta).sin() / theta.sin(),
        )
    };
    let q: [f64; 4] = std::array::from_fn(|i| u * a[i] + v * b[i]);
    let norm = q.iter().map(|v| v * v).sum::<f64>().sqrt();
    q.map(|v| v / norm)
}
