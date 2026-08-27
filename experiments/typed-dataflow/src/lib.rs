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

mod buffer;
mod camera;
mod chunk;
mod component;
mod episode;
mod ports;
mod product;
mod pump;

pub use buffer::{
    Buffer, BufferCursor, BufferError, BufferLimits, BufferRange, BufferReader, BufferReaderStats,
    CursorRead, CursorStart, Gap, connect_buffer,
};
pub use camera::{
    AppliedResolution, CameraBufferError, CameraBufferRoller, CameraComponent, CameraError,
    CameraProductBuffer, DriverReseeded, ReseedDriver, SetResolution, VideoFrame,
};
pub use chunk::{Chunk, ChunkBuilder, ChunkBuilderConfig, ChunkBuilderError, ChunkBuilderStats};
pub use component::{
    Catalog, CatalogComponentEntry, CatalogError, CatalogOutputEntry, CatalogProductEntry,
    ComponentManifest, ComponentReference, EverySelectedDelivery, Exposure, InMemoryTransport,
    Invocation, InvocationContext, InvocationError, ManifestHash, Observable, ObservableContract,
    ObservableTarget, Observation, ObservationAccess, ObservationDelivery, ObservationError,
    ObservationEvent, ObservationHandle, ObservationStats, ObservationStatus, Operable,
    OperableContract, OutputManifest, OutputReference, OutputTransition, PayloadContract,
    PeerRuntime, ProductForm, ProductManifest, SerializedInMemoryTransport, TransportStats,
    manifest_hash, observation_input,
};
pub use episode::{Episode, EpisodeError, EpisodeState, connect_episode};
pub use ports::{
    Connection, ConnectionControl, ConnectionError, ConnectionOptions, ConnectionStats, Envelope,
    EveryFullPolicy, InputPort, OutputPort, PublishReport, StaticConnection, connect,
};
pub use product::{FiniteObservations, ProductAccessError, RetainedProduct, TimeRangeRequest};
pub use pump::{
    PumpError, PumpOptions, PumpStats, SinkFullPolicy, StreamPump, connect_direct_latest_pump,
};
