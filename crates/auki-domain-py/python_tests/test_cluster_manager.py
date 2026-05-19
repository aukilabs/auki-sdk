"""ClusterManager binding surface tests."""


def test_cluster_manager_exposes_generic_open_stream():
    import auki_domain

    assert hasattr(auki_domain.ClusterManager, "open_stream")
    assert "open_stream" in dir(auki_domain.ClusterManager)
