import Auki, {
  closeSession,
  data,
  domains,
  importZitadelSession,
} from '@aukilabs/auki-sdk-expo';

const loopbackHost = '127.0.0.1';
export const domainDataBase = `http://${loopbackHost}:18114`;
export const domainFixture = async (path, value) => {
  const response = await fetch(domainDataBase + path, value === undefined ? {} : {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(value),
  });
  if (!response.ok) throw new Error(`fixture HTTP ${response.status}`);
  return response.json();
};

const DOMAIN_ID = 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa';
const INITIAL_DATA_ID = 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb';
const PORTAL_ID = 'eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee';
const DENIED_DATA_ID = 'ffffffff-ffff-4fff-8fff-ffffffffffff';
const sameBytes = (actual, expected) =>
  actual.length === expected.length && actual.every((value, index) => value === expected[index]);
const check = (condition, message) => { if (!condition) throw new Error(message); };
const gate = () => { let resolve; const promise = new Promise(done => { resolve = done; }); return { promise, resolve }; };

export async function runDomainDataCases(report) {
  await domainFixture('/__reset', {});
  const session = await Auki.loginWithEnvironment(
    domainDataBase,
    domainDataBase,
    domainDataBase,
    'expo@example.test',
    'password',
    'expo-domain-fixture',
  );
  report('PASS password session opened without a peer');
  await domainFixture('/__configure', { domainsUnauthorizedOnce: true });

  let client;
  try {
    const directory = domains(session);
    const page = await directory.list({ limit: 1, offset: 0 });
    check(page.total === 1 && page.limit === 1 && page.offset === 0, 'paged Domain result lost fields');
    check(page.domains[0]?.id === DOMAIN_ID, 'paged Domain result lost its Domain');

    const portals = await directory.portals(DOMAIN_ID);
    const portal = await directory.portal(DOMAIN_ID, PORTAL_ID);
    const portalDomains = await directory.forPortal(portal.short_id);
    check(portals.length === 1 && portal.id === PORTAL_ID, 'portal lookup lost fields');
    check(portalDomains[0]?.id === DOMAIN_ID && portalDomains[0]?.is_default, 'portal Domain lookup lost fields');
    report('PASS paged Domains and portal metadata');

    client = await data(session, DOMAIN_ID);
    const records = await client.list({ dataType: 'fixture.v1' });
    const initial = await client.get(INITIAL_DATA_ID);
    const initialBytes = await client.read(INITIAL_DATA_ID);
    check(records.length === 1 && initial.name === 'fixture', 'data metadata lookup lost fields');
    check(sameBytes(initialBytes, new Uint8Array([102, 105, 120, 116, 117, 114, 101])), 'buffered read changed bytes');

    const bufferedBytes = new Uint8Array([1, 2, 3]);
    const buffered = await client.write(
      { name: 'expo-buffered', dataType: 'fixture.expo.v1' },
      bufferedBytes,
    );
    check(sameBytes(await client.read(buffered.id), bufferedBytes), 'buffered round trip changed bytes');

    const poses = await client.poses();
    const pose = await client.pose(PORTAL_ID);
    check(poses.length === 1 && pose.domain_id === DOMAIN_ID && pose.px === 1, 'pose lookup lost fields');
    report('PASS metadata, buffered CRUD, and poses');

    const streamedBytes = new Uint8Array([10, 11, 12, 13, 14, 15, 16, 17, 18]);
    let sourceOffset = 0;
    let eofCalls = 0;
    const streamed = await client.writeStream(
      { name: 'expo-streamed', dataType: 'fixture.expo.v1' },
      streamedBytes.length,
      maximum => {
        if (sourceOffset === streamedBytes.length) {
          eofCalls++;
          return new Uint8Array();
        }
        const end = Math.min(streamedBytes.length, sourceOffset + maximum);
        const chunk = streamedBytes.slice(sourceOffset, end);
        sourceOffset = end;
        return chunk;
      },
      { maxBytes: 64, maxChunkBytes: 4 },
    );
    const downloaded = [];
    const downloadedSize = await client.readTo(
      streamed.id,
      async chunk => { downloaded.push(...chunk); },
      { maxBytes: 64, maxChunkBytes: 3 },
    );
    check(downloadedSize === streamedBytes.length, 'streamed download size changed');
    check(sameBytes(new Uint8Array(downloaded), streamedBytes), 'streamed round trip changed bytes');
    check(eofCalls === 1, 'upload did not request explicit EOF exactly once');
    report('PASS backpressured multipart upload and streamed download');

    try {
      await client.get(DENIED_DATA_ID);
      throw new Error('denied data unexpectedly readable');
    } catch (error) {
      check(error.kind === 'http' && error.status === 403, 'HTTP denial lost kind or status');
    }
    report('PASS structured permission failure');

    const secondSourceCall = gate();
    const abort = new AbortController();
    let sourceCalls = 0;
    const cancelled = client.writeStream(
      { name: 'expo-cancelled', dataType: 'fixture.expo.v1' },
      8,
      () => {
        sourceCalls++;
        if (sourceCalls === 1) return new Uint8Array([20, 21, 22, 23]);
        secondSourceCall.resolve();
        return new Promise(() => {});
      },
      { maxBytes: 64, maxChunkBytes: 4 },
      abort.signal,
    );
    await secondSourceCall.promise;
    abort.abort();
    try {
      await cancelled;
      throw new Error('cancelled upload unexpectedly completed');
    } catch (error) {
      check(error.kind === 'cancelled', 'upload cancellation lost its kind');
    }

    await client.delete(buffered.id);
    await client.delete(streamed.id);
    const stats = await domainFixture('/__stats');
    check(stats.refreshes === 1 && stats.exchanges >= 2, '401 renewal did not refresh once');
    check(stats.multipartCompletions === 1, 'multipart completion was not recorded');
    check(stats.multipartAborts === 1 && stats.outstandingUploads === 0, 'cancelled multipart was not drained');
    check(stats.records === 1, 'test records were not deleted');
    report('PASS renewal, cancellation, cleanup, and deletion');
  } finally {
    if (client) await client.close();
    await closeSession(session);
  }

  await domainFixture('/__reset', {});
  const credentials = {
    accessToken: 'imported-access-0',
    refreshToken: 'imported-refresh-0',
    clientId: 'domain-data-public-client',
    issuer: domainDataBase,
    accessTokenExpiresAt: '2000-01-01T00:00:00Z',
  };
  const snapshots = [];
  let rejectSave = true;
  const importedSession = await importZitadelSession(credentials, async snapshot => {
    snapshots.push(snapshot.exposeRefreshToken());
    if (rejectSave) throw new Error('DO_NOT_LEAK_STORAGE_FAILURE');
  }, {
    apiBaseUrl: domainDataBase,
    ddsBaseUrl: domainDataBase,
    dmsBaseUrl: domainDataBase,
  });
  let importedClient;
  try {
    importedClient = await data(importedSession, DOMAIN_ID);
    try {
      await importedClient.get(INITIAL_DATA_ID);
      throw new Error('failed imported save unexpectedly authorized data');
    } catch (error) {
      check(error.kind === 'auth' && error.code === 'persistence', 'imported save failure lost auth persistence code');
      check(!String(error).includes('DO_NOT_LEAK'), 'storage failure detail crossed the bridge');
    }
    let stats = await domainFixture('/__stats');
    check(stats.importedRefreshes === 1 && stats.exchanges === 0, 'exchange ran before imported credentials were saved');

    rejectSave = false;
    check((await importedClient.get(INITIAL_DATA_ID)).id === INITIAL_DATA_ID, 'retained imported session retry failed');
    stats = await domainFixture('/__stats');
    check(stats.importedRefreshes === 1 && snapshots.join(',') === 'imported-refresh-1,imported-refresh-1', 'imported retry rotated credentials twice');

    const portalsPage = await domains(importedSession).portalsPage(DOMAIN_ID, 1);
    check(portalsPage.items.length === 1 && portalsPage.paginated === false,
      'portal page lost its bounded legacy acknowledgement');

    const page = await domains(importedSession).list({ limit: 1, offset: 1 });
    check(page.total === 2 && page.limit === 1 && page.offset === 1
      && page.domains[0]?.id === 'dddddddd-dddd-4ddd-8ddd-dddddddddddd',
    'imported server pagination lost its Domain or page fields');
    stats = await domainFixture('/__stats');
    check(stats.p2pExchanges === 0
      && stats.requests['GET /api/v1/domains'] === 1,
    'imported owner listing did not use the ordinary User Domain route');

    const listingCalls = stats.requests['GET /api/v1/domains'] ?? 0;
    try {
      await domains(importedSession).list({ organization: 'all', limit: 1 });
      throw new Error('imported filtered Domain listing unexpectedly succeeded');
    } catch (error) {
      check(error.kind === 'auth' && error.code === 'configuration', 'imported filtered listing lost unsupported classification');
    }
    stats = await domainFixture('/__stats');
    check((stats.requests['GET /api/v1/domains'] ?? 0) === listingCalls,
      'unsupported imported filter performed provider I/O');
  } finally {
    if (importedClient) await importedClient.close();
    await closeSession(importedSession);
  }
  report('PASS imported persistence retry, paged Domains, and known-Domain data');

  const importedListingFailure = async (config, expectedCode) => {
    await domainFixture('/__reset', config);
    const session = await importZitadelSession({
      ...credentials,
      accessTokenExpiresAt: '2099-01-01T00:00:00Z',
    }, async () => { throw new Error('unexpected credential save'); }, {
      apiBaseUrl: domainDataBase,
      ddsBaseUrl: domainDataBase,
      dmsBaseUrl: domainDataBase,
    });
    try {
      await domains(session).list({ limit: 1 });
      throw new Error(`imported listing unexpectedly accepted ${expectedCode}`);
    } catch (error) {
      check(error.kind === 'auth' && error.code === expectedCode,
        `imported listing lost ${expectedCode} classification`);
    } finally {
      await closeSession(session);
    }
    const stats = await domainFixture('/__stats');
    check(stats.exchanges === 2 && stats.p2pExchanges === 1
      && (stats.requests['GET /api/v1/domains'] ?? 0) === 0
      && (stats.requests['GET /api/v1/accessible-domains'] ?? 0) === 0,
    'viewer grant reached DDS or skipped role classification');
  };
  await importedListingFailure({ importedViewer: true, denyP2pExchange: true }, 'authorization_denied');
  await importedListingFailure({ importedViewer: true, wrongP2pToken: true }, 'configuration');
  report('PASS viewer listing denies permission without an unsafe App fallback');

  report('PASS 8 Expo Domain data host cases');
  await domainFixture('/__phase', { phase: 'passed', count: 8 });
}
