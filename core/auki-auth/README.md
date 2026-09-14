# auki-auth

Sign in with User or App credentials, or import a ZITADEL session, to access
an Auki Domain. `auki-sdk` uses this crate when starting a peer.

See [Sign in and choose a Domain](../../docs/how-to/authenticate.md).
The same session (`AukiCredential`) also supplies renewable
[Domain data access](../../docs/how-to/domain-data.md), without a peer or DMS.
