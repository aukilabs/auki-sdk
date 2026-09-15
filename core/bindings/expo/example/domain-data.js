import { data, domains } from '@aukilabs/auki-sdk-expo';

/**
 * Round-trip a Blob/File using an existing session and an explicitly selected
 * Domain. The caller owns its destination and partial-file cleanup on failure.
 */
export async function roundTrip(session, domainId, source, destination, signal) {
  const domainData = await data(session, domainId);
  const name = `sdk-374-expo-${Date.now()}`;
  try {
    const directory = domains(session);
    const page = await directory.list({ limit: 50 }, signal);
    const portals = await directory.portals(domainId, signal);
    const poses = await domainData.poses(signal);
    if (portals.length) {
      await directory.portal(domainId, portals[0].id, signal);
      await directory.forPortal(portals[0].short_id, 'own', signal);
    }
    if (poses.length) await domainData.pose(poses[0].id, signal);

    let offset = 0;
    const saved = await domainData.writeStream(
      { name, dataType: 'sdk-test.file.v1' },
      source.size,
      async maximum => {
        const end = Math.min(source.size, offset + maximum);
        const chunk = new Uint8Array(await source.slice(offset, end).arrayBuffer());
        offset = end;
        return chunk;
      },
      { maxBytes: source.size },
      signal,
    );
    const downloaded = await domainData.readTo(
      saved.id,
      chunk => destination.write(chunk),
      { maxBytes: source.size },
      signal,
    );
    return { saved, downloaded, domains: page.total, portals: portals.length, poses: poses.length };
  } finally {
    try {
      // Use fresh cancellation state to reconcile and remove an interrupted upload.
      const records = await domainData.list({ name });
      const results = await Promise.allSettled(records.map(record => domainData.delete(record.id)));
      const failure = results.find(result => result.status === 'rejected');
      if (failure?.status === 'rejected') throw failure.reason;
    } finally {
      await domainData.close();
    }
  }
}
