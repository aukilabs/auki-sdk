# Auki SDK for Python

Use `auki_sdk` to connect peers, work with Domain data, and run compute or robot
task handlers. Requires Python 3.8+ and Rust 1.89+.

## Build

From the SDK repository root, create a virtual environment and build the binding:

~~~sh
python3 -m venv core/bindings/python/auki-sdk-py/.venv
. core/bindings/python/auki-sdk-py/.venv/bin/activate
python -m pip install 'maturin>=1.5,<2.0'
maturin develop --locked --no-default-features --manifest-path core/bindings/python/auki-sdk-py/Cargo.toml
~~~

This build includes networking, Domain data, and tasks. Omit
`--no-default-features` to include the default application protocols. Custom
Rust protocols must be compiled into the same extension; see
[custom protocols](../../../../docs/how-to/protocols.md).

## Use the binding

| Task | API | Guide or example |
| --- | --- | --- |
| Connect peers | `AukiSession`, `AukiPeer` | [Python Echo](../../../examples/portable-echo/python/README.md) |
| Read and write Domain data | `session.domains()`, `session.data(domain_id)` | [Domain data guide](../../../../docs/how-to/domain-data.md#use-web-or-python) |
| Inspect robots and compute activity | `session.fleet(domain_id)` | [Fleet guide](../../../../docs/how-to/inspect-fleet.md) |
| Submit and monitor DMS jobs | `session.jobs(domain_id)` | `estimate`, `submit`, `list`, `get`, `cancel` |
| Run compute or robot handlers | `AukiComputeCredential`, `AukiRobotCredential`, `AukiDmsTasks` | [Task guide](../../../../docs/how-to/run-compute-tasks.md) |

`AukiSession.login_dev` signs in to development services. Backend services can
use `login_app_dev`. Use `login_app_with_environment(api, dds, dms, access_key,
secret, client_id=installation_id)` for exact custom service URLs. App login
methods also accept `gateway_mac` when the App's DDS policy requires it; the
same policy is retained during renewal. Keep App secrets on trusted backends.
User and App login accept a persistent `client_id` for the installation.
Close data clients and peers before closing their session. For tasks, await
`tasks.close()` before closing the machine credential.

### Submit and monitor jobs

User, App, and imported ZITADEL sessions can create a Domain-scoped jobs client
without starting a peer. App credentials remain for trusted Python hosts. Job
specifications and responses use ordinary dictionaries; custom capability names
pass through unchanged, including dedicated work:

~~~python
jobs = session.jobs(selected_domain_id)
spec = {
    "label": "prepare-assets",
    "tasks": [{
        "label": "convert",
        "stage": "convert",
        "capability": "com.example.convert.v1",
        "mode": "dedicated",
        "capability_filters": {"format": "glb"},
    }],
}
try:
    estimate = await jobs.estimate(spec)
    job_id = await jobs.submit(spec)
    details = await jobs.get(job_id)
    page = await jobs.list(limit=50, capabilities=["com.example.convert.v1"])
finally:
    await jobs.close()
~~~

Python task dictionaries use the DMS snake-case names shown above. Omitted
`mode`, `priority`, collections, metadata, and `max_attempts` use SDK defaults.
Canceling the Python await cancels its HTTP operation. `AukiJobsError` exposes
`kind`, `code`, and optional `status`; `code == "persistence"` is recoverable on
the retained imported session. A `kind == "submission_uncertain"` error includes
the underlying `source` code and means the submit request may have succeeded, so
reconcile with `list` before retrying.
Always await `jobs.close()` before closing the shared session.

For a deployment verified to support the idempotency contract, use
`await jobs.submit_with_key(spec, idempotency_key)`. Persist the key and immutable
specification before the first send and reuse both for recovery; no automatic
retry is added. An in-progress submission exposes the retry delay through `retry_after_seconds`;
a plain HTTP 409 is a conflict. Older servers ignore keys, so verify the backend
rollout before enabling this method. Existing unkeyed `submit` behavior is unchanged.

See the [jobs reference](../../../../docs/reference/jobs.md) for required write
authority, worker availability and provider limitations.

### Import a ZITADEL session

Mobile and desktop hosts that own a ZITADEL PKCE login can import its complete
credential snapshot synchronously. Call the import method on the asyncio loop
that owns the storage callback. Import performs no network request.

~~~python
credentials = auki_sdk.ZitadelSessionCredentials(
    access_token,
    refresh_token,
    public_client_id,
    trusted_issuer,
    access_token_expires_at,  # RFC 3339, or None when unknown
)

async def save_replacement(replacement):
    # Atomically persist every field before returning. Token getters are explicit.
    await secure_store.replace({
        "access_token": replacement.expose_access_token(),
        "refresh_token": replacement.expose_refresh_token(),
        "client_id": replacement.client_id,
        "issuer": replacement.issuer,
        "access_token_expires_at": replacement.access_token_expires_at,
    })

session = auki_sdk.AukiSession.import_zitadel_dev(
    credentials, save_replacement
)
page = await session.domains().list(limit=50)
# Present page["domains"] to the user. The host must explicitly select an ID;
# continue with offset += len(page["domains"]) to show another server page.
selected_domain_id = await choose_domain_id(page["domains"])
data = session.data(selected_domain_id)
items = await data.list()
peer = await session.start_peer(selected_domain_id, identity_file)
~~~

The callback receives one immutable, redacted replacement object. The SDK waits
for the callback before using its new access token. If persistence fails, retain
the session and retry an operation; the SDK offers the same replacement again
without rotating twice. Cancelling an operation does not cancel a storage write
that has already started, and `await session.close()` waits for that write before
the host clears secure storage.

Imported owner and User sessions list Domains through the API's ordinary User
access profile; the SDK validates each DDS page against the token's organization
and any Domain allowlist. Viewer profiles use the API's narrower P2P listing
grant. The default `session.domains().list()` query is supported; organization
and Domain Server filters and portal association queries remain unsupported. A
403 listing denial is recoverable, and the same session may still access a known
Domain ID when the provider authorizes it. Use
`import_zitadel_with_environment` to supply exact API, DDS, and DMS base URLs.

The [compute](examples/compute_task.py) and [robot](examples/robot_task.py)
examples include shutdown handling. The [file example](examples/domain_data.py)
streams a 17 MiB file by default, verifies SHA-256, and deletes its unique dev
record. It requires Python 3.9+ and optionally accepts an input file path.

To run the file example, set `AUKI_EMAIL`, `AUKI_PASSWORD`, `AUKI_DOMAIN_ID`, and
`AUKI_CLIENT_ID` for a dev account and explicitly selected Domain approved for
data writes:

~~~sh
python core/bindings/python/auki-sdk-py/examples/domain_data.py
~~~

See the [data](../../../../docs/reference/domain-data.md) and
[task](../../../../docs/reference/tasks.md) references for limits, errors,
credential support, and cancellation behavior.

## Test locally

After building, run the data and task suites from the repository root.
These use local DDS, DMS, and data fixtures:

~~~sh
python -m pip install -r core/bindings/python/auki-sdk-py/python_tests/requirements.txt
python -m pytest core/bindings/python/auki-sdk-py/python_tests/test_domain_data.py core/bindings/python/auki-sdk-py/python_tests/test_jobs.py core/bindings/python/auki-sdk-py/python_tests/test_zitadel_session.py core/bindings/python/auki-sdk-py/python_tests/test_tasks.py core/bindings/python/auki-sdk-py/python_tests/test_robot_tasks.py -q
~~~

The process-exit regression uses fresh subprocesses, synthetic unregistered
credentials, and no services. It checks normal exit status after awaited close,
including cancellation and repeated close. Each child has a 10-second deadline;
a signal or timeout is a failure. Run the extended exit gate with:

~~~sh
AUKI_PROCESS_EXIT_ATTEMPTS=100 python -m pytest core/bindings/python/auki-sdk-py/python_tests/test_process_exit.py -q
~~~

The common async completion adapter joins the native bridge completion task in
an asyncio done callback before resuming awaiters. A ready Python Future alone
does not prove that native completion has released its Python references.
Individual closes leave the shared runtime available to other SDK objects.

## Fleet inventory and activity

```python
fleet = session.fleet(selected_domain_id)
try:
    inventory = await fleet.list()
    candidates = await fleet.compute_pool(mode="dedicated", capabilities=["vendor.example/inspect/v7"])
    print(inventory["machines"], inventory["sources"])
finally:
    await fleet.close()
```

Results are dictionaries with snake_case fields. Inspect `sources` and
`work_state`; unknown activity must remain unknown. `AukiFleetError` preserves
`kind`, `code` and `status`, including credential-persistence failures. App hosts
can inspect permitted inventory and jobs, but global work state remains unknown.
See the [fleet reference](../../../../docs/reference/fleet.md).
Local tests: `python -m pytest core/bindings/python/auki-sdk-py/python_tests/test_fleet.py -q`
after building the extension in your virtual environment.
