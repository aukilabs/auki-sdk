//! Time-based hysteresis for turning raw point-cloud observations into a
//! stable occupancy map.

use std::{collections::BTreeMap, time::Duration};

use auki_datatypes::map::{ColorEvidenceDelta, MapUpdate, VoxelChunkUpdate, VoxelDelta};

/// Temporal reliability policy for voxel-map observations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoxelPersistenceConfig {
    /// A candidate must be observed across at least this much time before it
    /// becomes part of the persistent map.
    pub confirmation_duration: Duration,
    /// Minimum number of distinct point-cloud frames that must observe the
    /// candidate during `confirmation_duration`.
    pub minimum_confirmation_observations: u32,
    /// Maximum gap between observations that still counts as one continuous
    /// occupied or free-space streak.
    pub maximum_observation_gap: Duration,
    /// A confirmed voxel must be traversed as free space continuously for this
    /// long before it is removed from the persistent map.
    pub clearing_duration: Duration,
}

impl Default for VoxelPersistenceConfig {
    fn default() -> Self {
        Self {
            confirmation_duration: Duration::from_secs(3),
            minimum_confirmation_observations: 6,
            maximum_observation_gap: Duration::from_millis(500),
            clearing_duration: Duration::from_secs(1),
        }
    }
}

impl VoxelPersistenceConfig {
    pub(crate) fn validate(self) -> bool {
        self.confirmation_duration > Duration::ZERO
            && self.minimum_confirmation_observations >= 2
            && self.maximum_observation_gap > Duration::ZERO
            && self.clearing_duration > Duration::ZERO
            && duration_ns(self.confirmation_duration).is_some()
            && duration_ns(self.maximum_observation_gap).is_some()
            && duration_ns(self.clearing_duration).is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct VoxelKey {
    chunk_x: i32,
    chunk_y: i32,
    chunk_z: i32,
    x: u32,
    y: u32,
    z: u32,
}

#[derive(Debug, Default)]
struct FrameObservation {
    occupied: bool,
    color_sums: [f64; 3],
    color_weight: f64,
}

#[derive(Debug)]
struct Candidate {
    first_hit_ns: i64,
    last_hit_ns: i64,
    hit_frames: u32,
    color_sums: [f64; 3],
    color_frames: u32,
}

#[derive(Debug)]
struct Confirmed {
    free_streak: Option<(i64, i64)>,
}

#[derive(Debug)]
enum VoxelState {
    Candidate(Candidate),
    Confirmed(Confirmed),
}

/// Stateful per-Mapper filter. Raw evidence is normalized to one occupied or
/// free observation per voxel and input frame before temporal hysteresis.
pub(crate) struct VoxelPersistenceFilter {
    config: VoxelPersistenceConfig,
    occupied_delta: f32,
    states: BTreeMap<VoxelKey, VoxelState>,
}

impl VoxelPersistenceFilter {
    pub(crate) fn new(config: VoxelPersistenceConfig, occupied_delta: f32) -> Self {
        debug_assert!(config.validate());
        Self {
            config,
            occupied_delta,
            states: BTreeMap::new(),
        }
    }

    pub(crate) fn apply(&mut self, timestamp_ns: i64, raw: MapUpdate) -> MapUpdate {
        let observations = normalize_frame(raw);
        let maximum_gap_ns = duration_ns(self.config.maximum_observation_gap).unwrap();
        self.states.retain(|_, state| match state {
            VoxelState::Candidate(candidate) => {
                timestamp_ns.saturating_sub(candidate.last_hit_ns) <= maximum_gap_ns
            }
            VoxelState::Confirmed(_) => true,
        });

        let mut emitted = Vec::new();
        for (key, observation) in observations {
            let prior = self.states.remove(&key);
            let next = if observation.occupied {
                self.observe_hit(timestamp_ns, key, observation, prior, &mut emitted)
            } else {
                self.observe_free(timestamp_ns, key, prior, &mut emitted)
            };
            if let Some(next) = next {
                self.states.insert(key, next);
            }
        }
        map_update(emitted)
    }

    fn observe_hit(
        &self,
        timestamp_ns: i64,
        key: VoxelKey,
        observation: FrameObservation,
        prior: Option<VoxelState>,
        emitted: &mut Vec<(VoxelKey, VoxelDelta)>,
    ) -> Option<VoxelState> {
        if matches!(prior, Some(VoxelState::Confirmed(_))) {
            return Some(VoxelState::Confirmed(Confirmed { free_streak: None }));
        }

        let maximum_gap_ns = duration_ns(self.config.maximum_observation_gap).unwrap();
        let mut candidate = match prior {
            Some(VoxelState::Candidate(candidate))
                if timestamp_ns.saturating_sub(candidate.last_hit_ns) <= maximum_gap_ns =>
            {
                candidate
            }
            _ => Candidate {
                first_hit_ns: timestamp_ns,
                last_hit_ns: timestamp_ns,
                hit_frames: 0,
                color_sums: [0.0; 3],
                color_frames: 0,
            },
        };
        candidate.last_hit_ns = timestamp_ns;
        candidate.hit_frames = candidate.hit_frames.saturating_add(1);
        if observation.color_weight > 0.0 {
            for (sum, observed) in candidate.color_sums.iter_mut().zip(observation.color_sums) {
                *sum += observed / observation.color_weight;
            }
            candidate.color_frames = candidate.color_frames.saturating_add(1);
        }

        let confirmation_ns = duration_ns(self.config.confirmation_duration).unwrap();
        let confirmed = timestamp_ns.saturating_sub(candidate.first_hit_ns) >= confirmation_ns
            && candidate.hit_frames >= self.config.minimum_confirmation_observations;
        if !confirmed {
            return Some(VoxelState::Candidate(candidate));
        }

        let color = (candidate.color_frames > 0).then(|| {
            let denominator = f64::from(candidate.color_frames);
            ColorEvidenceDelta {
                red_sum_delta: (candidate.color_sums[0] / denominator) as f32,
                green_sum_delta: (candidate.color_sums[1] / denominator) as f32,
                blue_sum_delta: (candidate.color_sums[2] / denominator) as f32,
                weight_delta: 1.0,
            }
        });
        emitted.push((key, voxel_delta(key, self.occupied_delta, color)));
        Some(VoxelState::Confirmed(Confirmed { free_streak: None }))
    }

    fn observe_free(
        &self,
        timestamp_ns: i64,
        key: VoxelKey,
        prior: Option<VoxelState>,
        emitted: &mut Vec<(VoxelKey, VoxelDelta)>,
    ) -> Option<VoxelState> {
        let Some(VoxelState::Confirmed(mut confirmed)) = prior else {
            // Free space cancels an unconfirmed candidate without ever adding
            // negative-only voxels to the persistent map.
            return None;
        };
        let maximum_gap_ns = duration_ns(self.config.maximum_observation_gap).unwrap();
        let (first_free_ns, last_free_ns) = confirmed
            .free_streak
            .filter(|(_, last)| timestamp_ns.saturating_sub(*last) <= maximum_gap_ns)
            .unwrap_or((timestamp_ns, timestamp_ns));
        let clearing_ns = duration_ns(self.config.clearing_duration).unwrap();
        if timestamp_ns.saturating_sub(first_free_ns) >= clearing_ns {
            emitted.push((key, voxel_delta(key, -self.occupied_delta, None)));
            None
        } else {
            confirmed.free_streak = Some((first_free_ns, timestamp_ns.max(last_free_ns)));
            Some(VoxelState::Confirmed(confirmed))
        }
    }
}

fn normalize_frame(raw: MapUpdate) -> BTreeMap<VoxelKey, FrameObservation> {
    let mut observations = BTreeMap::<VoxelKey, FrameObservation>::new();
    for chunk in raw.voxel_chunks {
        for voxel in chunk.voxels {
            let key = VoxelKey {
                chunk_x: chunk.chunk_x,
                chunk_y: chunk.chunk_y,
                chunk_z: chunk.chunk_z,
                x: voxel.x,
                y: voxel.y,
                z: voxel.z,
            };
            let observation = observations.entry(key).or_default();
            if voxel.occupancy_delta > 0.0 {
                observation.occupied = true;
                if let Some(color) = voxel.color {
                    observation.color_sums[0] += f64::from(color.red_sum_delta);
                    observation.color_sums[1] += f64::from(color.green_sum_delta);
                    observation.color_sums[2] += f64::from(color.blue_sum_delta);
                    observation.color_weight += f64::from(color.weight_delta);
                }
            }
        }
    }
    observations
}

fn voxel_delta(
    key: VoxelKey,
    occupancy_delta: f32,
    color: Option<ColorEvidenceDelta>,
) -> VoxelDelta {
    VoxelDelta {
        x: key.x,
        y: key.y,
        z: key.z,
        occupancy_delta,
        semantics: Vec::new(),
        color,
    }
}

fn map_update(deltas: Vec<(VoxelKey, VoxelDelta)>) -> MapUpdate {
    let mut chunks = BTreeMap::<(i32, i32, i32), Vec<VoxelDelta>>::new();
    for (key, delta) in deltas {
        chunks
            .entry((key.chunk_x, key.chunk_y, key.chunk_z))
            .or_default()
            .push(delta);
    }
    MapUpdate {
        voxel_chunks: chunks
            .into_iter()
            .map(|((chunk_x, chunk_y, chunk_z), voxels)| VoxelChunkUpdate {
                chunk_x,
                chunk_y,
                chunk_z,
                voxels,
            })
            .collect(),
        checkpoint: None,
    }
}

fn duration_ns(duration: Duration) -> Option<i64> {
    i64::try_from(duration.as_nanos()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(delta: f32, duplicates: usize) -> MapUpdate {
        MapUpdate {
            voxel_chunks: vec![VoxelChunkUpdate {
                chunk_x: 0,
                chunk_y: 0,
                chunk_z: 0,
                voxels: (0..duplicates)
                    .map(|_| VoxelDelta {
                        x: 1,
                        y: 2,
                        z: 3,
                        occupancy_delta: delta,
                        semantics: Vec::new(),
                        color: None,
                    })
                    .collect(),
            }],
            checkpoint: None,
        }
    }

    fn config() -> VoxelPersistenceConfig {
        VoxelPersistenceConfig {
            confirmation_duration: Duration::from_secs(2),
            minimum_confirmation_observations: 3,
            maximum_observation_gap: Duration::from_secs(1),
            clearing_duration: Duration::from_secs(1),
        }
    }

    #[test]
    fn transient_hits_never_enter_the_map() {
        let mut filter = VoxelPersistenceFilter::new(config(), 0.8);
        assert!(filter.apply(0, raw(0.8, 100)).voxel_chunks.is_empty());
        assert!(
            filter
                .apply(500_000_000, raw(-0.2, 100))
                .voxel_chunks
                .is_empty()
        );
    }

    #[test]
    fn repeated_frames_promote_once_and_sustained_free_space_removes_once() {
        let mut filter = VoxelPersistenceFilter::new(config(), 0.8);
        assert!(filter.apply(0, raw(0.8, 50)).voxel_chunks.is_empty());
        assert!(
            filter
                .apply(1_000_000_000, raw(0.8, 50))
                .voxel_chunks
                .is_empty()
        );
        let promoted = filter.apply(2_000_000_000, raw(0.8, 50));
        assert_eq!(promoted.voxel_chunks[0].voxels.len(), 1);
        assert_eq!(promoted.voxel_chunks[0].voxels[0].occupancy_delta, 0.8);
        assert!(
            filter
                .apply(3_000_000_000, raw(0.8, 50))
                .voxel_chunks
                .is_empty()
        );

        assert!(
            filter
                .apply(4_000_000_000, raw(-0.2, 50))
                .voxel_chunks
                .is_empty()
        );
        let removed = filter.apply(5_000_000_000, raw(-0.2, 50));
        assert_eq!(removed.voxel_chunks[0].voxels.len(), 1);
        assert_eq!(removed.voxel_chunks[0].voxels[0].occupancy_delta, -0.8);
    }

    #[test]
    fn observation_gap_resets_confirmation_streak() {
        let mut filter = VoxelPersistenceFilter::new(config(), 0.8);
        filter.apply(0, raw(0.8, 1));
        filter.apply(2_000_000_000, raw(0.8, 1));
        assert!(
            filter
                .apply(3_000_000_000, raw(0.8, 1))
                .voxel_chunks
                .is_empty()
        );
    }
}
