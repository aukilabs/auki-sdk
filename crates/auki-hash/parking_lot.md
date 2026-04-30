# Parking lot — auki-hash

---

## Cryptographic strength upgrade path

Current: XXH3-128, fast and non-cryptographic. Adversarial collision-resistance isn't a stated requirement today, but if the SDK eventually supports signed commits, tamper detection, or third-party domain trust, we'd need to swap to a cryptographic hash (e.g. BLAKE3). When does this swap happen, and how do we tag content-addressed entries to support both algorithms during transition? Or do we accept a hard cutover and rebuild registries?
