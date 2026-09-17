// Permit the sibling generated WASM package in this loopback-only test server.
export default { server: { fs: { allow: [new URL('../../../../', import.meta.url).pathname] } } };
