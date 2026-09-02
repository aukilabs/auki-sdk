# Auki Camera Mesh

This example turns the portable Auki peer and all six standard protocol facades
into one inspectable camera application. The Web runtime is the first complete
publisher and viewer; deterministic Rust and Python peers and the Swift/iOS
viewer follow the same protocol shapes in later phases.

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
