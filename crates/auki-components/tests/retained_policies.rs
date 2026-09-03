use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use auki_components::{
    Buffer, BufferError, BufferLimits, BufferTimePolicy, ComponentRuntime, ComponentSpec,
    ConfiguredObservableSpec, ContractType, DurationTimeBasis, Envelope, Episode, Exposure,
    ObservableContract, ObservationAccess, ObservationDelivery, ObservationEvent, PayloadContract,
    SerializedInMemoryTransport, SourceTimestampPolicy, StructuredPayloadContract,
    observation_input,
};
use serde::de::{SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

fn limits(duration_ns: Option<u64>) -> BufferLimits {
    BufferLimits {
        max_entries: Some(16),
        max_bytes: None,
        target_duration: duration_ns.map(Duration::from_nanos),
    }
}

#[test]
fn source_timestamp_policy_handles_equal_and_out_of_order_values_explicitly() {
    let strict = Buffer::with_limits_and_time_policy(
        "strict",
        limits(None),
        BufferTimePolicy {
            source_timestamps: SourceTimestampPolicy::StrictlyIncreasing,
            duration_basis: DurationTimeBasis::SourceTimestamp,
        },
        |_| 1,
    )
    .unwrap();
    strict
        .append_shared(Arc::new(Envelope::new(0, 10, 0)))
        .unwrap();
    assert!(matches!(
        strict.append_shared(Arc::new(Envelope::new(1, 10, 1))),
        Err(BufferError::NonMonotonicTimestamp {
            previous: 10,
            incoming: 10,
            ..
        })
    ));

    let equal = Buffer::with_limits_and_time_policy(
        "equal",
        limits(None),
        BufferTimePolicy {
            source_timestamps: SourceTimestampPolicy::NonDecreasing,
            duration_basis: DurationTimeBasis::SourceTimestamp,
        },
        |_| 1,
    )
    .unwrap();
    equal
        .append_shared(Arc::new(Envelope::new(0, 10, 0)))
        .unwrap();
    equal
        .append_shared(Arc::new(Envelope::new(1, 10, 1)))
        .unwrap();
    assert!(matches!(
        equal.append_shared(Arc::new(Envelope::new(2, 9, 2))),
        Err(BufferError::NonMonotonicTimestamp {
            previous: 10,
            incoming: 9,
            ..
        })
    ));
}

#[test]
fn unordered_source_time_requires_arrival_based_duration_eviction() {
    assert!(matches!(
        Buffer::<u64>::with_limits_and_time_policy(
            "invalid",
            limits(Some(10)),
            BufferTimePolicy {
                source_timestamps: SourceTimestampPolicy::Unordered,
                duration_basis: DurationTimeBasis::SourceTimestamp,
            },
            |_| 1,
        ),
        Err(BufferError::UnorderedSourceDuration)
    ));

    let buffer = Buffer::with_limits_and_time_policy(
        "arrival",
        limits(Some(10)),
        BufferTimePolicy {
            source_timestamps: SourceTimestampPolicy::Unordered,
            duration_basis: DurationTimeBasis::ArrivalTime,
        },
        |_| 1,
    )
    .unwrap();
    let start = Instant::now();
    buffer
        .append_shared_at(Arc::new(Envelope::new(0, 100, 0)), start)
        .unwrap();
    buffer
        .append_shared_at(
            Arc::new(Envelope::new(1, 50, 1)),
            start + Duration::from_nanos(5),
        )
        .unwrap();
    buffer
        .append_shared_at(
            Arc::new(Envelope::new(2, 75, 2)),
            start + Duration::from_nanos(11),
        )
        .unwrap();

    assert_eq!(buffer.range().first_sequence, Some(1));
    assert_eq!(
        buffer
            .snapshot_time_ns(70, 100)
            .into_iter()
            .map(|entry| entry.sequence)
            .collect::<Vec<_>>(),
        vec![2]
    );
}

struct ExternalAllocation {
    bytes: usize,
    recycled: Arc<AtomicBool>,
}

impl Drop for ExternalAllocation {
    fn drop(&mut self) {
        self.recycled.store(true, Ordering::Release);
    }
}

struct ExternalFrame {
    allocation: Arc<ExternalAllocation>,
}

#[test]
fn external_leases_survive_eviction_and_count_toward_hard_byte_limits() {
    let buffer = Buffer::with_limits(
        "external",
        BufferLimits {
            max_entries: None,
            max_bytes: Some(150),
            target_duration: None,
        },
        |frame: &ExternalFrame| frame.allocation.bytes,
    )
    .unwrap();

    let first_recycled = Arc::new(AtomicBool::new(false));
    let first_allocation = Arc::new(ExternalAllocation {
        bytes: 100,
        recycled: Arc::clone(&first_recycled),
    });
    buffer
        .append_shared(Arc::new(Envelope::new(
            0,
            0,
            ExternalFrame {
                allocation: Arc::clone(&first_allocation),
            },
        )))
        .unwrap();
    drop(first_allocation);
    let outside_lease = Arc::clone(&buffer.snapshot(0, 0)[0].payload.allocation);

    buffer
        .append_shared(Arc::new(Envelope::new(
            1,
            1,
            ExternalFrame {
                allocation: Arc::new(ExternalAllocation {
                    bytes: 100,
                    recycled: Arc::new(AtomicBool::new(false)),
                }),
            },
        )))
        .unwrap();
    assert_eq!(buffer.range().first_sequence, Some(1));
    assert_eq!(buffer.range().retained_payload_bytes, 100);
    assert!(!first_recycled.load(Ordering::Acquire));
    drop(outside_lease);
    assert!(first_recycled.load(Ordering::Acquire));

    assert!(matches!(
        buffer.append_shared(Arc::new(Envelope::new(
            2,
            2,
            ExternalFrame {
                allocation: Arc::new(ExternalAllocation {
                    bytes: 200,
                    recycled: Arc::new(AtomicBool::new(false)),
                }),
            },
        ))),
        Err(BufferError::PayloadExceedsByteLimit {
            payload_bytes: 200,
            limit: 150
        })
    ));
}

#[test]
fn episode_and_buffer_share_retained_storage_and_release_it_independently() {
    let recycled = Arc::new(AtomicBool::new(false));
    let buffer = Buffer::new("window", 1).unwrap();
    let allocation = Arc::new(ExternalAllocation {
        bytes: 100,
        recycled: Arc::clone(&recycled),
    });
    buffer
        .append_shared(Arc::new(Envelope::new(
            0,
            0,
            ExternalFrame {
                allocation: Arc::clone(&allocation),
            },
        )))
        .unwrap();
    drop(allocation);
    let episode = Episode::promote("event", &buffer, 0, 0).unwrap();
    assert!(Arc::ptr_eq(
        &buffer.snapshot(0, 0)[0],
        &episode.snapshot()[0]
    ));

    buffer
        .append_shared(Arc::new(Envelope::new(
            1,
            1,
            ExternalFrame {
                allocation: Arc::new(ExternalAllocation {
                    bytes: 1,
                    recycled: Arc::new(AtomicBool::new(false)),
                }),
            },
        )))
        .unwrap();
    assert!(!recycled.load(Ordering::Acquire));
    drop(episode);
    assert!(recycled.load(Ordering::Acquire));
}

struct ExternalTransportAllocation {
    bytes: Arc<[u8]>,
    readbacks: Arc<AtomicUsize>,
}

#[derive(Clone)]
struct ExternalBytes {
    allocation: Arc<ExternalTransportAllocation>,
}

impl ContractType for ExternalBytes {
    const DATATYPE: &'static str = "external_bytes";
}

impl Serialize for ExternalBytes {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.allocation.readbacks.fetch_add(1, Ordering::Relaxed);
        self.allocation.bytes.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ExternalBytes {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct BytesVisitor;
        impl<'de> Visitor<'de> for BytesVisitor {
            type Value = Vec<u8>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a byte sequence")
            }

            fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut bytes = Vec::with_capacity(sequence.size_hint().unwrap_or(0));
                while let Some(byte) = sequence.next_element()? {
                    bytes.push(byte);
                }
                Ok(bytes)
            }
        }

        let bytes = deserializer.deserialize_seq(BytesVisitor)?;
        Ok(Self {
            allocation: Arc::new(ExternalTransportAllocation {
                bytes: bytes.into(),
                readbacks: Arc::new(AtomicUsize::new(0)),
            }),
        })
    }
}

