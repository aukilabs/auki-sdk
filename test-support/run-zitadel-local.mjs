// Local-only real-service acceptance. Run from any directory; no deployments,
// releases, remote test accounts, global clock/DNS or trust-store modifications.
import fs from 'node:fs';
import path from 'node:path';
import net from 'node:net';
import tls from 'node:tls';
import dns from 'node:dns/promises';
import { X509Certificate, createHash } from 'node:crypto';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { startIdentityFixture, identityBase } from './zitadel-local-fixture.mjs';

const args = process.argv.slice(2);
if (args.length === 1 && args[0] === '--help') {
  console.log('Usage: node test-support/run-zitadel-local.mjs [--soak]\nDefault: short E2E smoke (two minutes of relay traffic after readiness).\n--soak: optional normal-lifetime renewal/expiry check (~68 minutes after readiness).');
  process.exit(0);
}
if (args.length > 1 || (args.length === 1 && args[0] !== '--soak')) {
  console.error('Expected no arguments (short smoke), --soak, or --help.');
  process.exit(2);
}
const soak = args[0] === '--soak';
const mode = soak ? 'soak' : 'smoke';

const sdk = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const workspace = path.dirname(sdk);
const hagall = path.resolve(process.env.Z13_HAGALL_DIR ?? path.join(workspace, 'hagall'));
if (!fs.existsSync(path.join(hagall, 'pkg/verification/keyring.go'))) {
  console.error('A standalone-relay Hagall checkout is required at ../hagall, or set Z13_HAGALL_DIR to its path.');
  process.exit(2);
}
fs.mkdirSync(path.join(sdk, 'target'), {recursive:true});
const runDir = fs.mkdtempSync(path.join(sdk, 'target/zitadel-z10-'));
fs.chmodSync(runDir, 0o700);
const artifacts = path.join(sdk, 'output/playwright', path.basename(runDir));
fs.mkdirSync(artifacts, {recursive:true});
const session = path.basename(runDir);
const children = new Map(), containers = [];
const childEnv = { ...process.env, Z10_RUN_DIR:runDir, HTTP_PROXY:'', HTTPS_PROXY:'', ALL_PROXY:'', NO_PROXY:'127.0.0.1,localhost,127.0.0.1.nip.io' };
let fixture, tlsServer, browser = false, finishing = false;
const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
const check = (ok, message) => { if (!ok) throw Error(message); };
const notice = message => console.log(`${new Date().toISOString()} ${message}`);
async function command(name, binary, args, cwd=sdk, extra={}) {
  const logfile = fs.openSync(path.join(runDir, `${name}.log`), 'a', 0o600);
  try {
    await new Promise((resolve, reject) => {
      const child = spawn(binary, args, { cwd, env:{...childEnv,...extra}, stdio:['ignore',logfile,logfile] });
      child.on('error', reject); child.on('exit', code => code === 0 ? resolve() : reject(Error(`${name} failed (${code}); inspect ${name}.log`)));
    });
  } finally { fs.closeSync(logfile); }
}
async function output(binary, args, extra={}, cwd=sdk) {
  return await new Promise((resolve,reject) => {
    const child = spawn(binary,args,{cwd,env:{...childEnv,...extra},stdio:['ignore','pipe','pipe']});
    let stdout='', stderr=''; child.stdout.on('data',v=>stdout+=v); child.stderr.on('data',v=>stderr+=v);
    child.on('error',reject); child.on('exit',code=>code===0?resolve(stdout):reject(Error(`${binary} diagnostic failed: ${stderr.slice(-500)}`)));
  });
}
function service(name, binary, args, cwd, extra={}) {
  const log = fs.openSync(path.join(runDir, `${name}.log`), 'a', 0o600);
  const child = spawn(binary,args,{cwd,env:{...childEnv,...extra},stdio:['ignore',log,log]});
  fs.closeSync(log); children.set(name,child);
  child.on('error',err=>{ child.launchError=err; });
  return child;
}
async function until(checker, message, seconds=90) {
  const start = Date.now();
  while (Date.now()-start < seconds*1000) {
    if (!finishing) for (const [name,child] of children) {
      if (child.exitCode !== null || child.signalCode !== null || child.launchError) throw Error(`${name} exited; inspect ${name}.log`);
    }
    if (await checker()) return;
    await wait(250);
  }
  throw Error(message);
}
async function http(url, body, bearer, method) {
  return fetch(url, {method:method ?? (body === undefined?'GET':'POST'), signal:AbortSignal.timeout(10000),
    headers:{'Content-Type':'application/json',...(bearer?{Authorization:`Bearer ${bearer}`}:{})},
    ...(body===undefined?{}:{body:JSON.stringify(body)})});
}
async function json(url, body) { const res=await http(url,body); check(res.ok,'HTTP fixture/control request failed'); return res.json(); }
async function ready(url) { await until(async()=>{try{return(await http(url)).ok;}catch{return false;}},`not ready: ${url}`); }
async function pw(...args) {
  const result = await output('npx',['--yes','--package','@playwright/cli@0.1.19','playwright-cli','--session',session,...args],{},artifacts);
  fs.appendFileSync(path.join(artifacts,'cli.log'), result+'\n');
  check(!result.includes('### Error'), 'Playwright reported an error; inspect cli.log');
  return result;
}
async function freePort(port) {
  await new Promise((resolve,reject)=>{const server=net.createServer();server.once('error',()=>reject(Error(`port ${port} is occupied; will not kill its owner`)));server.listen(port,'127.0.0.1',()=>server.close(resolve));});
}
async function databaseContainer(name, port) {
  check(name.startsWith('zitadel-'), 'only explicitly task-owned containers can be reused');
  const info = JSON.parse(await output('docker',['inspect',name]))[0];
  check(info.State.Running, 'task database container is not running');
  const binding = info.HostConfig.PortBindings[`${port}/tcp`];
  check(binding?.length===1 && binding[0].HostIp==='127.0.0.1' && binding[0].HostPort===String(port), 'task DB port is not isolated loopback');
}
async function setupDatabases() {
  let pg=process.env.Z10_POSTGRES_CONTAINER, redis=process.env.Z10_REDIS_CONTAINER;
  check(Boolean(pg)===Boolean(redis), 'supply both task Postgres and Redis container names, or neither');
  if (!pg) {
    await freePort(5432); await freePort(6379);
    pg=`zitadel-z10-pg-${process.pid}`; redis=`zitadel-z10-redis-${process.pid}`;
    await command('postgres-create','docker',['run','-d','--name',pg,'--platform','linux/amd64','--label','auki.zitadel-local=true',
      '-p','127.0.0.1:5432:5432','-e','POSTGRES_USER=test','-e','POSTGRES_PASSWORD=test','-e','POSTGRES_DB=postgres','postgis/postgis:latest']); containers.push(pg);
    await command('redis-create','docker',['run','-d','--name',redis,'--label','auki.zitadel-local=true','-p','127.0.0.1:6379:6379','redis:alpine']); containers.push(redis);
    await until(async()=>{try{await output('docker',['exec',pg,'pg_isready','-U','test']);return true;}catch{return false;}},'task Postgres not ready');
  }
  await databaseContainer(pg,5432); await databaseContainer(redis,6379);
  const names = Object.fromEntries(['api','dds','dms'].map(name=>[name,`zitadel_z10_${name}_${process.pid}`]));
  for (const [name,db] of Object.entries(names)) {
    await command(`db-create-${name}`,'docker',['exec',pg,'createdb','-U','test',db]);
    if (name==='dms') continue; // production SQLx migration at DMS startup
    const repo=name==='dds'?'domain-service':'api';
    await command(`db-migrate-${name}`,'docker',['run','--rm','--name',`${session}-migrate-${name}`,'--network',`container:${pg}`,
      '-e',`MIGRATE_URL=postgres://test:test@127.0.0.1:5432/${db}?sslmode=disable`,'-e','MIGRATE_SOURCES=file:///data/migrations',
      '-e','WAIT_FOR_IT_HOST=127.0.0.1','-e','WAIT_FOR_IT_PORT=5432','-v',`${workspace}/${repo}/data/migrations:/data/migrations:ro`,'aukilabs/pgmigrator:latest']);
  }
  fs.writeFileSync(path.join(runDir,'databases.json'),JSON.stringify({postgres:pg,redis,names},null,2),{mode:0o600});
  return {pg,names};
}
async function tlsProxy() {
  const host='127.0.0.1.nip.io';
  const addresses=await dns.lookup(host,{all:true});
  check(addresses.length>0 && addresses.every(a=>a.address==='127.0.0.1'), 'relay DNS must resolve exclusively to loopback');
  await command('tls-certificate','openssl',['req','-x509','-newkey','rsa:2048','-nodes','-keyout',path.join(runDir,'tls-key.pem'),
    '-out',path.join(runDir,'tls-cert.pem'),'-subj',`/CN=${host}`,'-addext',`subjectAltName=DNS:${host}`,'-days','1']);
  fs.chmodSync(path.join(runDir,'tls-key.pem'),0o600);
  const cert=fs.readFileSync(path.join(runDir,'tls-cert.pem'));
  tlsServer=tls.createServer({cert,key:fs.readFileSync(path.join(runDir,'tls-key.pem'))}, socket=>{
    const upstream=net.connect(18127,'127.0.0.1'); socket.pipe(upstream);upstream.pipe(socket);
    socket.on('error',()=>upstream.destroy());upstream.on('error',()=>socket.destroy());
    socket.on('close',()=>upstream.destroy());upstream.on('close',()=>socket.destroy());
  });
  await new Promise((resolve,reject)=>{tlsServer.once('error',reject);tlsServer.listen(18125,'127.0.0.1',resolve);});
  const pin=createHash('sha256').update(new X509Certificate(cert).publicKey.export({type:'spki',format:'der'})).digest('base64');
  const configFile=path.join(artifacts,'playwright-cli.json');
  fs.writeFileSync(configFile,JSON.stringify({browser:{launchOptions:{args:[`--ignore-certificate-errors-spki-list=${pin}`,'--no-proxy-server']}}}));
  return configFile;
}
async function wireCases(seed) {
  check((await json('http://127.0.0.1:18120/__stats')).selectedDomainRows === 0,
    'selected ZITADEL Domains must be absent from the API catalog');
  const credentials=await json(`${identityBase}/__credentials/wire-check`);
  const res=await fetch(`${identityBase}/token`,{method:'POST',headers:{'Content-Type':'application/x-www-form-urlencoded'},
    body:new URLSearchParams({grant_type:'refresh_token',client_id:credentials.clientId,refresh_token:credentials.refreshToken})});
  check(res.ok,'synthetic wire grant failed');
  const tokens=await res.json();
  const issued=await http('http://127.0.0.1:18120/service/domains-access-token?purpose=p2p',{},tokens.access_token);
  check(issued.ok,'real API peer service issuance failed');
  const {access_token:bearer}=await issued.json();
  const claims=JSON.parse(Buffer.from(bearer.split('.')[1],'base64url').toString());
  check(claims.sub===seed.subject && claims.exp-claims.iat===3600,'service subject or normal TTL changed');
  const domains=await http('http://127.0.0.1:18121/api/v1/accessible-domains',undefined,bearer);
  const allowed=await domains.json();
  check(domains.ok && allowed.domains.length===1 && allowed.domains[0].id===seed.legacyDomainId,'DDS did not enforce legacy bridge read scope');
  for (const [url,method] of [[`/api/v1/domains/${seed.legacyDomainId}/`,'PUT'],[`/api/v1/domains/${seed.legacyDomainId}/auth`,'POST']]) {
    const denied=await http('http://127.0.0.1:18121'+url,{},bearer,method);
    check([401,403].includes(denied.status),'peer read token gained HTTP write/data-token rights');
  }
  for (const [url,method] of [[`/api/v1/domains/${seed.domainId}/`,'PUT'],[`/api/v1/domains/${seed.domainId}/auth`,'POST']]) {
    const denied=await http('http://127.0.0.1:18121'+url,{},tokens.access_token,method);
    check([401,403].includes(denied.status),'raw ZITADEL bearer gained legacy HTTP authority');
  }
  notice('PASS migration-only API bridge regression; selected SDK Domains absent from API; HTTP escalation denied');
  return bearer; // retained only in controller memory for literal expiry proof
}
async function cleanup() {
  finishing=true;
  if (fixture) fixture.state.stop=true;
  const native=children.get('native');
  if (native?.exitCode===null) await Promise.race([new Promise(resolve=>native.once('exit',resolve)),wait(20000)]);
  if (browser) { try { await pw('close'); } catch {} browser=false; }
  // The relay needs DMS alive to drain leases and release its provider session.
  for (const name of ['relay','dms']) {
    const child=children.get(name);
    if(child?.exitCode===null && child.signalCode===null) {
      child.kill('SIGTERM');
      await Promise.race([new Promise(resolve=>child.once('exit',resolve)),wait(10000)]);
    }
  }
  for (const port of [18120,18121]) {try{await http(`http://127.0.0.1:${port}/__stop`,{});}catch{}}
  for (const child of children.values()) {
    if(child.exitCode===null && child.signalCode===null) {
      await Promise.race([new Promise(resolve=>child.once('exit',resolve)),wait(5000)]);
      if(child.exitCode===null && child.signalCode===null)child.kill('SIGTERM');
    }
  }
  tlsServer?.close(); fixture?.server.close(); fixture?.host.close();
  for (const name of containers) {try{await command('container-stop','docker',['stop',name]);}catch{}}
  notice(`Local artifacts and isolated databases retained for inspection: ${runDir}`);
}
process.once('SIGINT',()=>{void cleanup().then(()=>process.exit(130));});
process.once('SIGTERM',()=>{void cleanup().then(()=>process.exit(143));});
try {
  notice(`Z13 direct-DDS ${mode}; working directory: ${runDir}`);
  const relayRevision=(await output('git',['rev-parse','HEAD'],{},hagall)).trim();
  notice(`Using Hagall relay source ${relayRevision}`);
  for (const port of [18120,18121,18122,18123,18125,18126,18127,18128,18129,18130,18131]) await freePort(port);
  const {pg,names}=await setupDatabases();
  fixture=startIdentityFixture(runDir,sdk);
  await ready(`${identityBase}/__stats`);
  const browserConfig=await tlsProxy();
  notice('Building the actual service entrypoints and SDK hosts');
  await Promise.all([
    command('api-build','go',['test','-c','-tags','zitadel_acceptance','-o',path.join(runDir,'api.test'),'./pkg/api'],path.join(workspace,'api')),
    command('dds-build','go',['test','-c','-tags','zitadel_acceptance','-o',path.join(runDir,'dds.test'),'./dds/http'],path.join(workspace,'domain-service')),
    command('relay-build','go',['build','-o',path.join(runDir,'relay'),'./cmd'],hagall),
    command('dms-build','cargo',['build','--example','zitadel_acceptance','--locked','--quiet'],path.join(workspace,'domain-manager-service'),{SQLX_OFFLINE:'true'}),
    command('native-build','cargo',['build','-p','auki-standard-protocols-native','--bin','zitadel_acceptance','--locked','--quiet']),
    command('web-build','npm',['run','check'],path.join(sdk,'bindings/web/auki-sdk-web')),
  ]);
  service('api',path.join(runDir,'api.test'),['-test.run','^TestZitadelAcceptanceAPI$','-test.v','-test.timeout',soak?'100m':'10m'],path.join(workspace,'api'),
    {APP_DIRECTORY:path.join(workspace,'api'),Z10_API_DATABASE_URL:`postgres://test:test@127.0.0.1:5432/${names.api}?sslmode=disable`});
  await ready('http://127.0.0.1:18120/__stats');
  service('dds',path.join(runDir,'dds.test'),['-test.run','^TestZitadelAcceptanceDDS$','-test.v','-test.timeout',soak?'100m':'10m'],path.join(workspace,'domain-service'),
    {Z10_DDS_DATABASE_URL:`postgres://test:test@127.0.0.1:5432/${names.dds}?sslmode=disable`});
  await ready('http://127.0.0.1:18121/__stats');
  service('dms',path.join(workspace,'domain-manager-service/target/debug/examples/zitadel_acceptance'),[],path.join(workspace,'domain-manager-service'),
    {DATABASE_URL:`postgres://test:test@127.0.0.1:5432/${names.dms}?sslmode=disable`,PORT:'18122',ADMIN_PORT:'18131',
      DDS_ISSUER:'dds',DDS_AUDIENCE:'http://127.0.0.1:18121',DDS_SERVICE_PUBLIC_KEY_URL:'http://127.0.0.1:18121/service/public-key.pem',
      DDS_P2P_VERIFICATION_KEYS_URL:'http://127.0.0.1:18121/service/p2p-verification-keys',DDS_ADMIN_URL:'http://127.0.0.1:18121',CREDIT_LOCKING_ENABLED:'false'});
  await ready('http://127.0.0.1:18122/health');
  const relay=JSON.parse(fs.readFileSync(path.join(runDir,'relay.json')));
  service('relay',path.join(runDir,'relay'),[],hagall,{
    RELAY_DDS_URL:'http://127.0.0.1:18121',RELAY_DMS_URL:'http://127.0.0.1:18122/v1',RELAY_DDS_PUBLIC_KEY_URL:'http://127.0.0.1:18121/service/p2p-verification-keys',
    RELAY_LOCAL_TEST_ALLOW_HTTP:'true',RELAY_REGISTRATION_CREDENTIALS_FILE:path.join(runDir,'relay-registration'),
    RELAY_WALLET_PRIVATE_KEY_FILE:path.join(runDir,'relay-wallet'),RELAY_LIBP2P_PRIVATE_KEY_FILE:path.join(runDir,'relay-peer-key'),
    RELAY_PUBLIC_BASE_MULTIADDRS:`/dns4/127.0.0.1.nip.io/tcp/18126/p2p/${relay.peerId},/dns4/127.0.0.1.nip.io/tcp/18125/wss/p2p/${relay.peerId}`,
    RELAY_TCP_LISTEN_MULTIADDR:'/ip4/127.0.0.1/tcp/18126',RELAY_WS_LISTEN_MULTIADDR:'/ip4/127.0.0.1/tcp/18127/ws',
    RELAY_ADMIN_ADDR:'127.0.0.1:18128',RELAY_METRICS_ADDR:'127.0.0.1:18129',RELAY_SHUTDOWN_DRAIN:'1s',RELAY_ACCEPT_BOOKINGS:'true',
  });
  await ready('http://127.0.0.1:18128/readyz');
  const seed=JSON.parse(fs.readFileSync(path.join(runDir,'seed.json')));
  const oldServiceBearer=await wireCases(seed);
  service('native',path.join(sdk,'target/debug/zitadel_acceptance'),[],sdk);
  await until(()=>fixture.state.events.some(e=>e.runtime==='native'&&e.event==='ready'),'native peers did not start',120);
  browser=true;
  await pw('open','http://127.0.0.1:18130/','--browser','chrome','--config',browserConfig);
  await pw('snapshot');
  await until(()=>{
    check(!fixture.state.events.some(e=>e.event==='failed'),'host reported failure; inspect events/host logs');
    return fixture.state.events.some(e=>e.runtime==='browser'&&e.event==='ready');
  },'browser peers did not start',120);
  const readyEvents=fixture.state.events.filter(e=>e.event==='ready');
  const ids=readyEvents.flatMap(e=>e.peerIds);check(ids.length===4&&new Set(ids).size===4,'expected four distinct native/browser peers');
  // Exactly one deliberate bridge regression and two legacy password/app
  // exchanges. ZITADEL SDK startup and all future renewals must add none.
  const expectedApiExchanges = 3;
  check((await json('http://127.0.0.1:18120/__stats')).exchanges === expectedApiExchanges,
    'ZITADEL SDK unexpectedly called the API exchange');
  notice(`Four direct-DDS SDK peers ready on a DDS-only Domain; ${soak?'optional normal-lifetime soak (~68 minutes)':'short smoke (two minutes of relay traffic)'}`);
  const started=Date.now();
  let lastNotice=0, lastSuccess={native:started,browser:started};
  for (;;) {
    check(Date.now()-started < (soak?80:5)*60000,`${mode} deadline`);
    for (const [name,child] of children) check(child.exitCode===null && child.signalCode===null,`${name} stopped during ${mode}`);
    check(!fixture.state.events.some(e=>e.event==='failed'),`host reported failure during ${mode}`);
    const probeCounts = {};
    for (const runtime of ['native','browser']) {
      const crossRuntimeField=runtime==='native'?'browserProbes':'nativeProbes';
      probeCounts[runtime]=fixture.state.events.filter(e=>e.runtime===runtime&&e.event==='probe'&&e[crossRuntimeField]>0&&Date.parse(e.receivedAt)>=started).length;
      const latest=fixture.state.events.findLast(e=>e.runtime===runtime&&e.event==='probe');
      if(latest && latest[crossRuntimeField]>0)lastSuccess[runtime]=Date.parse(latest.receivedAt);
      check(Date.now()-lastSuccess[runtime]<120000,`${runtime} has no successful cross-runtime relay probe for two minutes`);
    }
    const stats=await json('http://127.0.0.1:18121/__stats');
    const rounds=ids.map(id=>stats.issued[id]?.length??0);
    for(const id of ids) for(const claims of stats.issued[id]??[])check(claims.direct===true&&claims.domainId===seed.domainId&&claims.subject===seed.subject&&claims.exp-claims.iat===1800,'direct DDS subject, Domain or 30-minute lifetime changed');
    check((await json('http://127.0.0.1:18120/__stats')).exchanges === expectedApiExchanges,
      'ZITADEL renewal unexpectedly called API');
    if(Date.now()-lastNotice>45000){notice(`${mode} ${Math.floor((Date.now()-started)/1000)}s; P2P generations ${rounds.join('/')}; successful native/browser probe cycles ${probeCounts.native}/${probeCounts.browser}`);lastNotice=Date.now();}
    if(Date.now()-started>=120000 && Object.values(probeCounts).every(n=>n>=3) &&
      rounds.every(n=>n>=(soak?4:1)) &&
      ['native-live','browser-live'].every(name=>fixture.state.grants.get(name).refreshes>=(soak?2:1)))break;
    await wait(5000);
  }
  if (soak) {
    const expired=await http('http://127.0.0.1:18121/api/v1/accessible-domains',undefined,oldServiceBearer);
    check(expired.status===401,'expired original API bearer accepted by DDS');
    check((await json('http://127.0.0.1:18121/__expired-proof')).status===401,'expired original DDS P2P bearer accepted');
  }
  const rows=JSON.parse(await output('docker',['exec',pg,'psql','-U','test','-d',names.dms,'-At','-c',
    "SELECT COALESCE(json_agg(json_build_object('subject',requester_id,'peerId',target_peer_id,'domainId',domain_id,'state',state)), '[]'::json) FROM relay_bookings"]));
  check(ids.every(id=>rows.some(r=>r.peerId===id&&r.subject===seed.subject&&r.domainId===seed.domainId&&r.state==='active')),'DMS booking subject/owner changed');
  fixture.state.revoked=true;
  await until(()=>{
    check(!fixture.state.events.some(e=>e.event==='failed'),'host failed during revocation check');
    return fixture.state.events.some(e=>e.event==='revocation-checked'&&e.nativeProbes>0);
  },'revocation delay case did not finish',90);
  notice('PASS direct DDS admission, no SDK API exchanges, stable Peer IDs, relay/discovery and accepted revocation delay');
  notice(soak?'PASS normal-lifetime P2P/ZITADEL renewals and literal expiry':'NOT RUN in smoke: sustained P2P renewals and literal expiry; use focused lifecycle tests, with --soak optional');
  fixture.state.stop=true; finishing=true;
  await until(()=>{
    check(!fixture.state.events.some(e=>e.event==='failed'),'host failed during shutdown');
    return ['native','browser'].every(runtime=>fixture.state.events.some(e=>e.runtime===runtime&&e.event==='stopped'));
  },'hosts did not close cleanly',60);
  await pw('snapshot');
  fs.writeFileSync(path.join(runDir,'result.json'),JSON.stringify({passed:true,mode,relayRevision,sustainedRenewalChecked:soak,literalExpiryChecked:soak,ids,elapsedSeconds:Math.round((Date.now()-started)/1000),
    rows,grants:Object.fromEntries(fixture.state.grants)},null,2),{mode:0o600});
  notice(`PASS Z13 direct-DDS real-service local ${mode}`);
} catch(error) {
  notice(`FAIL ${error.message}`);process.exitCode=1;
} finally { await cleanup(); }
