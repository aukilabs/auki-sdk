import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../../../../', import.meta.url));
export function networkConfig(env = process.env) {
  return {
    relay: resolve(root, env.CARGO_TARGET_DIR || 'target', 'debug/core-explorer-test-relay'),
    python: env.ROBOT_PYTHON || 'python3',
  };
}
