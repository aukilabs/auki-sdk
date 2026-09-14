# Work with Domain data

Use `AukiDomains` and `AukiDomainData` without starting networking. They share an
`AukiCredential` (the existing `AuthSession`) with `AukiPeerBootstrap`.
User password login works in native Rust and Rust compiled to WASM. App key/secret
login is native-only and belongs on trusted backends.

```rust,no_run
use auki_sdk::{
    AukiDomainData, AukiDomains, AukiPeerBootstrap, AukiPeerConfig, AuthClient,
    AuthEnvironment, Credentials, DataListQuery, DataWrite, DomainListQuery,
};

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let env = AuthEnvironment::dev()
    .with_client_id(std::env::var("AUKI_CLIENT_ID")?)?;
let credential = AuthClient::new(env)?
    .authenticate(Credentials::user_password(
        std::env::var("AUKI_EMAIL")?, std::env::var("AUKI_PASSWORD")?,
    )).await?;
let result: Result<(), Box<dyn std::error::Error>> = async {
    let domains = AukiDomains::new(credential.clone());
    let page = domains.list(&DomainListQuery { limit: 50, offset: 0, ..Default::default() }).await?;
    // The caller selects a Domain; a known ID also works without listing.
    let domain_id = std::env::var("AUKI_DOMAIN_ID")?.parse()?;
    let data = AukiDomainData::new(credential.clone())?.in_domain(domain_id);
    let records = data.list(&DataListQuery {
        data_type: Some("my-app.report.v1".into()), ..Default::default()
    }).await?;
    if let Some(record) = records.first() {
        let metadata = data.get(record.id).await?;
        let bytes = data.read(metadata.id).await?;
    }
    let saved = data.write(DataWrite::Named {
        name: "report", data_type: "my-app.report.v1",
    }, b"application-defined bytes").await?;
    data.write(DataWrite::ById(saved.id), b"replacement bytes").await?;
    data.delete(saved.id).await?;
    data.close().await;
    // Optional networking uses this same credential; construction starts no peer.
    let bootstrap = AukiPeerBootstrap::from_session(credential.clone(), AukiPeerConfig::dev());
    Ok(())
}.await;
credential.close().await;
result?;
# Ok(())
# }
```

Persist one `AUKI_CLIENT_ID` per application installation and reuse it across
logins. DDS uses this value for access accounting. Without an explicit value,
the environment generates an ID shared by its clones for that environment's
lifetime. Metadata listing sends `issue_token=false` and SDK identification;
it does not obtain potentially billed tokens for each picker entry.

For another environment, use `AuthEnvironment::new(api, dds)` with aligned
endpoints. The selected Domain's server URL comes from authenticated DDS.
The client caches renewable data grants by credential and Domain, checks their
issuer/Domain/audience/expiry before sending, and obtains one replacement after a 401.
P2P uses its own signed peer-bound credentials. The Domain Server checks data
signatures and permissions on every request; successful discovery or P2P
admission does not establish write/delete access.

## Data and pagination semantics

- `DomainListQuery` supports `own` (default), an organization UUID, and `all`
  only with an owned Domain Server. It uses real DDS `limit`/`offset` pages
  (1–100). Totals may change between pages; they are not snapshot guarantees.
- Listing a Domain establishes visibility, not effective read/write permission.
  Permission filters require backend issue #383 and are not exposed here.
- Data lists support `ids`, `name` and `data_type`, returning metadata only.
  They have no server pagination; oversized responses fail explicitly.
- Named writes use the simple POST upload endpoint, which returns 409 for a
  duplicate name. ID replacement preserves the name/type and fails for a missing
  ID. No lookup-and-overwrite fallback runs on conflict. The server's separate
  multipart-session completion path can replace by name; it is not used here.
  There is no version history, automatic merging or replay after a lost response.
  Names/types are 1–128 bytes and follow the server's punctuation restrictions;
  `/` is disallowed, so use a type such as `my-app.report.v1`.
- Buffered transfers default to 8 MiB of data and 1 MiB of metadata, configurable
  through `DataLimits` up to 64 MiB/4 MiB. Uploads also check the selected server's
  `/api/v1/info` request/file limits, including multipart envelope overhead.
  Large-file streaming and resumable multipart sessions are separate work.
- `DataError::status()` preserves HTTP distinctions such as 401, 402, 403, 404,
  413, 429 and 5xx. Bodies, tokens and application data are excluded from errors.
  Retries are limited to a single confirmed 401; a timeout, cancellation or
  connection loss after a write may leave its outcome unknown. Reconcile it
  before resubmitting.

## Cancellation and shutdown

Every data operation has a `_with_cancellation` variant taking a
`tokio_util::sync::CancellationToken`. Dropping the future stops its request.
`DomainDataClient::close().await` cancels and drains that client's operations,
including its clones. It leaves separately created clients and peers usable.
There are no background data tasks or refresh loops.

The application closes all users of the credential before calling
`credential.close().await`. This fences all session clones and clears cached
Domain grants. See [peer cleanup](lifecycle.md) and [authentication](authenticate.md)
for logout and imported-session persistence. Imported ZITADEL data operations
return an explicit unsupported-configuration error in this milestone, without
refreshing or changing stored credentials; existing P2P import behavior is unchanged.

## Try the dev round trip

The [example](../../core/auki-sdk/examples/domain_data.rs) requires `AUKI_EMAIL`,
`AUKI_PASSWORD`, a selected `AUKI_DOMAIN_ID`, and a persistent `AUKI_CLIENT_ID`.
It signs in to dev, lists Domains, creates a uniquely named small data record,
reads it, replaces it by ID, checks duplicate-name conflicts, and deletes it. Cleanup reconciles by the
unique name even when a write response is lost. It leaves existing records alone.

Run only with an approved dev account/Domain that permits these operations:

```sh
cargo run --locked -p auki-sdk --example domain_data
# Also start and stop a direct-only peer sharing the credential:
cargo run --locked -p auki-sdk --example domain_data -- --with-peer
```

The optional peer uses no relay booking or discovery publication. It verifies
shared authentication/lifecycle, not a two-peer exchange. No environment file is
loaded automatically; supply credentials through your shell or secret loader.

## Existing Posemesh consumers

This is an additive SDK API. Existing `posemesh-domain-http`, `@auki/domain-client`
and Python clients remain available, including their administration and task
helpers. Rust callers can migrate data/discovery operations to the types above
and replace independent login calls with a cloned SDK session. The server's
`data_type`, IDs, names and bytes retain their meaning. JavaScript/Python/Swift/Expo
data APIs will follow; current bindings still expose networking only.

Machine/task credential adapters belong to #375 integration. Portal/pose wrappers,
streaming and imported ZITADEL data integration remain within later #374 milestones;
permission-aware discovery, identity reconciliation and additional pagination
depend on #383–#385. This change does not require a backend contract change.
