# Submit and monitor Domain jobs

Use `AukiDmsJobs` to estimate, submit, list, inspect, or cancel jobs through DMS.
No peer, discovery registration, or task runner starts in your app. DMS schedules
the work; [compute and robot handlers](run-compute-tasks.md) execute it.

## Use an existing session

Reuse the [authenticated bootstrap](authenticate.md). Its DMS URL must target
the same environment as its API and DDS endpoints, including the API path
(for example, `https://dms.dev.aukiverse.com/v1/`). Creating the client is local;
requests begin when you call an operation.

```rust,no_run
use auki_sdk::{AukiPeerBootstrap, JobMode, JobSpec, JobTaskSpec, JobsError};

async fn submit_report(
    bootstrap: &AukiPeerBootstrap,
    domain_id: uuid::Uuid,
) -> Result<uuid::Uuid, JobsError> {
    let jobs = bootstrap.jobs()?.in_domain(domain_id);
    let mut task = JobTaskSpec::new("report", "vendor/report/v1");
    task.mode = JobMode::Dedicated;
    task.meta = serde_json::json!({"format": "summary"});
    let spec = JobSpec::single("report-request-123", task);

    // Optional: DMS supplies the price; decimal amounts remain strings.
    let estimate = jobs.estimate(&spec).await?;
    println!("estimated credits: {}", estimate.total);

    let result = jobs.submit(&spec).await;
    jobs.close().await;
    result
}
```

This example submits real work and can consume credits. Run it only against an
approved environment and Domain with an eligible worker. A third-party capability
such as `vendor/report/v1` is passed through without an SDK catalogue. Unlisted
non-Auki capabilities in dedicated mode use the backend's default dedicated price.

User, trusted backend App, and imported ZITADEL sessions share the same client.
The SDK obtains a selected-Domain DDS grant; it never sends a raw ZITADEL token
to DMS. All job endpoints currently require Domain write authority, including
listing and inspection. Session refresh and imported-credential persistence remain
owned by the existing session. On a `persistence` failure, retain that session and
retry persistence as described in the authentication guide.

## Submit a graph

Put multiple `JobTaskSpec` entries in `spec.tasks` and connect their **stage names**
with `JobEdge { from, to }` entries in `spec.edges`. DMS validates the graph and
expands each stage edge to the tasks in those stages. The SDK does not schedule
dependencies or execute task handlers.

## Monitor and cancel

```rust,no_run
use auki_sdk::{DomainJobsClient, JobListQuery, JobStatus, JobsError};

async fn inspect(jobs: &DomainJobsClient, id: uuid::Uuid) -> Result<(), JobsError> {
    let details = jobs.get(id).await?;
    for task in details.tasks {
        println!("{}: {:?}, progress: {}", task.stage, task.status, task.meta["progress"]);
    }
    for receipt in details.receipts {
        println!("task {} outputs: {:?}", receipt.task_id, receipt.outputs);
    }

    let page = jobs.list(&JobListQuery {
        status: Some(JobStatus::Running),
        ..Default::default()
    }).await?;
    // Pass page.next_cursor unchanged to the next query. Do not assume that an
    // empty page is the last page when DMS supplies another cursor.
    let _next_cursor = page.next_cursor;

    jobs.cancel(id).await?;
    Ok(())
}
```

Task progress and recent events are worker-defined JSON in `task.meta.progress`
and `task.meta.events`. Results are receipts containing `outputs`, which are
opaque references. A task may have multiple receipts across retries or cancellation;
use task status and receipt metadata when choosing results. Read Domain data through
a separate [data client](domain-data.md) when those references identify stored data.

`cancel(id)` requests cancellation of the whole job. Running tasks may still be
stopping after DMS marks the job canceled. Inspect task states; cancellation is
not a guarantee that a physical robot has stopped.

## Handle failures and close

Only an explicit HTTP 401 gets one grant renewal and retry. Other HTTP responses,
timeouts, and transport failures are returned to the host. If submission fails with
`submission_uncertain`, DMS may already have created and charged the job. For the
unkeyed `submit` call, inspect jobs using an application label or metadata and
reconcile the outcome first. DMS does not enforce uniqueness for those values.

After operators verify the [keyed-submission rollout](../reference/jobs.md#recover-a-keyed-submission),
persist an opaque random key and the complete specification before sending:

```rust,ignore
// Load the same persisted operation on recovery; never generate a new key per retry.
let (key, spec) = operation_store.load_pending().await?;
let job_id = jobs.submit_with_key(&spec, &key).await?;
operation_store.save_job_id(&key, job_id).await?;
```

On `SubmissionInProgress`, wait at least `retry_after_seconds` before another
attempt. On an uncertain outcome, retain the same key/spec for explicit recovery.
Use bounded attempts and cancellation; the SDK starts no retry loop. A plain 409
means conflict and must not trigger a replacement operation. Older DMS ignores
keys, so the rollout gate must be checked before relying on this behavior.

`*_with_cancellation` methods accept a `CancellationToken`. Canceling an HTTP request
does not cancel a backend job. `jobs.close().await` aborts and drains this client's
requests; its clones close together. Separately created clients and peers continue
using their shared session. On logout, close clients and peers, await session close,
then erase persisted credentials.

See the [jobs reference](../reference/jobs.md) for limits, backend constraints, and
platform APIs.
