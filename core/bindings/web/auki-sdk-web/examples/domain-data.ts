import type { AukiUserSession, DataSink } from "../pkg-test/auki_sdk_web";

/** Uses an already initialized shared login and an explicitly selected Domain.
 * Creates a unique record, streams a Blob/File to it and back, and deletes it.
 * The caller owns the destination and its partial-file cleanup on failure.
 */
export async function roundTrip(
  session: AukiUserSession,
  domainId: string,
  source: Blob,
  sink: DataSink,
  signal?: AbortSignal,
) {
  const data = session.data(domainId);
  const name = `sdk-374-web-${crypto.randomUUID()}`;
  try {
    const domains = session.domains();
    const page = await domains.list({ limit: 50 }, signal);
    const portals = await domains.portals(domainId, signal);
    const poses = await data.poses(signal);
    if (portals.length) {
      await domains.portal(domainId, portals[0].id, signal);
      await domains.forPortal(portals[0].short_id, "own", signal);
    }
    if (poses.length) await data.pose(poses[0].id, signal);
    let offset = 0;
    const saved = await data.writeStream(
      { name, dataType: "sdk-test.file.v1" },
      source.size,
      async maximum => {
        const end = Math.min(source.size, offset + maximum);
        const bytes = new Uint8Array(await source.slice(offset, end).arrayBuffer());
        offset = end;
        return bytes;
      },
      undefined,
      signal,
    );
    const downloaded = await data.readTo(saved.id, sink, { maxBytes: source.size, maxChunkBytes: 65536 }, signal);
    return { metadata: saved, downloaded, domains: page.total, portals: portals.length, poses: poses.length };
  } finally {
    try {
      // Cleanup uses fresh cancellation state after a cancelled transfer.
      const records = await data.list({ name });
      const results = await Promise.allSettled(records.map(record => data.delete(record.id)));
      const failure = results.find(result => result.status === "rejected");
      if (failure?.status === "rejected") throw failure.reason;
      if ((await data.list({ name })).length) throw Error("Temporary record remains");
    } finally {
      await data.close();
    }
  }
}
