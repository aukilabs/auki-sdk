//! Resolve explicitly admitted clock evidence without network access or timers.
//!
//! Equal names, epochs, units, or peer roles never create an edge. The caller
//! fetches registry definitions and retained Products through existing SDK APIs,
//! then explicitly admits evidence. No Domain clock or privileged peer is needed.

use auki_components::{ClockReference, ContractType, OutputReference, ProductReference};
use auki_registry::ClockRegistryEntry;
use serde::{Deserialize, Serialize};

mod product;
pub use product::clock_reference;

pub const OFFSET_SCHEMA: &str = "auki.clock-offset/v1";
pub const OFFSET_CONTRACT_SCHEMA: &str = "auki.clock-offset-contract/v1";

/// Inclusive coverage in the relationship's explicitly named source clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimeInterval {
    pub start_ns: u64,
    pub end_ns: u64,
}

/// Immutable endpoint and direction declaration, bound into the Output identity.
/// A third peer may publish this relationship; admission is the caller's policy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OffsetContract {
    pub from_clock: ClockReference,
    pub to_clock: ClockReference,
    pub allow_inverse: bool,
}

/// `to = from + offset_ns`. Observation time is the measurement time in the
/// source clock. Bounds are declarations, not inferred synchronization quality.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum OffsetSample {
    Available {
        offset_ns: i64,
        /// Error bound at the measurement instant; None is unknown, not zero.
        uncertainty_ns: Option<u64>,
        /// Maximum offset error growth, in nanoseconds per billion source ns.
        /// None only permits a finite bound for point coverage at measurement.
        max_drift_ppb: Option<u64>,
        validity: TimeInterval,
    },
    /// Does not revoke other evidence or establish a zero offset. Producers must
    /// bound available intervals so they do not cross an invalidated relationship.
    Unavailable { validity: TimeInterval },
}

impl ContractType for OffsetSample {
    const DATATYPE: &'static str = OFFSET_SCHEMA;
}

