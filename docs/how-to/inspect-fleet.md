# Inspect a fleet

Reuse an [authenticated session](authenticate.md) and an explicitly selected
Domain. Fleet reads do not start a peer. Use a Domain inventory for assigned
robots and observed Domain workers, or a separate compute pool for candidates.

## Rust

~~~rust,no_run
use auki_sdk::{AukiFleet, AuthSession, ComputePoolQuery, FleetError, FleetQuery, JobMode};
use uuid::Uuid;

async fn inspect(session: AuthSession, dms_url: &str, domain: Uuid) -> Result<(), FleetError> {
    let client = AukiFleet::new(session, dms_url)?.in_domain(domain);
    let result = async {
        let inventory = client.list(&FleetQuery::default()).await?;
        for machine in inventory.machines {
            println!("{} {:?} {:?}", machine.name, machine.presence, machine.work_state);
        }
        let pool = client.compute_pool(&ComputePoolQuery {
            mode: JobMode::Dedicated,
            capabilities: vec!["vendor.example/inspect/v7".into()],
            match_all_capabilities: true,
        }).await?;
        println!("{} candidates; sources: {:?}", pool.machines.len(), pool.sources);
        Ok(())
    }.await;
    client.close().await;
    result
}
~~~

`AukiPeerBootstrap::fleet()` also uses its existing session and configured DMS
URL. `AukiFleet::with_limits` lets Rust callers reduce traversal or deadline
bounds. Use `_with_cancellation` methods to supply a cancellation token.

## Python

~~~python
client = session.fleet(selected_domain_id)
try:
    inventory = await client.list()
    pool = await client.compute_pool(
        mode="dedicated", capabilities=["vendor.example/inspect/v7"]
    )
    for machine in inventory["machines"]:
        print(machine["name"], machine["presence"], machine["work_state"])
    print(inventory["sources"], pool["complete"])
finally:
    await client.close()
~~~

Catch `auki_sdk.AukiFleetError` for its `kind`, `code` and `status`. asyncio task
cancellation reaches the native operation. App credentials belong on trusted
backends; their global work state remains unknown with the current provider.

## Web and Expo

~~~ts
// Web: const client = session.fleet(selectedDomainId);
// Expo: import { fleet } from "@aukilabs/auki-sdk-expo";
//       const client = await fleet(sessionId, selectedDomainId);
const controller = new AbortController();
try {
  const inventory = await client.list({}, controller.signal);
  const pool = await client.computePool({
    mode: "dedicated", capabilities: ["vendor.example/inspect/v7"],
    matchAllCapabilities: true,
  }, controller.signal);
  for (const machine of inventory.machines) {
    console.log(machine.name, machine.presence, machine.work_state);
  }
  console.log(inventory.sources, pool.complete);
} finally {
  await client.close();
}
~~~

Queries use camelCase; snapshot JSON uses snake_case. Results are typed in both
packages. An `AbortSignal` cancels one call; closing the client drains all its
calls. Web consumers can `free()` the WASM object after awaiting `close()`.

## Swift

~~~swift
let client = try session.fleet(domainId: selectedDomainId)
do {
    let inventory = try await client.list()
    let pool = try await client.computePool(.init(
        mode: .dedicated, capabilities: ["vendor.example/inspect/v7"]
    ))
    for machine in inventory.machines {
        print(machine.name, machine.presence, machine.workState)
    }
    print(inventory.sources, pool.complete)
    try await client.close()
} catch {
    try? await client.close()
    throw error
}
~~~

Pass `AukiCancellation` as `cancellation:` for explicit cancellation. Models are
Codable with typed status enums and optional provider timestamps. Fleet failures
use the new `AukiSdkError.Fleet` case.

Always display unknown status and source completeness. A candidate is not
assigned to this Domain; an idle observation is not a reservation. Missing
inventory must not erase visible task references in `unresolved_activity`.
See the [reference](../reference/fleet.md) for permission differences, limits,
provider versions, pagination defects and rollout requirements.
