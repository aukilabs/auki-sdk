import {
  parseBootstrapRecords,
  type AukiBrowserBootstrapRecord,
  type OfferSummary,
} from "@aukilabs/auki-p2p-browser";

export function parseBootstrapText(text: string): AukiBrowserBootstrapRecord[] {
  return parseBootstrapRecords(JSON.parse(text));
}

export function mergeBootstrapRecords(
  current: AukiBrowserBootstrapRecord[],
  incoming: AukiBrowserBootstrapRecord[],
): AukiBrowserBootstrapRecord[] {
  const byPeer = new Map<string, AukiBrowserBootstrapRecord>();
  for (const record of current) {
    byPeer.set(record.peerId, record);
  }
  for (const record of incoming) {
    byPeer.set(record.peerId, record);
  }
  return Array.from(byPeer.values());
}

export function offerLabel(offer: OfferSummary | undefined): string {
  if (!offer) {
    return "None";
  }
  return `${shortId(offer.peerId)}/${shortId(offer.domainId)}/${offer.offerId}`;
}

export type OfferActionState = {
  getting?: boolean;
  subscribing?: boolean;
  stopping?: boolean;
  subscription?: unknown;
};

export function canRequestSnapshot(hasPeer: boolean, runtime: OfferActionState): boolean {
  return Boolean(
    hasPeer &&
      !runtime.getting &&
      !runtime.subscribing &&
      !runtime.stopping &&
      !runtime.subscription,
  );
}

export function shortId(value: string, visible = 8): string {
  if (value.length <= visible * 2 + 1) {
    return value;
  }
  return `${value.slice(0, visible)}...${value.slice(-visible)}`;
}
