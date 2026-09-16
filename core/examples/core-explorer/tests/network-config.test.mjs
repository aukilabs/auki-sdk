import assert from 'node:assert/strict';
import { test } from 'node:test';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { networkConfig } from './network-config.mjs';

const root = fileURLToPath(new URL('../../../../', import.meta.url));
test('network harness defaults need no environment overrides', () => {
  assert.deepEqual(networkConfig({}), {
    relay: resolve(root, 'target/debug/core-explorer-test-relay'),
    python: 'python3',
  });
});
test('network harness uses an explicit target cache and Python interpreter', () => {
  assert.deepEqual(networkConfig({ CARGO_TARGET_DIR: '/tmp/example-cache', ROBOT_PYTHON: '/tmp/example-venv/bin/python' }), {
    relay: '/tmp/example-cache/debug/core-explorer-test-relay',
    python: '/tmp/example-venv/bin/python',
  });
  assert.equal(networkConfig({ CARGO_TARGET_DIR: 'relative-cache' }).relay,
    resolve(root, 'relative-cache/debug/core-explorer-test-relay'));
});