#[test]
fn external_storage_fanout_is_zero_copy_locally_and_readback_is_explicit_for_transport() {
    let peer = ComponentRuntime::new("peer-a");
    let component = peer
        .component(
            ComponentSpec::new("external-camera").observable(ObservableContract {
                name: "frames".to_owned(),
                datatype: "external_bytes".to_owned(),
                schema: "demo.external-bytes/v1".to_owned(),
                access: vec![ObservationAccess::FollowNew],
                exposure: Exposure::Cluster,
            }),
        )
        .unwrap();
    let output = component
        .configured_observable::<ExternalBytes>(ConfiguredObservableSpec::new(
            "frames",
            "frames-1",
            "peer-a.session-clock",
            PayloadContract::Structured(StructuredPayloadContract {
                modality: "camera".to_owned(),
                datatype: "external_bytes".to_owned(),
                schema: "demo.external-bytes/v1".to_owned(),
                observes: "visible_light".to_owned(),
                unit: None,
            }),
        ))
        .unwrap();
    component.expose().unwrap();

    let local = Arc::new(Mutex::new(None));
    let local_sink = Arc::clone(&local);
    let local_input = observation_input("local", move |event: &ObservationEvent<ExternalBytes>| {
        if let ObservationEvent::Observation(observation) = event {
            *local_sink.lock().unwrap() = Some(Arc::clone(&observation.payload.allocation));
        }
    });
    let _local_handle = output
        .observable()
        .follow_new(&local_input, ObservationDelivery::inline_every_selected())
        .unwrap();

    let remote = Arc::new(Mutex::new(None));
    let remote_sink = Arc::clone(&remote);
    let remote_input =
        observation_input("remote", move |event: &ObservationEvent<ExternalBytes>| {
            if let ObservationEvent::Observation(observation) = event {
                *remote_sink.lock().unwrap() = Some(Arc::clone(&observation.payload.allocation));
            }
        });
    let transport = SerializedInMemoryTransport::default();
    let _remote_handle = transport
        .follow_new(
            &output.observable(),
            &remote_input,
            ObservationDelivery::inline_every_selected(),
        )
        .unwrap();

    let readbacks = Arc::new(AtomicUsize::new(0));
    let allocation = Arc::new(ExternalTransportAllocation {
        bytes: Arc::from(vec![7_u8; 1024]),
        readbacks: Arc::clone(&readbacks),
    });
    output
        .publish(
            0,
            Arc::new(ExternalBytes {
                allocation: Arc::clone(&allocation),
            }),
        )
        .unwrap();

    assert!(Arc::ptr_eq(
        &allocation,
        local.lock().unwrap().as_ref().unwrap()
    ));
    assert!(!Arc::ptr_eq(
        &allocation,
        remote.lock().unwrap().as_ref().unwrap()
    ));
    assert_eq!(readbacks.load(Ordering::Relaxed), 1);
    assert!(transport.stats().encoded_bytes >= 1024);
}
