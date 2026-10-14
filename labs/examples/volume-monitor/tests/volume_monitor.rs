use std::sync::Arc;
use std::time::Duration;

use auki_component_protocol::{
    ComponentProtocolClient, ComponentProtocolEndpoint, ObservationStart, RemoteObservationEvent,
};
use auki_component_volume_monitor::{
    AUDIO_BUFFER_BLOCKS, AUDIO_PRODUCT, AudioBlock, AudioFormat, BLOCK_NS, EPISODE_PRODUCT,
    LEVEL_PRODUCT, SILENCE_FLOOR_DBFS, VolumePeer,
    demo::{RunOptions, run},
    local::LocalPair,
    rms_dbfs, synthetic_block,
};
use auki_components::{
    AudioLayout, AudioSampleFormat, BufferLimits, PayloadContract, ProductForm, ProductState,
};

#[tokio::test]
async fn manifests_describe_emitted_audio_and_a_real_float_gauge() {
    let format = AudioFormat::new(44_100, 2).unwrap();
    let mut graph = VolumePeer::new("fixture-peer", format, true).unwrap();
    let source = graph
        .runtime
        .catalog()
        .component("synthetic-audio")
        .unwrap();
    assert_eq!(source.manifest_hash, source.manifest.hash());
    let PayloadContract::Audio(audio) = &source.current_outputs["audio"].manifest.payload else {
        panic!("not audio");
    };
    assert_eq!(
        (audio.sample_rate_hz, audio.channels, audio.frames_per_block),
        (44_100, 2, 441)
    );
    assert_eq!(audio.sample_format, AudioSampleFormat::F32);
    assert_eq!(audio.layout, AudioLayout::Interleaved);
    assert_eq!(audio.observes, "synthetic_waveform");
    let meter = graph.runtime.catalog().component("volume-meter").unwrap();
    assert_eq!(
        meter.current_product_inputs["audio"]
            .manifest
            .product
            .product_id,
        AUDIO_PRODUCT
    );
    let PayloadContract::Gauge(gauge) = &meter.current_outputs["level"].manifest.payload else {
        panic!("not a Gauge");
    };
    assert_eq!(gauge.datatype, "float64");
    assert_eq!(gauge.unit, "dBFS");
    assert_eq!(graph.runtime.catalog().products().len(), 3);
    for product in graph.runtime.catalog().products() {
        assert_eq!(product.manifest_hash, product.manifest.hash());
    }
    graph.publish_audio(synthetic_block(format, 0.5)).unwrap();
    graph.drain_meter().await.unwrap();
    let level = graph.level_episode.product().observations()[0]
        .payload
        .clone();
    // The claimed float64 serializes as a scalar, not { "value": ... }.
    assert!(serde_json::to_value(*level).unwrap().is_number());
    graph.finish().await.unwrap();
}

#[tokio::test]
async fn audio_retention_evicts_but_the_session_episode_keeps_all_derived_values() {
    let format = AudioFormat::new(8_000, 1).unwrap();
    let mut graph = VolumePeer::new("fixture-peer", format, true).unwrap();
    let first = graph.publish_audio(synthetic_block(format, 0.5)).unwrap();
    graph.drain_meter().await.unwrap();
    assert!(Arc::ptr_eq(&first, &graph.last_meter_input().unwrap()));
    assert!(Arc::ptr_eq(
        &first,
        &graph
            .audio_buffer
            .product()
            .latest_existing()
            .unwrap()
            .unwrap()
            .payload
    ));
    let level = graph.level_episode.product().observations()[0]
        .payload
        .clone();
    assert!(Arc::ptr_eq(
        &level,
        &graph
            .level_buffer
            .product()
            .latest_existing()
            .unwrap()
            .unwrap()
            .payload
    ));
    for _ in 0..AUDIO_BUFFER_BLOCKS {
        graph.publish_audio(synthetic_block(format, 0.25)).unwrap();
        graph.drain_meter().await.unwrap();
    }
    let range = graph.audio_buffer.product().buffer().range();
    assert_eq!(range.entries, AUDIO_BUFFER_BLOCKS);
    assert_eq!(range.first_sequence, Some(1));
    assert_eq!(
        graph.level_episode.product().observations().len(),
        AUDIO_BUFFER_BLOCKS + 1
    );
    graph.finish().await.unwrap();
    let entry = graph.runtime.catalog().product(EPISODE_PRODUCT).unwrap();
    assert_eq!(entry.manifest.form, ProductForm::Episode);
    assert_eq!(
        entry.state,
        ProductState::Episode {
            observations: AUDIO_BUFFER_BLOCKS + 1,
            concluded_at_ns: Some((AUDIO_BUFFER_BLOCKS as u64 + 1) * BLOCK_NS)
        }
    );
    assert!(graph.publish_audio(synthetic_block(format, 0.5)).is_err());
    assert!(graph.finish().await.is_err());
}

