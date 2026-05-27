# Overwatch

Web-only Auki SDK example using Park's operator UI.

Overwatch proves that a browser can act as an Auki Domain peer through generated SDK JavaScript/WASM bindings while rendering the same frontend shell as `../park/src/ui`. Park's backend-facing data modules are replaced with a browser-local SDK runtime. Discovery is still used for rendezvous, but Overwatch does not run an app backend or call app `/api/*` routes.

Run from the repository root:

```bash
just overwatch
```

`just overwatch` stages the generated `@aukilabs/auki-network`, `@aukilabs/auki-domain`, `@aukilabs/auki-geometry`, and `@aukilabs/auki-proto` JavaScript/WASM packages into the example, installs the Park UI dependencies, and starts the Vite dev server on port 7880.

The app imports the generated SDK bindings directly through local `file:` package dependencies. It does not use a fake peer, app backend, or app-specific signaling service. The Park-shaped UI state comes from `src/sdk/runtime.ts`, which adapts generated SDK participant, catalog, registry, and stream state into the data contracts expected by the copied Park views.

The current browser peer publishes deterministic demo sensors through the generated SDK `publishSensor` binding so other browser peers can discover a catalog entry and subscribe to a byte stream without a Park server.

Run the acceptance smoke with:

```bash
just overwatch-smoke
```

The smoke starts Discovery from `/Users/jb/Developer/Aukilabs/repos/discovery`, starts Vite, opens two isolated Chromium browser contexts, joins them into one browser Domain, subscribes to a generated sensor stream, and checks that no request goes to an app `/api/` route.
