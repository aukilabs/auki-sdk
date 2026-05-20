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

## Timing Modes

`UTC` mode flashes every three seconds on host UTC wall-clock boundaries and applies no Auki correction. Use this first to eyeball whether two machines have visibly different UTC time.

`Domain` mode is reserved for heartbeat domain-clock sync. In this SDK build, the domain sync snapshot API is not implemented yet, so the app shows Domain mode as unavailable rather than faking corrected timing.

## Two-Laptop Test

1. Start Discovery.
2. Run this app on the macOS laptop.
3. Run this app on the Linux laptop.
4. Set the same Discovery URL and cluster name on both.
5. Click `Join / Create` on both.
6. Confirm both apps show two peers and peer-id suffixes.
7. Put the laptops side by side and compare UTC flashes.
8. After heartbeat domain-clock sync lands, switch both apps to Domain mode and compare the corrected flashes.
