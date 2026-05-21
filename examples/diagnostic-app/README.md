# Auki Diagnostic App

Native macOS/Linux diagnostic example for Auki networking, clustering, and time-sync visibility.

## Run

```bash
cargo run -p auki-diagnostic-app
```

The app defaults to:

- Discovery URL: `http://127.0.0.1:8080`
- Cluster name: `hagall-test`
- Flash mode: `UTC`
- Sound: enabled when the host audio backend is available

## Timing Modes

`UTC` mode flashes and beeps every three seconds on host UTC wall-clock boundaries and applies no Auki correction. Use this first to eyeball whether two machines have visibly different UTC time. Use the `Sound` checkbox in the flash panel to silence or re-enable the beep.

`Domain` mode flashes against the cluster domain clock reported by `ClusterManager::domain_clock_estimate()` and `ClusterManager::domain_time_now()`. It stays unavailable until heartbeat sync can produce an explicit domain-time reading; the app does not fall back to wall time.

## Two-Laptop Test

1. Start Discovery.
2. Run this app on the macOS laptop.
3. Run this app on the Linux laptop.
4. Set the same Discovery URL and cluster name on both.
5. Click `Join / Create` on both.
6. Confirm both apps show two peers and peer-id suffixes.
7. Put the laptops side by side and compare UTC flashes.
8. When the `Session -> domain` status shows `synced`, switch both apps to Domain mode and compare the corrected flashes.
