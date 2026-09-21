use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, bail, ensure};
use auki_component_protocol::{
    CatalogResponse, ComponentProtocolClient, ComponentProtocolEndpoint, ObservationStart,
    RemoteObservationEvent, RemoteProductSubscription,
};
use auki_components::{
    BufferLimits, Component, ComponentRuntime, ComponentSpec, ConfiguredBufferInput, CursorStart,
    Exposure, InputPort, Observation, PayloadContract, ProductForm, ProductInputContract,
};
use auki_sdk::AukiPeer;
use tokio::sync::Notify;

use crate::{
    AudioBlock, AudioFormat, LEVEL_BUFFER_BLOCKS, LEVEL_PRODUCT, LEVEL_SCHEMA, VolumePeer,
    local::LocalPair, synthetic_block,
};

#[derive(Clone, Copy)]
pub struct RunOptions {
    pub blocks: u64,
    pub paced: bool,
    pub microphone: bool,
    pub print_levels: bool,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            blocks: 300,
            paced: true,
            microphone: false,
            print_levels: true,
        }
    }
}

#[derive(Debug)]
pub struct LocalReport {
    pub peer_id: String,
    pub audio_entries: usize,
    pub volume_observations: usize,
    pub concluded: bool,
}

#[derive(Debug)]
pub struct RemoteReport {
    pub source_peer_id: String,
    pub observations: u64,
    pub gap_entries: u64,
    pub last_sequence: Option<u64>,
    pub latest_dbfs: Option<f64>,
}

#[derive(Debug)]
pub struct DemoReport {
    pub a: LocalReport,
    pub b: LocalReport,
    pub a_observed_b: RemoteReport,
    pub b_observed_a: RemoteReport,
}

enum SourceA {
    Synthetic,
    #[cfg(feature = "microphone")]
    Microphone(crate::microphone::Microphone),
}

impl SourceA {
    fn prepare(microphone: bool) -> Result<Self> {
        if microphone {
            #[cfg(feature = "microphone")]
            return Ok(Self::Microphone(crate::microphone::Microphone::prepare()?));
            #[cfg(not(feature = "microphone"))]
            bail!("rebuild with --features microphone to use --microphone");
        }
        Ok(Self::Synthetic)
    }

    fn format(&self) -> AudioFormat {
        match self {
            Self::Synthetic => AudioFormat::new(48_000, 1).unwrap(),
            #[cfg(feature = "microphone")]
            Self::Microphone(microphone) => microphone.format(),
        }
    }

    fn start(&mut self) -> Result<()> {
        #[cfg(feature = "microphone")]
        if let Self::Microphone(microphone) = self {
            microphone.start()?;
        }
        Ok(())
    }

    async fn next(&mut self) -> Result<AudioBlock> {
        match self {
            Self::Synthetic => Ok(synthetic_block(self.format(), 0.5)),
            #[cfg(feature = "microphone")]
            Self::Microphone(microphone) => microphone.next().await,
        }
    }
}

/// Two independent authenticated peers in one process. The same adapter works
/// with a caller-owned real AukiPeer; this runner never contacts shared services.
pub async fn run(options: RunOptions) -> Result<DemoReport> {
    ensure!(
        (1..=30_000).contains(&options.blocks),
        "demo must run for 1 to 30,000 blocks (at most five minutes)"
    );
    let source = SourceA::prepare(options.microphone)?;
    let pair = LocalPair::start().await?;
    let result = run_on_pair(&pair, source, options).await;
    let cleanup = pair.shutdown().await;
    let report = result?;
    cleanup?;
    Ok(report)
}

