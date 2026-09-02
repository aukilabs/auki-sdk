# Auki Camera Mesh

This example turns the portable Auki peer and protocol facades into one small,
inspectable camera application. The Web runtime lands first; deterministic Rust
and Python publishers and the Swift/iOS viewer follow the same protocol shapes.

## Run the Web demo

```bash
cd examples/camera-mesh/web
npm ci
npm run dev
```

Open the loopback URL in two browser tabs. Sign both into the same Domain, run
one as a Publisher and one as a Viewer, then follow the controls in the page.
The synthetic source works without camera permission and is the deterministic
browser-to-browser test source.

See [web/README.md](web/README.md) for the current scope and expected flow.
