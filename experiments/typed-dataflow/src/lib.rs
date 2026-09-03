//! Network-independent experiment for typed, in-process SDK dataflow.
//!
//! The crate deliberately keeps executable Components separate from retained
//! data products. Components connect through typed ports; a [`Buffer`] is an
//! explicit owning subscriber; an [`Episode`] is a deliberately retained
//! interval; and a [`StreamPump`] follows a Buffer for one recipient.
//!
//! Port types make incompatible connections fail at compile time:
//!
//! ```compile_fail
//! use auki_typed_dataflow_experiment::{connect, ConnectionOptions, InputPort, OutputPort};
//!
//! struct CameraFrame;
//! struct AudioFrame;
//!
//! let camera = OutputPort::<CameraFrame>::new("camera.frames");
//! let microphone = InputPort::<AudioFrame>::new("microphone.frames", |_| {});
//! let _connection = connect(&camera, &microphone, ConnectionOptions::InlineEvery).unwrap();
//! ```
//!
//! Typed Operables reject the wrong instruction type:
//!
//! ```compile_fail
//! use auki_typed_dataflow_experiment::{
//!     ComponentManifest, Exposure, InvocationContext, InvocationError, Operable,
//! };
//!
//! struct SetResolution;
//! struct PlayAudio;
//! struct Applied;
//!
//! let owner = ComponentManifest {
//!     schema: "demo/v1".into(), peer_id: "p".into(), component_id: "camera".into(),
//!     observables: vec![], operables: vec![],
//! }.reference();
//! let resize = Operable::<SetResolution, Applied>::new(
//!     "resize", owner, Exposure::Local, |_| true, |_, _| Ok(Applied),
//! );
//! let context = InvocationContext {
//!     invocation_id: "i".into(), caller_peer_id: "p".into(),
//!     caller_component_id: "controller".into(),
//! };
//! let _ = resize.invoke(context, PlayAudio);
//! ```
//!
//! Catalog mutation is intentionally unavailable to applications; entries
//! originate from live runtime handles:
//!
//! ```compile_fail
//! use auki_typed_dataflow_experiment::{ComponentManifest, PeerRuntime};
//!
//! let peer = PeerRuntime::new("peer-a");
//! peer.catalog().register_component(ComponentManifest {
//!     schema: "made-up/v1".into(), peer_id: "peer-a".into(),
//!     component_id: "not-live".into(), observables: vec![], operables: vec![],
//! });
//! ```
//!
//! Every observation has a source-clock timestamp. Missing source timestamps
//! are rejected by the type system rather than interpreted as zero:
//!
//! ```compile_fail
//! use auki_typed_dataflow_experiment::Envelope;
//!
//! let _missing = Envelope::new(0, None, 42_u64);
//! ```

mod buffer;
mod camera;
mod chunk;
mod component;
mod episode;
mod ports;
mod product;
mod pump;
mod runtime;

pub use buffer::{
    Buffer, BufferCursor, BufferError, BufferLimits, BufferRange, BufferReader, BufferReaderStats,
    BufferTimePolicy, CursorRead, CursorStart, DurationTimeBasis, Gap, SourceTimestampPolicy,
    connect_buffer,
};
pub use camera::{
    AppliedResolution, CameraBufferCapture, CameraBufferError, CameraComponent, CameraError,
    CameraProductBuffer, DriverReseeded, ReseedDriver, SetResolution, VideoFrame,
};
pub use chunk::{Chunk, ChunkBuilder, ChunkBuilderConfig, ChunkBuilderError, ChunkBuilderStats};
pub use component::{
    AudioLayout, AudioPayloadContract, AudioSampleFormat, CameraPayloadContract, Catalog,
    CatalogComponentEntry, CatalogError, CatalogOutputEntry, CatalogProductEntry,
    ComponentManifest, ComponentReference, EverySelectedDelivery, Exposure, GaugePayloadContract,
    InMemoryTransport, Invocation, InvocationContext, InvocationError, InvocationHandle,
    InvocationOptions, InvocationOrdering, InvocationStatus, ManifestHash, Observable,
    ObservableContract, Observation, ObservationAccess, ObservationDelivery, ObservationEnd,
    ObservationEndReason, ObservationError, ObservationEvent, ObservationHandle, ObservationStats,
    ObservationStatus, Operable, OperableContract, OutputManifest, OutputReference,
    PayloadContract, PeerRuntime, ProductForm, ProductManifest, ProductState,
    SerializedInMemoryTransport, StructuredPayloadContract, TransportStats, manifest_hash,
    observation_input,
};
pub use episode::{Episode, EpisodeError, EpisodeState, connect_episode};
pub use ports::{
    ComponentError, Connection, ConnectionControl, ConnectionError, ConnectionOptions,
    ConnectionStats, Envelope, EveryFullPolicy, InputPort, OutputPort, PublishReport,
    SchedulerError, SharedDelivery, SharedDispatcher, SharedScheduler, SharedSchedulerStats,
    StaticConnection, connect, connect_shared,
};
pub use product::{
    BufferProductCapture, EpisodeProduct, EpisodeProductCapture, FiniteObservations,
    ProductAccessError, ProductCaptureError, RetainedProduct, TimeRangeRequest,
};
pub use pump::{
    PumpError, PumpOptions, PumpStats, SinkFullPolicy, StreamPump, connect_direct_latest_pump,
};
pub use runtime::{
    Component, ComponentBuildError, ComponentSpec, ConfiguredObservable,
    ConfiguredObservableReplacement, ConfiguredObservableSpec, ContractType, PublishError,
};