async fn run_on_pair(pair: &LocalPair, source: SourceA, options: RunOptions) -> Result<DemoReport> {
    let mut a = VolumePeer::new(
        &pair.a.peer_id().to_string(),
        source.format(),
        !options.microphone,
    )?;
    let mut b = VolumePeer::new(
        &pair.b.peer_id().to_string(),
        AudioFormat::new(48_000, 1)?,
        true,
    )?;
    let endpoint_a = ComponentProtocolEndpoint::mount(pair.a.protocols(), a.runtime.clone())?;
    let endpoint_b = match ComponentProtocolEndpoint::mount(pair.b.protocols(), b.runtime.clone()) {
        Ok(endpoint) => endpoint,
        Err(error) => {
            endpoint_a.close().await?;
            return Err(error.into());
        }
    };
    let exchange = async {
        for (endpoint, graph) in [(&endpoint_a, &a), (&endpoint_b, &b)] {
            endpoint.export_product(&graph.audio_buffer.product())?;
            endpoint.export_product(&graph.level_buffer.product())?;
        }
        // Discover Products through the network Catalog, then explicitly connect.
        let mut a_observes_b = subscribe_levels(&pair.a, &pair.b).await?;
        let mut b_observes_a = subscribe_levels(&pair.b, &pair.a).await?;
        let monitor_a = LevelMonitor::new(&a.runtime, &a_observes_b)?;
        let monitor_b = LevelMonitor::new(&b.runtime, &b_observes_a)?;
        let (_, a_report, b_report) = tokio::try_join!(
            produce(&mut a, &mut b, source, options),
            observe(
                &mut a_observes_b,
                &monitor_a,
                "A observes B",
                options.print_levels
            ),
            observe(
                &mut b_observes_a,
                &monitor_b,
                "B observes A",
                options.print_levels
            ),
        )?;
        Ok::<_, anyhow::Error>(DemoReport {
            a: local_report(&a),
            b: local_report(&b),
            a_observed_b: a_report,
            b_observed_a: b_report,
        })
    };
    let result: Result<DemoReport> = tokio::select! {
        result = tokio::time::timeout(Duration::from_secs(options.blocks / 100 + 10), exchange) =>
            result.map_err(|_| anyhow::anyhow!("demo deadline exceeded; subscriptions are not automatically reconnected")).and_then(|result| result),
        signal = tokio::signal::ctrl_c() => match signal {
            Ok(()) => Err(anyhow::anyhow!("demo cancelled by user")),
            Err(error) => Err(error.into()),
        },
    };
    if result.is_err() {
        // The capture future has been dropped, so no more hardware blocks arrive.
        // Best-effort finalization is distinct from reporting a successful run.
        let _ = tokio::join!(a.finish(), b.finish());
    }
    // Attempt both closures even on an exchange/capture failure.
    let (cleanup_a, cleanup_b) = tokio::join!(endpoint_a.close(), endpoint_b.close());
    let report = result?;
    cleanup_a?;
    cleanup_b?;
    Ok(report)
}

fn local_report(graph: &VolumePeer) -> LocalReport {
    LocalReport {
        peer_id: graph.runtime.peer_id().into(),
        audio_entries: graph.audio_buffer.product().buffer().range().entries,
        volume_observations: graph.level_episode.product().observations().len(),
        concluded: matches!(
            graph.level_episode.product().state(),
            auki_components::EpisodeState::Concluded { .. }
        ),
    }
}

async fn produce(
    a: &mut VolumePeer,
    b: &mut VolumePeer,
    source: SourceA,
    options: RunOptions,
) -> Result<()> {
    {
        // Keep capture scoped so hardware stops before the Episode concludes.
        let mut source = source;
        source.start()?; // Activate only after both subscriptions exist.
        let mut pace = tokio::time::interval(Duration::from_millis(10));
        pace.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        for _ in 0..options.blocks {
            if options.paced && !options.microphone {
                pace.tick().await;
            }
            a.publish_audio(source.next().await?)?;
            b.publish_audio(synthetic_block(AudioFormat::new(48_000, 1)?, 0.25))?;
            // Local computation drains before accepting the next source block; slow
            // network subscribers do not participate in this barrier.
            tokio::try_join!(a.drain_meter(), b.drain_meter())?;
        }
    }
    tokio::try_join!(a.finish(), b.finish())?;
    Ok(())
}

async fn subscribe_levels(
    consumer: &AukiPeer,
    producer: &AukiPeer,
) -> Result<RemoteProductSubscription<f64>> {
    let client = ComponentProtocolClient::new(consumer.protocols());
    let route = producer.listen_addresses()[0].clone();
    let CatalogResponse::Snapshot { snapshot } = client
        .catalog_exact(producer.peer_id(), route.clone(), None)
        .await?
    else {
        bail!("initial Catalog request must return a snapshot");
    };
    let entry = snapshot
        .products
        .iter()
        .find(|entry| entry.manifest.product_id == LEVEL_PRODUCT)
        .ok_or_else(|| anyhow::anyhow!("remote volume Buffer not advertised"))?;
    client
        .subscribe_product_exact(
            producer.peer_id(),
            route,
            auki_components::ProductReference {
                peer_id: entry.manifest.peer_id.clone(),
                product_id: entry.manifest.product_id.clone(),
                manifest_hash: entry.manifest_hash.clone(),
            },
            ObservationStart::FromSequence { sequence: 0 },
            BufferLimits::entries(LEVEL_BUFFER_BLOCKS),
            |_| size_of::<f64>(),
        )
        .await
        .map_err(Into::into)
}

