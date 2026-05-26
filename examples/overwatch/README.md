# Overwatch

Web-only Auki SDK example.

Overwatch proves that a browser can act as an Auki Domain peer through generated SDK JavaScript/WASM bindings. It uses Discovery for rendezvous and generic WebRTC signaling. It does not run an app backend.

Run from the repository root:

```bash
just overwatch
```

`just overwatch` stages the generated `@aukilabs/auki-network` and `@aukilabs/auki-domain` JavaScript/WASM packages into the example, installs the React app dependencies, and starts the Vite dev server on port 7880.

The app imports the generated SDK bindings directly through local `file:` package dependencies. It does not use a fake peer, app backend, or app-specific signaling service.

The webcam stream is browser-owned: Overwatch captures frames with `getUserMedia`, encodes them to JPEG in a canvas, and publishes the resulting async byte stream through the generated SDK `publishSensor` binding.

Run the acceptance smoke with:

```bash
just overwatch-smoke
```

The smoke starts Discovery from `/Users/jb/Developer/Aukilabs/repos/discovery`, starts Vite, opens two isolated Chromium browser contexts, joins them into one browser Domain, subscribes to a generated sensor stream, and checks that no request goes to an app `/api/` route.