/// Frozen provenance and payload. Later Product appends cannot change a result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemporalEvidence {
    pub product: ProductReference,
    pub output: OutputReference,
    pub sequence: u64,
    pub measurement_ns: u64,
    pub contract: OffsetContract,
    pub sample: OffsetSample,
    /// Hashes of the original descriptor/payload, including for helper imports.
    pub source_contract_hash: String,
    pub source_payload_hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedStep {
    pub evidence: TemporalEvidence,
    pub inverted: bool,
    /// Conservative error over the evidence's entire declared interval.
    pub applied_uncertainty_ns: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemporalQuery {
    pub from_clock: ClockReference,
    pub to_clock: ClockReference,
    pub timestamp_ns: u64,
    pub input_uncertainty_ns: u64,
    pub max_uncertainty_ns: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemporalResolution {
    pub from_clock: ClockReference,
    pub to_clock: ClockReference,
    pub source_timestamp_ns: u64,
    pub timestamp_ns: u64,
    pub uncertainty_ns: u64,
    /// Source query centers for which this same path and bound remain valid,
    /// including the query's input uncertainty. Not an extrapolation permission.
    pub source_coverage: TimeInterval,
    pub path: Vec<ResolvedStep>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolutionIssue {
    OutsideCoverage,
    Unavailable,
    UnknownUncertainty,
    ArithmeticOverflow,
    UncertaintyExceeded,
}

#[derive(Debug, thiserror::Error)]
pub enum TemporalError {
    #[error("invalid temporal evidence: {0}")]
    Invalid(String),
    #[error("exact clock definition is missing: {0:?}")]
    MissingClock(Box<ClockReference>),
    #[error("temporal sample {sequence} is not retained")]
    MissingSample { sequence: u64 },
    #[error("temporal evidence was not admitted by the caller")]
    Denied,
    #[error("temporal resolver {resource} limit exceeded; no partial answer")]
    LimitExceeded { resource: &'static str },
    #[error("no applicable bounded temporal path: {issues:?}")]
    Unresolved { issues: Vec<ResolutionIssue> },
    #[error("applicable temporal paths disagree")]
    Conflict { candidates: Vec<TemporalResolution> },
}

/// Count bounds, plus bounded identity strings on intake. No implicit eviction.
#[derive(Clone, Copy, Debug)]
pub struct ResolverLimits {
    pub max_clocks: usize,
    pub max_samples: usize,
    pub max_hops: usize,
    pub max_expansions: usize,
    pub max_candidates: usize,
}

impl Default for ResolverLimits {
    fn default() -> Self {
        Self {
            max_clocks: 256,
            max_samples: 4096,
            max_hops: 16,
            max_expansions: 16384,
            max_candidates: 128,
        }
    }
}

/// An index of verified definitions and explicitly admitted, immutable SDK
/// Product observations. No discovery watcher, registry store or new transport.
pub struct TemporalResolver {
    limits: ResolverLimits,
    clocks: Vec<(ClockReference, ClockRegistryEntry)>,
    samples: Vec<TemporalEvidence>,
}

impl TemporalResolver {
    pub fn new(limits: ResolverLimits) -> Result<Self, TemporalError> {
        if limits.max_clocks == 0
            || limits.max_samples == 0
            || !(1..=64).contains(&limits.max_hops)
            || limits.max_expansions == 0
            || limits.max_candidates == 0
        {
            return Err(TemporalError::Invalid(
                "positive limits required; max_hops must be in 1..=64".into(),
            ));
        }
        Ok(Self {
            limits,
            clocks: Vec::new(),
            samples: Vec::new(),
        })
    }

    /// Enumerate bounded simple paths. Cycles are not revisited. Search limits
    /// fail explicitly rather than hiding an unexplored conflicting path.
    /// Distinct valid paths are checked before applying the caller's tolerance.
    pub fn resolve(&self, query: &TemporalQuery) -> Result<TemporalResolution, TemporalError> {
        self.require_clock(&query.from_clock)?;
        self.require_clock(&query.to_clock)?;
        let initial = SearchState {
            timestamp: query.timestamp_ns,
            uncertainty: query.input_uncertainty_ns,
            offset: 0,
            coverage: TimeInterval {
                start_ns: 0,
                end_ns: u64::MAX,
            },
            path: Vec::new(),
        };
        let initial = initial
            .restrict(0, u64::MAX, 0, query.input_uncertainty_ns)
            .map_err(|issue| TemporalError::Unresolved {
                issues: vec![issue],
            })?;
        let mut search = Search {
            resolver: self,
            query,
            expansions: 0,
            candidates: Vec::new(),
            issues: Vec::new(),
        };
        search.visit(
            &query.from_clock,
            initial,
            &mut vec![query.from_clock.clone()],
        )?;
        // Do not pick one convenient path when other admitted evidence disagrees.
        let lower = search
            .candidates
            .iter()
            .map(|c| i128::from(c.timestamp_ns) - i128::from(c.uncertainty_ns))
            .max();
        let upper = search
            .candidates
            .iter()
            .map(|c| i128::from(c.timestamp_ns) + i128::from(c.uncertainty_ns))
            .min();
        if matches!((lower, upper), (Some(l), Some(u)) if l > u) {
            return Err(TemporalError::Conflict {
                candidates: search.candidates,
            });
        }
        // Stable, declaration-order-independent choice, without fusing bounds.
        search.candidates.sort_by_cached_key(|c| {
            (
                c.uncertainty_ns,
                c.path.len(),
                c.path.iter().map(step_key).collect::<Vec<_>>(),
            )
        });
        if let Some(result) = search.candidates.into_iter().next() {
            if result.uncertainty_ns <= query.max_uncertainty_ns {
                return Ok(result);
            }
            search.issues.push(ResolutionIssue::UncertaintyExceeded);
        }
        Err(TemporalError::Unresolved {
            issues: search.issues,
        })
    }
}

fn step_key(step: &ResolvedStep) -> (String, String, String, u64, bool) {
    let evidence = &step.evidence;
    (
        evidence.output.peer_id.clone(),
        evidence.output.component_id.clone(),
        evidence.output.manifest_hash.clone(),
        evidence.sequence,
        step.inverted,
    )
}

#[derive(Clone)]
struct SearchState {
    timestamp: u64,
    uncertainty: u64,
    /// Cumulative offset from the original query clock; widened, never negated
    /// or accumulated in i64, even for an edge with offset i64::MIN.
    offset: i128,
    coverage: TimeInterval,
    path: Vec<ResolvedStep>,
}

impl SearchState {
    /// Constrain original query centers so their whole uncertainty interval at
    /// this edge fits [start,end]. Shift in i128 and clamp to the u64 time domain.
    fn restrict(
        mut self,
        start: u64,
        end: u64,
        offset: i128,
        uncertainty: u64,
    ) -> Result<Self, ResolutionIssue> {
        let lower = (i128::from(start) + i128::from(uncertainty) - offset)
            .max(i128::from(self.coverage.start_ns));
        let upper = (i128::from(end) - i128::from(uncertainty) - offset)
            .min(i128::from(self.coverage.end_ns));
        if lower > upper {
            return Err(ResolutionIssue::OutsideCoverage);
        }
        // Initial coverage is u64, hence the intersection is already in range.
        self.coverage = TimeInterval {
            start_ns: u64::try_from(lower).map_err(|_| ResolutionIssue::ArithmeticOverflow)?,
            end_ns: u64::try_from(upper).map_err(|_| ResolutionIssue::ArithmeticOverflow)?,
        };
        let original = i128::from(self.timestamp) - self.offset;
        if original < lower || original > upper {
            return Err(ResolutionIssue::OutsideCoverage);
        }
        Ok(self)
    }

    fn apply(self, evidence: &TemporalEvidence, inverted: bool) -> Result<Self, ResolutionIssue> {
        let OffsetSample::Available {
            offset_ns,
            uncertainty_ns,
            max_drift_ppb,
            validity,
        } = evidence.sample
        else {
            return Err(ResolutionIssue::Unavailable);
        };
        let base = uncertainty_ns.ok_or(ResolutionIssue::UnknownUncertainty)?;
        let elapsed = evidence
            .measurement_ns
            .abs_diff(validity.start_ns)
            .max(evidence.measurement_ns.abs_diff(validity.end_ns));
        let drift = match (max_drift_ppb, elapsed) {
            (_, 0) => 0,
            (None, _) => return Err(ResolutionIssue::UnknownUncertainty),
            (Some(ppb), elapsed) => {
                // ceil() in a widened domain; no unknown-as-zero or saturation.
                let bound = (u128::from(ppb) * u128::from(elapsed)).div_ceil(1_000_000_000);
                u64::try_from(bound).map_err(|_| ResolutionIssue::ArithmeticOverflow)?
            }
        };
        let edge_error = base
            .checked_add(drift)
            .ok_or(ResolutionIssue::ArithmeticOverflow)?;
        let total_error = self
            .uncertainty
            .checked_add(edge_error)
            .ok_or(ResolutionIssue::ArithmeticOverflow)?;
        let signed = i128::from(offset_ns) * if inverted { -1 } else { 1 };
        let total_offset = self.offset + signed;
        let timestamp = u64::try_from(i128::from(self.timestamp) + signed)
            .map_err(|_| ResolutionIssue::ArithmeticOverflow)?;
        let source_offset = if inverted { total_offset } else { self.offset };
        // Inverse applicability includes the edge error in the original source
        // clock, rather than naively shifting the interval into the target clock.
        let source_error = if inverted {
            total_error
        } else {
            self.uncertainty
        };
        let mut next = self.restrict(
            validity.start_ns,
            validity.end_ns,
            source_offset,
            source_error,
        )?;
        next.timestamp = timestamp;
        next.uncertainty = total_error;
        next.offset = total_offset;
        // Also keep every intermediate result plus its error in representable time.
        next = next.restrict(0, u64::MAX, total_offset, total_error)?;
        next.path.push(ResolvedStep {
            evidence: evidence.clone(),
            inverted,
            applied_uncertainty_ns: edge_error,
        });
        Ok(next)
    }
}

struct Search<'a> {
    resolver: &'a TemporalResolver,
    query: &'a TemporalQuery,
    expansions: usize,
    candidates: Vec<TemporalResolution>,
    issues: Vec<ResolutionIssue>,
}

impl Search<'_> {
    fn visit(
        &mut self,
        clock: &ClockReference,
        state: SearchState,
        visited: &mut Vec<ClockReference>,
    ) -> Result<(), TemporalError> {
        if clock == &self.query.to_clock {
            if self.candidates.len() == self.resolver.limits.max_candidates {
                return Err(TemporalError::LimitExceeded {
                    resource: "candidates",
                });
            }
            self.candidates.push(TemporalResolution {
                from_clock: self.query.from_clock.clone(),
                to_clock: self.query.to_clock.clone(),
                source_timestamp_ns: self.query.timestamp_ns,
                timestamp_ns: state.timestamp,
                uncertainty_ns: state.uncertainty,
                source_coverage: state.coverage,
                path: state.path,
            });
            return Ok(());
        }
        for evidence in &self.resolver.samples {
            // Bound all scans, including unrelated edges and cycles, not only
            // successful traversals. Dense or disconnected graphs remain bounded.
            if self.expansions == self.resolver.limits.max_expansions {
                return Err(TemporalError::LimitExceeded {
                    resource: "expansions",
                });
            }
            self.expansions += 1;
            let (inverted, next_clock) = if clock == &evidence.contract.from_clock {
                (false, &evidence.contract.to_clock)
            } else if evidence.contract.allow_inverse && clock == &evidence.contract.to_clock {
                (true, &evidence.contract.from_clock)
            } else {
                continue;
            };
            if visited.contains(next_clock) {
                continue;
            }
            match state.clone().apply(evidence, inverted) {
                Ok(next) => {
                    if state.path.len() == self.resolver.limits.max_hops {
                        return Err(TemporalError::LimitExceeded { resource: "hops" });
                    }
                    visited.push(next_clock.clone());
                    self.visit(next_clock, next, visited)?;
                    visited.pop();
                }
                Err(issue) => {
                    if !self.issues.contains(&issue) {
                        self.issues.push(issue);
                    }
                }
            }
        }
        Ok(())
    }
}
