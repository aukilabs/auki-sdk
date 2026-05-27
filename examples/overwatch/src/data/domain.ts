// Local-only cache of the last-accepted Domain name. Used purely as a
// warmup hint for the topbar chip during the ~1s gap between modal
// submit and the next `/api/cluster/status` poll — the cluster poller
// is the source of truth.
//
// Key shape mirrors `data/seenDaemons.ts`'s `auki.park.*.v<N>` scheme
// so all of Park's UI state is greppable under the same prefix.

const STORAGE_KEY = "auki.park.domain.v1";

/** Read the saved domain name. Returns `null` when nothing has been
 * set yet (first boot) or when the saved value is the empty string. */
export function getDomainName(): string | null {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw === null || raw.length === 0) return null;
    return raw;
  } catch {
    return null;
  }
}

/** Persist the canonical form of the domain name. The caller is
 * expected to have already passed `canonicalize` over the input —
 * `data/domainName.ts` is the source of truth for that. */
export function setDomainName(canonical: string): void {
  try {
    localStorage.setItem(STORAGE_KEY, canonical);
  } catch {
    // Quota / SecurityError / etc. — best-effort persistence. The
    // modal will simply reprompt on next boot, which is a reasonable
    // degradation.
  }
}

/** Forget the saved domain. Next boot reprompts. */
export function clearDomainName(): void {
  try {
    localStorage.removeItem(STORAGE_KEY);
  } catch {
    // Same defensive shape as setDomainName. If we can't remove the
    // key, the next-load prompt won't appear — operator can fall back
    // to devtools.
  }
}

// Exposed so tests can pin the key shape and other modules can
// invalidate caches if needed.
export const DOMAIN_STORAGE_KEY = STORAGE_KEY;
