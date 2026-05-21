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
- Simulated UTC offset: `0ms`

## Timing Modes

`UTC` mode flashes and beeps every three seconds on host UTC wall-clock boundaries and applies no Auki correction. Use this first to eyeball whether two machines have visibly different UTC time. Use the `Sound` checkbox in the flash panel to silence or re-enable the beep.

`Simulated UTC offset` adds a diagnostic-only millisecond offset to UTC flash timing without changing the OS clock. Domain timing ignores this offset. Set one client to `+250ms` to make UTC mode visibly worse while keeping the machine clock untouched.

`Domain` mode flashes against the cluster domain clock reported by `ClusterManager::domain_clock_estimate()` and `ClusterManager::domain_time_now()`. It stays unavailable until heartbeat sync can produce an explicit domain-time reading; the app does not fall back to wall time.

## Sync Quality

When clustered, every flash rising edge publishes a best-effort diagnostic tick report to peers. Matching local and peer reports are shown in the `Sync Quality` table:

- `UTC latest / p95`: skew between clients using simulated UTC timing.
- `Domain latest / p95`: skew between clients using domain timing.
- `Improvement`: latest UTC skew divided by latest Domain skew when both are available.

## Two-Laptop Test

1. Start Discovery.
2. Run this app on the macOS laptop.
3. Run this app on the Linux laptop.
4. Set the same Discovery URL and cluster name on both.
5. Click `Join / Create` on both.
6. Confirm both apps show two peers and peer-id suffixes.
7. Put the laptops side by side and compare UTC flashes.
8. Set one laptop's `Simulated UTC offset` to `+250ms` and confirm UTC flashes diverge.
9. When the `Session -> domain` status shows `synced`, switch both apps to Domain mode and compare the corrected flashes.
10. Check `Sync Quality`: UTC skew should be larger than Domain skew, producing an improvement ratio above `1.0x`.
