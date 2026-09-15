"""Optional, explicitly authorized Info exchange; no hardware or secret data."""


def info_provider(peer_id, run_id, expected_peer_id):
    if not expected_peer_id:
        raise ValueError("P2P requires an expected partner Peer ID")

    def info(requester):
        # The binding passes the authenticated principal, not a bare Peer ID.
        if not isinstance(requester, dict) or requester.get("peer_id") != expected_peer_id:
            return None
        return {"app": "compute-robot", "app_version": "1.0.0", "name": run_id,
                "session_id": run_id, "session_clock_id": run_id,
                "session_clock_hash": "00" * 32, "session_now_ns": 0,
                "peer_id": peer_id, "app_instance": run_id}
    return info


async def fetch_robot(task, run_id, expected_peer_id):
    import auki_sdk
    remote = task.meta.get("remote_peer_id")
    if not remote or remote != expected_peer_id or not task.meta.get("remote_route"):
        raise ValueError("P2P target is not the configured partner")
    peer = task.peer()
    if peer is None:
        raise ValueError("P2P task requires a peer")
    info = await auki_sdk.AukiInfoClient(peer).fetch_exact(remote, task.meta["remote_route"])
    if info["name"] != run_id or info["peer_id"] != expected_peer_id:
        raise ValueError("unexpected robot response")
    await task.log_event({"phase": "peer-verified", "run_id": run_id})