#[tokio::test]
async fn invalid_samples_and_changed_configuration_are_rejected_before_publication() {
    let format = AudioFormat::new(48_000, 1).unwrap();
    let mut graph = VolumePeer::new("fixture-peer", format, false).unwrap();
    let mut invalid = synthetic_block(format, 0.5);
    invalid.interleaved_samples.pop();
    assert!(graph.publish_audio(invalid).is_err());
    let mut invalid = synthetic_block(format, 0.5);
    invalid.interleaved_samples[0] = f32::NAN;
    assert!(graph.publish_audio(invalid).is_err());
    assert!(
        graph
            .publish_audio(synthetic_block(AudioFormat::new(44_100, 2).unwrap(), 0.5))
            .is_err()
    );
    assert_eq!(graph.audio_buffer.product().buffer().range().entries, 0);
    graph.publish_audio(synthetic_block(format, 0.5)).unwrap();
    graph.finish().await.unwrap();
    assert_eq!(graph.level_episode.product().observations()[0].sequence, 0);
}

#[test]
fn dbfs_is_finite_and_uncalibrated() {
    assert_eq!(rms_dbfs(&[0.0; 80]).unwrap(), SILENCE_FLOOR_DBFS);
    assert!((rms_dbfs(&[0.5; 80]).unwrap() + 6.020_599_913).abs() < 1e-8);
    assert!(rms_dbfs(&[]).is_err());
    assert!(rms_dbfs(&[f32::INFINITY]).is_err());
    assert!(rms_dbfs(&[2.0]).is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_real_peers_exchange_volume_and_conclude_both_episodes() {
    let report = run(RunOptions {
        blocks: 20,
        paced: false,
        microphone: false,
        print_levels: false,
    })
    .await
    .unwrap();
    assert_ne!(report.a.peer_id, report.b.peer_id);
    for local in [&report.a, &report.b] {
        assert_eq!(local.audio_entries, 20);
        assert_eq!(local.volume_observations, 20);
        assert!(local.concluded);
    }
    assert_eq!(report.a_observed_b.source_peer_id, report.b.peer_id);
    assert_eq!(report.b_observed_a.source_peer_id, report.a.peer_id);
    for (remote, expected) in [
        (&report.a_observed_b, -15.0515),
        (&report.b_observed_a, -9.0309),
    ] {
        assert_eq!(remote.observations, 20);
        assert_eq!(remote.last_sequence, Some(19));
        assert_eq!(remote.gap_entries, 0);
        assert!((remote.latest_dbfs.unwrap() - expected).abs() < 0.001);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_same_protocol_can_subscribe_to_raw_audio_without_lying_about_its_shape() {
    let pair = LocalPair::start().await.unwrap();
    let format = AudioFormat::new(48_000, 1).unwrap();
    let mut graph = VolumePeer::new(&pair.a.peer_id().to_string(), format, true).unwrap();
    let endpoint =
        ComponentProtocolEndpoint::mount(pair.a.protocols(), graph.runtime.clone()).unwrap();
    endpoint
        .export_product(&graph.audio_buffer.product())
        .unwrap();
    let client = ComponentProtocolClient::new(pair.b.protocols());
    let mut subscription = client
        .subscribe_product_exact::<AudioBlock>(
            pair.a.peer_id(),
            pair.a.listen_addresses()[0].clone(),
            graph.audio_buffer.product().reference(),
            ObservationStart::NewOnly,
            BufferLimits::entries(2),
            AudioBlock::retained_bytes,
        )
        .await
        .unwrap();
    let local = graph.publish_audio(synthetic_block(format, 0.5)).unwrap();
    let event = tokio::time::timeout(Duration::from_secs(3), subscription.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let RemoteObservationEvent::Observation(remote) = event else {
        panic!("expected audio");
    };
    remote.payload.validate(format).unwrap();
    assert_eq!(*remote.payload, *local);
    assert!(!Arc::ptr_eq(&remote.payload, &local));
    assert_eq!(
        remote.output,
        graph.audio_buffer.product().manifest.producer
    );
    assert_eq!(remote.timestamp_ns, 0);
    assert_ne!(subscription.product().manifest.product_id, LEVEL_PRODUCT);
    graph.finish().await.unwrap();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(3), subscription.next())
            .await
            .unwrap()
            .unwrap(),
        Some(RemoteObservationEvent::Closed(None))
    ));
    subscription.close().await.unwrap();
    endpoint.close().await.unwrap();
    pair.shutdown().await.unwrap();
}
