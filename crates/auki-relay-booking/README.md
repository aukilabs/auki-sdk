# auki-relay-booking

Strict, bounded client and validated wire types for the DMS relay-booking API.

This crate owns the requester-side HTTP contract. It does not select a relay,
run a libp2p node, recover a reservation, or supervise application authority.
Those lifecycle decisions belong to the native or browser SDK facade.

## Browser transport contract

The Wasm build uses browser Fetch. DMS must answer the requested endpoint
without a redirect and allow cross-origin `GET`, `POST`, and `DELETE` requests
with `Authorization`, `Content-Type`, and `Idempotency-Key`. It must expose
`Location` and `Retry-After` response headers. Redirected final URLs are
rejected, but the browser may already have followed the redirect; DMS endpoints
therefore must not redirect authenticated requests.
