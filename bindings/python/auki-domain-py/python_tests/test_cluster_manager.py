"""ClusterManager binding surface tests."""


def test_cluster_manager_exposes_generic_open_stream():
    import auki_domain

    assert hasattr(auki_domain.ClusterManager, "open_stream")
    assert "open_stream" in dir(auki_domain.ClusterManager)


def test_cluster_manager_exposes_relay_constructors():
    import auki_domain

    mgr = auki_domain.ClusterManager
    assert hasattr(mgr, "bootstrap")
    assert hasattr(mgr, "create_cluster")
    assert hasattr(mgr, "create_cluster_with_relay_multiaddrs")
    assert hasattr(mgr, "create_cluster_with_relay_reservation")
    assert hasattr(mgr, "join_cluster")
    assert hasattr(mgr, "list_clusters")


def test_cluster_manager_exposes_instance_methods():
    import auki_domain

    mgr = auki_domain.ClusterManager
    for method in (
        "membership",
        "participant_info",
        "fetch_participant_info",
        "admit_peer",
        "fetch_resources_catalog",
        "fetch_sensor_entry",
        "fetch_clock_entry",
        "fetch_frame_entry",
        "set_resource_catalog_provider",
        "set_registry_app_root",
        "open_camera_stream",
        "open_pointcloud_stream",
        "open_joint_encoders_stream",
        "open_audio_stream",
        "open_pose_stream",
        "open_stream",
        "open_stream_with_request",
        "shutdown",
    ):
        assert hasattr(mgr, method), f"ClusterManager missing method {method!r}"
