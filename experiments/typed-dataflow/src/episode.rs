use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::buffer::{Buffer, BufferRange};
use crate::ports::{
    ComponentError, Connection, ConnectionStats, DeliveryStatus, Endpoint, EndpointKind, Envelope,
    OutputPort,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EpisodeState {
    Active,
    Concluded {
        last_sequence: Option<u64>,
        end_timestamp_ns: u64,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EpisodeError {
    InvalidRange {
        first: u64,
        last: u64,
    },
    RangeUnavailable {
        requested_first: u64,
        requested_last: u64,
        available: BufferRange,
    },
    NonMonotonicSequence {
        previous: u64,
        incoming: u64,
    },
    Concluded,
}

impl fmt::Display for EpisodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRange { first, last } => {
                write!(formatter, "invalid Episode range [{first}, {last}]")
            }
            Self::RangeUnavailable {
                requested_first,
                requested_last,
                available,
            } => write!(
                formatter,
                "Episode range [{requested_first}, {requested_last}] is not retained; available range is {:?}..={:?}",
                available.first_sequence, available.last_sequence
            ),
            Self::NonMonotonicSequence { previous, incoming } => write!(
                formatter,
                "incoming sequence {incoming} is not newer than {previous}"
            ),
            Self::Concluded => formatter.write_str("Episode has already concluded"),
        }
    }
}

impl std::error::Error for EpisodeError {}

/// A deliberately retained interval which is active until explicitly concluded.
pub struct Episode<T> {
    inner: Arc<EpisodeInner<T>>,
}

struct EpisodeInner<T> {
    name: Arc<str>,
    state: Mutex<EpisodeStorage<T>>,
}

struct EpisodeStorage<T> {
    entries: Vec<Arc<Envelope<T>>>,
    lifecycle: EpisodeState,
}

impl<T> Clone for Episode<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<T> fmt::Debug for Episode<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Episode")
            .field("name", &self.inner.name)
            .field("state", &self.state())
            .field("len", &self.len())
            .finish()
    }
}

impl<T> Episode<T> {
    pub fn promote(
        name: impl Into<Arc<str>>,
        buffer: &Buffer<T>,
        first: u64,
        last: u64,
    ) -> Result<Self, EpisodeError> {
        if first > last {
            return Err(EpisodeError::InvalidRange { first, last });
        }
        let available = buffer.range();
        let entries = buffer.snapshot(first, last);
        if entries.first().map(|entry| entry.sequence) != Some(first)
            || entries.last().map(|entry| entry.sequence) != Some(last)
        {
            return Err(EpisodeError::RangeUnavailable {
                requested_first: first,
                requested_last: last,
                available,
            });
        }

        Ok(Self {
            inner: Arc::new(EpisodeInner {
                name: name.into(),
                state: Mutex::new(EpisodeStorage {
                    entries,
                    lifecycle: EpisodeState::Active,
                }),
            }),
        })
    }

    pub fn empty(name: impl Into<Arc<str>>) -> Self {
        Self {
            inner: Arc::new(EpisodeInner {
                name: name.into(),
                state: Mutex::new(EpisodeStorage {
                    entries: Vec::new(),
                    lifecycle: EpisodeState::Active,
                }),
            }),
        }
    }

    pub fn name(&self) -> &str {
        &self.inner.name
    }

    pub fn append_shared(&self, envelope: Arc<Envelope<T>>) -> Result<(), EpisodeError> {
        let mut state = self.inner.state.lock().unwrap();
        if !matches!(state.lifecycle, EpisodeState::Active) {
            return Err(EpisodeError::Concluded);
        }
        if let Some(previous) = state.entries.last().map(|entry| entry.sequence)
            && envelope.sequence <= previous
        {
            return Err(EpisodeError::NonMonotonicSequence {
                previous,
                incoming: envelope.sequence,
            });
        }
        state.entries.push(envelope);
        Ok(())
    }

    pub fn conclude(&self, end_timestamp_ns: u64) -> Result<(), EpisodeError> {
        let mut state = self.inner.state.lock().unwrap();
        if !matches!(state.lifecycle, EpisodeState::Active) {
            return Err(EpisodeError::Concluded);
        }
        state.lifecycle = EpisodeState::Concluded {
            last_sequence: state.entries.last().map(|entry| entry.sequence),
            end_timestamp_ns,
        };
        Ok(())
    }

    pub fn state(&self) -> EpisodeState {
        self.inner.state.lock().unwrap().lifecycle
    }

    pub fn len(&self) -> usize {
        self.inner.state.lock().unwrap().entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn snapshot(&self) -> Vec<Arc<Envelope<T>>> {
        self.inner.state.lock().unwrap().entries.clone()
    }
}

struct EpisodeEndpoint<T> {
    episode: Episode<T>,
    accepted: AtomicU64,
    overruns: AtomicU64,
    closed: AtomicBool,
    failure: Mutex<Option<ComponentError>>,
}

impl<T: Send + Sync + 'static> Endpoint<T> for EpisodeEndpoint<T> {
    fn kind(&self) -> EndpointKind {
        EndpointKind::Owning
    }

    fn deliver_owned(&self, envelope: Arc<Envelope<T>>) -> DeliveryStatus {
        if self.closed.load(Ordering::Acquire) {
            return DeliveryStatus::Disconnected;
        }
        match self.episode.append_shared(envelope) {
            Ok(()) => {
                self.accepted.fetch_add(1, Ordering::Relaxed);
                DeliveryStatus::Accepted
            }
            Err(error) => {
                self.overruns.fetch_add(1, Ordering::Relaxed);
                self.closed.store(true, Ordering::Release);
                *self.failure.lock().unwrap() = Some(ComponentError::Reported(error.to_string()));
                DeliveryStatus::Failed
            }
        }
    }

    fn stats(&self) -> ConnectionStats {
        let accepted = self.accepted.load(Ordering::Relaxed);
        ConnectionStats {
            accepted,
            delivered: accepted,
            replaced: 0,
            overruns: self.overruns.load(Ordering::Relaxed),
            closed: self.closed.load(Ordering::Acquire),
            failed: self.failure.lock().unwrap().is_some(),
        }
    }

    fn failure(&self) -> Option<ComponentError> {
        self.failure.lock().unwrap().clone()
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    fn close(&self) {
        self.closed.store(true, Ordering::Release);
    }
}

/// Continues an active Episode from subsequent output values.
pub fn connect_episode<T: Send + Sync + 'static>(
    from: &OutputPort<T>,
    episode: &Episode<T>,
) -> Connection<T> {
    from.attach(Arc::new(EpisodeEndpoint {
        episode: episode.clone(),
        accepted: AtomicU64::new(0),
        overruns: AtomicU64::new(0),
        closed: AtomicBool::new(false),
        failure: Mutex::new(None),
    }))
}