#[derive(Default)]
struct DisplayState {
    count: u64,
    latest: Option<f64>,
}

struct LevelMonitor {
    binding: ConfiguredBufferInput<f64>,
    _component: Component,
    state: Arc<Mutex<DisplayState>>,
    changed: Arc<Notify>,
}

impl LevelMonitor {
    fn new(
        runtime: &ComponentRuntime,
        subscription: &RemoteProductSubscription<f64>,
    ) -> Result<Self> {
        let PayloadContract::Gauge(gauge) = &subscription.product().producer.payload else {
            bail!("expected a Gauge");
        };
        ensure!(
            gauge.schema == LEVEL_SCHEMA
                && gauge.observes == "audio_rms_level"
                && gauge.unit == "dBFS",
            "remote level semantics are incompatible"
        );
        let component = runtime.component(
            ComponentSpec::new("remote-volume-display").product_input(ProductInputContract {
                name: "level".into(),
                form: ProductForm::Buffer,
                datatype: "float64".into(),
                schema: LEVEL_SCHEMA.into(),
                // Eligible for a manifest, but never exported by this endpoint.
                exposure: Exposure::Cluster,
            }),
        )?;
        let state = Arc::new(Mutex::new(DisplayState::default()));
        let changed = Arc::new(Notify::new());
        let target = state.clone();
        let wake = changed.clone();
        let input = InputPort::try_new(
            "remote-volume-display.input.level",
            move |envelope: &auki_components::Envelope<Observation<f64>>| {
                let value = *envelope.payload.payload;
                if !value.is_finite() {
                    return Err("non-finite volume".into());
                }
                let mut state = target.lock().unwrap();
                state.count += 1;
                state.latest = Some(value);
                drop(state);
                wake.notify_one();
                Ok(())
            },
        );
        let binding = component.configured_buffer_input(
            "level",
            subscription.product(),
            CursorStart::FromSequence(subscription.next_sequence()),
            &input,
        )?;
        component.expose()?;
        Ok(Self {
            binding,
            _component: component,
            state,
            changed,
        })
    }

    async fn drain(&self, expected: u64) -> Result<Option<f64>> {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let notified = self.changed.notified();
                {
                    let state = self.state.lock().unwrap();
                    if state.count == expected {
                        return Ok(state.latest);
                    }
                }
                ensure!(!self.binding.stats().failed, "remote display failed");
                notified.await;
            }
        })
        .await
        .map_err(|_| anyhow::anyhow!("remote display did not drain: {:?}", self.binding.stats()))?
    }
}

async fn observe(
    subscription: &mut RemoteProductSubscription<f64>,
    monitor: &LevelMonitor,
    label: &str,
    print: bool,
) -> Result<RemoteReport> {
    let mut report = RemoteReport {
        source_peer_id: subscription.product().manifest.peer_id.clone(),
        observations: 0,
        gap_entries: 0,
        last_sequence: None,
        latest_dbfs: None,
    };
    while let Some(event) = subscription.next().await? {
        match event {
            RemoteObservationEvent::Observation(observation) => {
                report.observations += 1;
                report.last_sequence = Some(observation.sequence);
                if print && report.observations.is_multiple_of(20) {
                    eprintln!("{label}: {:.1} dBFS", observation.payload);
                }
            }
            RemoteObservationEvent::Gap(gap) => {
                report.gap_entries += gap.available_from - gap.requested_sequence;
                if print {
                    eprintln!(
                        "{label}: retention gap {}..{}",
                        gap.requested_sequence, gap.available_from
                    );
                }
            }
            RemoteObservationEvent::Closed(Some(end)) => {
                bail!("{label}: producer ended: {:?}", end.reason)
            }
            RemoteObservationEvent::Closed(None) => {}
        }
    }
    report.latest_dbfs = monitor.drain(report.observations).await?;
    subscription.close().await?;
    Ok(report)
}
