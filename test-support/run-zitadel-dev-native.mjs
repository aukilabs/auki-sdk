// Native-only, shared-dev smoke test. Never run the synthetic fixture controller on dev.
// Node parses the private dotenv; only the Rust AuthSession may refresh the grant.
import { spawn, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { constants, closeSync, existsSync, lstatSync, mkdirSync, openSync, readFileSync, unlinkSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseEnv } from 'node:util';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const state = join(root, '.zitadel-dev-state');
const credentialsPath = join(state, 'native-session.json');
const lockPath = join(state, 'native.lock');
const args = process.argv.slice(2);
const exerciseRefresh = args.includes('--exercise-refresh');
if (args.some(arg => arg !== '--exercise-refresh')) throw new Error('Only --exercise-refresh is supported');

function privatePath(path, directory = false) {
  const stat = lstatSync(path);
  if (stat.isSymbolicLink() || (directory ? !stat.isDirectory() : !stat.isFile()) || (stat.mode & 0o077)) {
    throw new Error('Configuration/state must be private, regular files/directories, not symlinks');
  }
}

function childResult(child) {
  return new Promise(resolve => {
    child.once('error', () => resolve(1));
    child.once('close', code => resolve(code ?? 1));
  });
}

async function run() {
  const envPath = join(root, '.env.zitadel-dev');
  for (const path of ['.env.zitadel-dev', '.zitadel-dev-state/native-session.json', '.zitadel-dev-state/native.lock']) {
    if (spawnSync('git', ['check-ignore', '-q', path], { cwd: root, stdio: 'ignore' }).status !== 0) {
      throw new Error('Ignore the private dotenv and the entire .zitadel-dev-state directory before running');
    }
  }
  privatePath(envPath);
  const env = parseEnv(readFileSync(envPath, 'utf8'));
  for (const [key, expected] of Object.entries({
    AUKI_API_BASE_URL: 'https://api.dev.aukiverse.com/',
    AUKI_DDS_BASE_URL: 'https://dds.dev.aukiverse.com/',
    AUKI_DMS_BASE_URL: 'https://dms.dev.aukiverse.com/v1/',
    ZITADEL_ISSUER: 'https://auth.dev.aukiverse.com',
  })) {
    if (env[key] !== expected) throw new Error(`Unexpected dev endpoint: ${key}`);
  }
  for (const key of ['ZITADEL_ACCESS_TOKEN', 'ZITADEL_REFRESH_TOKEN', 'ZITADEL_CLIENT_ID']) {
    if (!env[key] || env[key].trim() !== env[key]) throw new Error(`Missing or invalid ${key}`);
  }
  if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(env.AUKI_DOMAIN_ID ?? '')) {
    throw new Error('AUKI_DOMAIN_ID must be a UUID');
  }
  const seedHash = createHash('sha256').update(env.ZITADEL_REFRESH_TOKEN).digest('hex');
  let credentials = {
    accessToken: env.ZITADEL_ACCESS_TOKEN,
    refreshToken: env.ZITADEL_REFRESH_TOKEN,
    clientId: env.ZITADEL_CLIENT_ID,
    issuer: env.ZITADEL_ISSUER,
    accessTokenExpiresAt: env.ZITADEL_ACCESS_TOKEN_EXPIRES_AT || null,
  };
  if (!existsSync(state)) mkdirSync(state, { mode: 0o700 });
  privatePath(state, true);
  if (existsSync(lockPath)) throw new Error('Prior native run is active or unclean; inspect it before reusing credentials');
  // Build before handing off the grant. No dotenv values enter the child environment or argv.
  const childEnv = Object.fromEntries(Object.entries(process.env).filter(([key]) => !/^(ZITADEL_|AUKI_|RUST_LOG$|RUST_BACKTRACE$)/.test(key)));
  childEnv.CARGO_TARGET_DIR = join(root, 'target');
  const build = spawn('cargo', ['build', '--locked', '-p', 'auki-standard-protocols-native', '--bin', 'zitadel_dev_native'], { cwd: root, env: childEnv, stdio: 'inherit' });
  if (await childResult(build)) throw new Error('Native runner build failed');
  // A crash/kill leaves this lock in place: an unknown refresh outcome must never replay a seed.
  closeSync(openSync(lockPath, constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL, 0o600));
  // Read the current generation only AFTER taking the lock. A previous run may
  // have rotated while this process was compiling; pre-lock snapshots are unsafe.
  if (existsSync(credentialsPath)) {
    privatePath(credentialsPath);
    let saved;
    try { saved = JSON.parse(readFileSync(credentialsPath, 'utf8')); }
    catch { throw new Error('Saved native session is invalid; do not replay the dotenv refresh token'); }
    if (saved.seedHash !== seedHash || saved.credentials.clientId !== credentials.clientId || new URL(saved.credentials.issuer).href !== new URL(credentials.issuer).href) {
      throw new Error('Dotenv grant/config changed; retain and review prior native state before starting a different grant');
    }
    credentials = saved.credentials;
    console.log('Using durably saved native credentials; not replaying the dotenv refresh token.');
  }
  const child = spawn(join(root, 'target/debug/zitadel_dev_native'), [], { cwd: root, env: childEnv, stdio: ['pipe', 'pipe', 'inherit'] });
  let cleanClose = false;
  let pending = '';
  child.stdout.setEncoding('utf8');
  child.stdout.on('data', chunk => {
    pending += chunk;
    const lines = pending.split('\n');
    pending = lines.pop();
    for (const line of lines) {
      // The native runner emits only non-secret structured events, with no HTTP tracing.
      console.log(line);
      try { const event = JSON.parse(line); if (event.event === 'session-closed' && event.canResume === true) cleanClose = true; } catch { /* not a control event */ }
    }
  });
  for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => child.kill(signal));
  child.stdin.on('error', () => { /* an early child exit leaves the safety lock */ });
  child.stdin.end(JSON.stringify({ domainId: env.AUKI_DOMAIN_ID, credentials, credentialsPath, seedHash, exerciseRefresh }));
  const code = await childResult(child);
  if (cleanClose) unlinkSync(lockPath);
  else console.error('Native safety lock retained. Do not reuse this grant until its outcome is reviewed.');
  process.exitCode = code;
}

run().catch(error => {
  // Never print parser errors: they can include input credential bytes.
  console.error(error instanceof SyntaxError ? 'Invalid private configuration (details suppressed)' : error.message);
  process.exitCode = 1;
});
