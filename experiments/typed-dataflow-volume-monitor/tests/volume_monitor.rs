use std::sync::{Arc, Mutex};

use auki_typed_dataflow_experiment::{
    AudioLayout, AudioSampleFormat, ObservationEvent, ProductForm, ProductState,
    SerializedInMemoryTransport, observation_input,
};
use auki_typed_dataflow_volume_monitor::{
    AUDIO_BUFFER_BLOCKS, FRAMES_PER_BLOCK, GaugeObservation, SILENCE_FLOOR_DBFS, VolumePeer,
    rms_dbfs,
};

#[test]
fn public_api_builds_truthful_two_peer_volume_graph() {
    let peer_a = VolumePeer::new("peer-a").unwrap();
    let peer_b = VolumePeer::new("peer-b").unwrap();

    for peer in [&peer_a, &peer_b] {
        let components = peer.runtime.catalog().components();
        let products = peer.runtime.catalog().products();
        assert_eq!(components.len(), 2);
        assert_eq!(products.len(), 2);
        assert_eq!(
            products
                .iter()
                .filter(|entry| entry.manifest.form == ProductForm::Buffer)
                .count(),
            1
        );
        assert_eq!(
            products
                .iter()
                .filter(|entry| entry.manifest.form == ProductForm::Episode)
                .count(),
            1
        );

        let microphone = peer.runtime.catalog().component("microphone").unwrap();
        let audio = &microphone.current_outputs["audio"].manifest.payload;
        let auki_typed_dataflow_experiment::PayloadContract::Audio(audio) = audio else {
            panic!("microphone must expose a typed audio contract");
        };
        assert_eq!(audio.sample_format, AudioSampleFormat::F32);
        assert_eq!(audio.layout, AudioLayout::Interleaved);
        assert_eq!(audio.frames_per_block, FRAMES_PER_BLOCK);

        let volume = peer.runtime.catalog().component("volume-meter").unwrap();
        let level = &volume.current_outputs["level"].manifest.payload;
        assert_eq!(level.kind(), "gauge");
        assert_eq!(level.observes(), "audio_level");
        assert_eq!(level.unit(), Some("dBFS"));
    }
}

#[test]
fn buffer_meter_and_episode_share_payloads_and_enforce_lifecycle() {
    let peer = VolumePeer::new("peer-a").unwrap();
    let first = peer
        .publish_audio(0, Arc::from(vec![0.5; FRAMES_PER_BLOCK as usize]))
        .unwrap();
    assert!(Arc::ptr_eq(&first, &peer.last_meter_input().unwrap()));
    let retained = peer.audio_buffer.product();
    let retained_first = retained.buffer.snapshot(0, 0).pop().unwrap();
    assert!(Arc::ptr_eq(&first, &retained_first.payload.payload));
    assert_eq!(peer.level_episode.product().observations().len(), 1);

    for sequence in 1..=AUDIO_BUFFER_BLOCKS as u64 {
        peer.publish_audio(
            sequence * 10_000_000,
            Arc::from(vec![0.25; FRAMES_PER_BLOCK as usize]),
        )
        .unwrap();
    }
    let retained = peer.audio_buffer.product();
    assert_eq!(retained.buffer.range().entries, AUDIO_BUFFER_BLOCKS);
    assert_eq!(retained.buffer.range().first_sequence, Some(1));
    assert_eq!(
        peer.level_episode.product().observations().len(),
        AUDIO_BUFFER_BLOCKS + 1
    );

    peer.conclude_session(60_010_000_000).unwrap();
    assert!(peer.conclude_session(60_020_000_000).is_err());
    let entry = peer
        .runtime
        .catalog()
        .product("volume-meter.level.session-episode")
        .unwrap();
    assert_eq!(
        entry.state,
        ProductState::Episode {
            observations: AUDIO_BUFFER_BLOCKS + 1,
            concluded_at_ns: Some(60_010_000_000),
        }
    );
}

#[test]
fn serialized_remote_observation_preserves_value_and_drop_stops_delivery() {
    let peer_a = VolumePeer::new("peer-a").unwrap();
    let received = Arc::new(Mutex::new(Vec::<Arc<GaugeObservation>>::new()));
    let received_input = Arc::clone(&received);
    let input = observation_input(
        "peer-b.input.peer-a-volume",
        move |event: &ObservationEvent<GaugeObservation>| {
            if let ObservationEvent::Observation(observation) = event {
                received_input
                    .lock()
                    .unwrap()
                    .push(Arc::clone(&observation.payload));
            }
        },
    );
    let transport = SerializedInMemoryTransport::default();
    let handle = peer_a.observe_volume_through(&transport, &input).unwrap();

    peer_a
        .publish_audio(0, Arc::from(vec![0.5; FRAMES_PER_BLOCK as usize]))
        .unwrap();
    assert_eq!(received.lock().unwrap().len(), 1);
    let local = peer_a.level_episode.product().observations()[0]
        .payload
        .clone();
    let remote = received.lock().unwrap()[0].clone();
    assert_eq!(*local, *remote);
    assert!(!Arc::ptr_eq(&local, &remote));
    assert!(transport.stats().encoded_bytes > 0);

    drop(handle);
    peer_a
        .publish_audio(10_000_000, Arc::from(vec![0.25; FRAMES_PER_BLOCK as usize]))
        .unwrap();
    assert_eq!(received.lock().unwrap().len(), 1);
}

#[test]
fn dbfs_has_a_finite_silence_floor() {
    assert_eq!(
        rms_dbfs(&vec![0.0; FRAMES_PER_BLOCK as usize]),
        SILENCE_FLOOR_DBFS
    );
    assert!((rms_dbfs(&vec![0.5; FRAMES_PER_BLOCK as usize]) + 6.020_599_913).abs() < 1e-9);
}
