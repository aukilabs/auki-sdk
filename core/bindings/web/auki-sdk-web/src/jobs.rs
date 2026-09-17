//! DMS job bindings share the authenticated session and never start a peer.
use crate::AukiUserSession;
use auki_sdk::{
    DomainJobsClient, JobEdge, JobListQuery, JobMode, JobSpec, JobStatus, JobTaskSpec, JobsError,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(typescript_custom_section)]
const TYPES: &str = r#"
export type JobMode = "public" | "dedicated";
export type JobStatus = "pending" | "running" | "completed" | "failed" | "canceled";
export type JobTaskStatus = "queued" | "leased" | "running" | "completed" | "failed" | "canceled";
export interface JobEdge { from: string; to: string }
export interface JobTaskSpec { label: string; stage: string; capability: string; mode?: JobMode; capabilityFilters?: Record<string, string>; priority?: number; inputsCids?: string[]; outputsPrefix?: string; meta?: Record<string, unknown>; maxAttempts?: number }
export interface JobSpec { label: string; priority?: number; meta?: Record<string, unknown>; tasks: JobTaskSpec[]; edges?: JobEdge[] }
export interface JobListQuery { limit?: number; cursor?: string; status?: JobStatus; capabilities?: string[]; matchAllCapabilities?: boolean }
export interface JobEstimateTask { label: string; stage: string; capability: string; mode: JobMode; billing_units: string; estimated_credit_cost: string }
export interface JobEstimate { total: string; tasks: JobEstimateTask[] }
export interface JobRecord { id: string; label: string; domain_id: string; status: JobStatus; priority: number; created_at: string; updated_at: string; organization_id: string | null; meta: Record<string, unknown>; credit_lock_id: string | null; credit_lock_amount: string | null; credit_locked_at: string | null; credit_released_at: string | null }
export interface JobTaskSummary { queued: number; leased: number; running: number; completed: number; failed: number; canceled: number }
export interface JobListItem { job: JobRecord; tasks_summary: JobTaskSummary }
export interface JobPage { items: JobListItem[]; next_cursor: string | null }
export interface JobTask { id: string; job_id: string; label: string; stage: string; capability: string; capability_filters: Record<string, string>; status: JobTaskStatus; deps_remaining: number; priority: number; inputs_cids: string[]; outputs_prefix: string | null; organization_id: string | null; attempts: number; max_attempts: number; lease_expires_at: string | null; reserved_by: string | null; meta: Record<string, unknown>; cancel_requested_at: string | null; last_heartbeat_at: string | null; created_at: string; updated_at: string; mode: JobMode; billing_units: string; estimated_credit_cost: string | null; debited_amount: string | null; debited_at: string | null }
export interface JobReceipt { id: string; job_id: string; task_id: string; node_id: string | null; outputs: string[]; meta: Record<string, unknown>; created_at: string }
export interface JobDetails { job: JobRecord; tasks_summary: JobTaskSummary; tasks: JobTask[]; receipts: JobReceipt[] }
export interface JobCancellation { id: string; status: JobStatus; updated_at: string }
/** Submission failures with kind `submission_uncertain` must be reconciled before retrying. */
export interface AukiJobsError extends Error { kind: string; code: string; status?: number; source?: string }
"#;

fn error(error: JobsError) -> JsValue {
    let result = js_sys::Error::new(&error.to_string());
    let kind = match &error {
        JobsError::Auth(_) => "auth",
        JobsError::InvalidInput(_) => "input",
        JobsError::InvalidResponse(_) => "response",
        JobsError::HttpStatus { .. } => "http",
        JobsError::Transport => "transport",
        JobsError::TimedOut => "timeout",
        JobsError::Cancelled => "cancelled",
        JobsError::Closed => "closed",
        JobsError::TooLarge { .. } => "limit",
        JobsError::SubmissionUncertain { .. } => "submission_uncertain",
    };
    let _ = js_sys::Reflect::set(&result, &"kind".into(), &kind.into());
    let _ = js_sys::Reflect::set(&result, &"code".into(), &error.code().into());
    if let Some(status) = error.http_status() {
        let _ = js_sys::Reflect::set(&result, &"status".into(), &status.into());
    }
    if let JobsError::SubmissionUncertain { source } = &error {
        let _ = js_sys::Reflect::set(&result, &"source".into(), &source.code().into());
    }
    result.into()
}

fn encode(value: &impl Serialize) -> Result<JsValue, JsValue> {
    value
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .map_err(|_| error(JobsError::InvalidResponse("cannot convert job response")))
}

fn parse<T: for<'de> Deserialize<'de> + Default>(value: JsValue) -> Result<T, JsValue> {
    if value.is_null() || value.is_undefined() {
        return Ok(T::default());
    }
    serde_wasm_bindgen::from_value(value)
        .map_err(|_| error(JobsError::InvalidInput("invalid job options")))
}

fn id(value: &str) -> Result<Uuid, JsValue> {
    Uuid::parse_str(value).map_err(|_| error(JobsError::InvalidInput("expected UUID")))
}

pub(crate) struct Cancellation {
    pub(crate) token: CancellationToken,
    signal: Option<web_sys::AbortSignal>,
    listener: Closure<dyn FnMut()>,
}

impl Cancellation {
    pub(crate) fn new(signal: Option<web_sys::AbortSignal>) -> Result<Self, JsValue> {
        let token = CancellationToken::new();
        let notify = token.clone();
        let listener = Closure::new(move || notify.cancel());
        if let Some(signal) = &signal {
            signal.add_event_listener_with_callback("abort", listener.as_ref().unchecked_ref())?;
            if signal.aborted() {
                token.cancel();
            }
        }
        Ok(Self {
            token,
            signal,
            listener,
        })
    }
}

impl Drop for Cancellation {
    fn drop(&mut self) {
        if let Some(signal) = &self.signal {
            let _ = signal.remove_event_listener_with_callback(
                "abort",
                self.listener.as_ref().unchecked_ref(),
            );
        }
    }
}

fn object() -> JsValue {
    js_sys::Object::new().into()
}

fn attempts() -> u32 {
    3
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SpecInput {
    label: String,
    #[serde(default)]
    priority: u32,
    #[serde(default = "object")]
    #[serde(with = "serde_wasm_bindgen::preserve")]
    meta: JsValue,
    tasks: Vec<TaskInput>,
    #[serde(default)]
    edges: Vec<JobEdge>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TaskInput {
    label: String,
    stage: String,
    capability: String,
    #[serde(default)]
    mode: JobMode,
    #[serde(default)]
    capability_filters: BTreeMap<String, String>,
    #[serde(default)]
    priority: i32,
    #[serde(default)]
    inputs_cids: Vec<String>,
    #[serde(default)]
    outputs_prefix: Option<String>,
    #[serde(default = "object")]
    #[serde(with = "serde_wasm_bindgen::preserve")]
    meta: JsValue,
    #[serde(default = "attempts")]
    max_attempts: u32,
}

impl SpecInput {
    fn into_spec(self) -> Result<JobSpec, JsValue> {
        let spec = JobSpec {
            label: self.label,
            priority: self.priority,
            meta: serde_wasm_bindgen::from_value(self.meta)
                .map_err(|_| error(JobsError::InvalidInput("job meta must be a JSON object")))?,
            tasks: self
                .tasks
                .into_iter()
                .map(TaskInput::into_spec)
                .collect::<Result<_, _>>()?,
            edges: self.edges,
        };
        if !spec.meta.is_object() {
            return Err(error(JobsError::InvalidInput(
                "job meta must be a JSON object",
            )));
        }
        Ok(spec)
    }
}

impl TaskInput {
    fn into_spec(self) -> Result<JobTaskSpec, JsValue> {
        let spec = JobTaskSpec {
            label: self.label,
            stage: self.stage,
            capability: self.capability,
            mode: self.mode,
            capability_filters: self.capability_filters,
            priority: self.priority,
            inputs_cids: self.inputs_cids,
            outputs_prefix: self.outputs_prefix,
            meta: serde_wasm_bindgen::from_value(self.meta)
                .map_err(|_| error(JobsError::InvalidInput("task meta must be a JSON object")))?,
            max_attempts: self.max_attempts,
        };
        if !spec.meta.is_object() {
            return Err(error(JobsError::InvalidInput(
                "task meta must be a JSON object",
            )));
        }
        Ok(spec)
    }
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct QueryInput {
    limit: Option<u32>,
    cursor: Option<String>,
    status: Option<JobStatus>,
    capabilities: Vec<String>,
    match_all_capabilities: bool,
}

#[wasm_bindgen]
impl AukiUserSession {
    /// Select DMS job access for one Domain without starting a peer.
    pub fn jobs(&self, domain_id: String) -> Result<AukiDmsJobs, JsValue> {
        Ok(AukiDmsJobs {
            inner: self
                .bootstrap
                .jobs()
                .map_err(error)?
                .in_domain(id(&domain_id)?),
        })
    }
}

#[wasm_bindgen]
pub struct AukiDmsJobs {
    inner: DomainJobsClient,
}

#[wasm_bindgen]
impl AukiDmsJobs {
    #[wasm_bindgen(getter, js_name = domainId)]
    pub fn domain_id(&self) -> String {
        self.inner.domain_id().to_string()
    }

    pub async fn close(&self) {
        self.inner.close().await;
    }

    #[wasm_bindgen(unchecked_return_type = "JobEstimate")]
    pub async fn estimate(
        &self,
        #[wasm_bindgen(unchecked_param_type = "JobSpec")] spec: JsValue,
        signal: Option<web_sys::AbortSignal>,
    ) -> Result<JsValue, JsValue> {
        let spec = serde_wasm_bindgen::from_value::<SpecInput>(spec)
            .map_err(|_| error(JobsError::InvalidInput("invalid job specification")))?;
        let spec = spec.into_spec()?;
        let cancel = Cancellation::new(signal)?;
        encode(
            &self
                .inner
                .estimate_with_cancellation(&spec, &cancel.token)
                .await
                .map_err(error)?,
        )
    }

    #[wasm_bindgen(unchecked_return_type = "string")]
    pub async fn submit(
        &self,
        #[wasm_bindgen(unchecked_param_type = "JobSpec")] spec: JsValue,
        signal: Option<web_sys::AbortSignal>,
    ) -> Result<String, JsValue> {
        let spec = serde_wasm_bindgen::from_value::<SpecInput>(spec)
            .map_err(|_| error(JobsError::InvalidInput("invalid job specification")))?;
        let spec = spec.into_spec()?;
        let cancel = Cancellation::new(signal)?;
        Ok(self
            .inner
            .submit_with_cancellation(&spec, &cancel.token)
            .await
            .map_err(error)?
            .to_string())
    }

    #[wasm_bindgen(unchecked_return_type = "JobPage")]
    pub async fn list(
        &self,
        #[wasm_bindgen(unchecked_param_type = "JobListQuery | undefined")] query: JsValue,
        signal: Option<web_sys::AbortSignal>,
    ) -> Result<JsValue, JsValue> {
        let query: QueryInput = parse(query)?;
        let cancel = Cancellation::new(signal)?;
        encode(
            &self
                .inner
                .list_with_cancellation(
                    &JobListQuery {
                        limit: query.limit.unwrap_or(50),
                        cursor: query.cursor,
                        status: query.status,
                        capabilities: query.capabilities,
                        match_all_capabilities: query.match_all_capabilities,
                    },
                    &cancel.token,
                )
                .await
                .map_err(error)?,
        )
    }

    #[wasm_bindgen(unchecked_return_type = "JobDetails")]
    pub async fn get(
        &self,
        job_id: String,
        signal: Option<web_sys::AbortSignal>,
    ) -> Result<JsValue, JsValue> {
        let cancel = Cancellation::new(signal)?;
        encode(
            &self
                .inner
                .get_with_cancellation(id(&job_id)?, &cancel.token)
                .await
                .map_err(error)?,
        )
    }

    #[wasm_bindgen(unchecked_return_type = "JobCancellation")]
    pub async fn cancel(
        &self,
        job_id: String,
        signal: Option<web_sys::AbortSignal>,
    ) -> Result<JsValue, JsValue> {
        let cancel = Cancellation::new(signal)?;
        encode(
            &self
                .inner
                .cancel_with_cancellation(id(&job_id)?, &cancel.token)
                .await
                .map_err(error)?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_futures::JsFuture;
    use wasm_bindgen_test::wasm_bindgen_test;

    const DOMAIN: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    const JOB: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
    const CAPABILITY: &str = "com.example.third-party.convert.v1";

    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            js_sys::eval(
                "globalThis.fetch=globalThis.__originalJobsFetch; delete globalThis.__originalJobsFetch",
            )
            .unwrap();
        }
    }

    fn fixture() -> Restore {
        js_sys::eval(
            r#"
            globalThis.__originalJobsFetch = globalThis.fetch;
            globalThis.__jobsRequests = [];
            globalThis.__blockJobsList = false;
            globalThis.__denyJobs = false;
            globalThis.fetch = async (input, init) => {
                const request = input instanceof Request ? input : new Request(input, init);
                const url = new URL(request.url);
                const domain = 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa';
                const jobId = 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb';
                const taskId = 'cccccccc-cccc-4ccc-8ccc-cccccccccccc';
                const capability = 'com.example.third-party.convert.v1';
                const date = '2026-09-16T00:00:00Z';
                const json = (body, status=200) => {
                    const response = new Response(JSON.stringify(body), {status, headers:{'content-type':'application/json'}});
                    Object.defineProperty(response, 'url', {value:request.url}); return response;
                };
                if (url.pathname === '/user/login') return json({access_token:'user',refresh_token:'refresh'});
                if (url.pathname === '/service/domains-access-token') return json({access_token:'service'});
                if (url.pathname.endsWith('/auth')) {
                    const claims={iss:'dds',domain_id:domain,aud:['dds','https://server.example','https://dms.example'],exp:Math.floor(Date.now()/1000)+3600};
                    const token='e30.'+btoa(JSON.stringify(claims)).replaceAll('=','').replaceAll('+','-').replaceAll('/','_')+'.sig';
                    return json({id:domain,domain_server:{url:'https://server.example'},access_token:token});
                }
                if (request.redirect !== 'error' || request.credentials !== 'omit') throw Error('unsafe jobs request');
                if (!request.headers.get('authorization')?.startsWith('Bearer ') || request.headers.get('posemesh-client-id') !== 'web-jobs-fixture') throw Error('missing jobs authentication');
                if (__denyJobs) return json({}, 403);
                if (url.pathname === '/jobs/estimate') {
                    const body=await request.json(); __jobsRequests.push(body);
                    const task=body.tasks[0];
                    return json({total:'2.50',tasks:[{label:task.label,stage:task.stage,capability:task.capability,mode:task.mode,billing_units:'1.0',estimated_credit_cost:'2.50'}]});
                }
                if (url.pathname === '/jobs' && request.method === 'POST') {
                    const body=await request.json(); __jobsRequests.push(body); return json({job_id:jobId});
                }
                const job={id:jobId,label:'prepare-assets',domain_id:domain,status:'running',priority:0,created_at:date,updated_at:date,organization_id:null,meta:{},credit_lock_id:null,credit_lock_amount:'2.50',credit_locked_at:date,credit_released_at:null};
                const summary={queued:0,leased:0,running:1,completed:0,failed:0,canceled:0};
                if (url.pathname === '/jobs' && request.method === 'GET') {
                    if (__blockJobsList) return new Promise((_,reject) => request.signal.addEventListener('abort',()=>reject(new DOMException('aborted','AbortError'))));
                    if (url.searchParams.get('capabilities') !== capability || url.searchParams.get('match_all_capabilities') !== 'true') throw Error('wrong list filters');
                    return json({items:[{job,tasks_summary:summary}],next_cursor:'next'});
                }
                if (url.pathname === '/jobs/'+jobId && request.method === 'GET') return json({job,tasks_summary:summary,tasks:[{id:taskId,job_id:jobId,label:'convert',stage:'convert',capability,capability_filters:{format:'glb'},status:'running',deps_remaining:0,priority:0,inputs_cids:['bafy-input'],outputs_prefix:'converted/',organization_id:null,attempts:1,max_attempts:3,lease_expires_at:date,reserved_by:null,meta:{progress:0.5},cancel_requested_at:null,last_heartbeat_at:date,created_at:date,updated_at:date,mode:'dedicated',billing_units:'1.0',estimated_credit_cost:'2.50',debited_amount:null,debited_at:null}],receipts:[]});
                if (url.pathname === '/jobs/'+jobId+'/cancel') return json({id:jobId,status:'canceled',updated_at:date});
                return json({}, 404);
            };
            "#,
        )
        .unwrap();
        Restore
    }

    async fn login() -> AukiUserSession {
        AukiUserSession::login_with_environment_and_client_id(
            "https://api.example".into(),
            "https://dds.example".into(),
            "https://dms.example".into(),
            "fixture@example.com".into(),
            "fixture".into(),
            Some("web-jobs-fixture".into()),
        )
        .await
        .unwrap()
    }

    fn spec() -> JsValue {
        js_sys::eval(
            r#"({label:'prepare-assets',tasks:[{label:'convert',stage:'convert',
            capability:'com.example.third-party.convert.v1',mode:'dedicated',
            capabilityFilters:{format:'glb'},inputsCids:['bafy-input'],
            outputsPrefix:'converted/'}]})"#,
        )
        .unwrap()
    }

    #[wasm_bindgen_test]
    async fn custom_dedicated_job_round_trip_uses_camel_case_inputs() {
        let _restore = fixture();
        let session = login().await;
        let jobs = session.jobs(DOMAIN.into()).unwrap();
        let estimate = jobs.estimate(spec(), None).await.unwrap();
        assert_eq!(
            js_sys::Reflect::get(&estimate, &"total".into()).unwrap(),
            "2.50"
        );
        assert_eq!(jobs.submit(spec(), None).await.unwrap(), JOB);
        let page = jobs
            .list(
                js_sys::eval(&format!(
                    "({{capabilities:['{CAPABILITY}'],matchAllCapabilities:true}})"
                ))
                .unwrap(),
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            js_sys::Reflect::get(&page, &"next_cursor".into()).unwrap(),
            "next"
        );
        let details = jobs.get(JOB.into(), None).await.unwrap();
        assert!(js_sys::Reflect::has(&details, &"tasks_summary".into()).unwrap());
        let canceled = jobs.cancel(JOB.into(), None).await.unwrap();
        assert_eq!(
            js_sys::Reflect::get(&canceled, &"status".into()).unwrap(),
            "canceled"
        );
        let request = js_sys::eval("globalThis.__jobsRequests[0]").unwrap();
        assert_eq!(
            js_sys::Reflect::get(&request, &"domain_id".into()).unwrap(),
            DOMAIN
        );
        let task = js_sys::eval("globalThis.__jobsRequests[0].tasks[0]").unwrap();
        assert!(
            js_sys::Reflect::get(&task, &"capability_filters".into())
                .unwrap()
                .is_object()
        );
        jobs.close().await;
        let closed = jobs.get(JOB.into(), None).await.unwrap_err();
        assert_eq!(
            js_sys::Reflect::get(&closed, &"code".into()).unwrap(),
            "closed"
        );
        JsFuture::from(session.close()).await.unwrap();
    }

    #[wasm_bindgen_test]
    async fn abort_and_http_errors_are_structured() {
        let _restore = fixture();
        let session = login().await;
        let jobs = session.jobs(DOMAIN.into()).unwrap();
        js_sys::eval("globalThis.__blockJobsList=true").unwrap();
        let controller = web_sys::AbortController::new().unwrap();
        js_sys::Reflect::set(
            &js_sys::global(),
            &"__jobsAbort".into(),
            controller.as_ref(),
        )
        .unwrap();
        js_sys::eval("setTimeout(()=>globalThis.__jobsAbort.abort(),0)").unwrap();
        let cancelled = jobs
            .list(JsValue::UNDEFINED, Some(controller.signal()))
            .await
            .unwrap_err();
        assert_eq!(
            js_sys::Reflect::get(&cancelled, &"kind".into()).unwrap(),
            "cancelled"
        );
        js_sys::eval("globalThis.__blockJobsList=false; globalThis.__denyJobs=true").unwrap();
        let denied = jobs.get(JOB.into(), None).await.unwrap_err();
        assert_eq!(
            js_sys::Reflect::get(&denied, &"status".into())
                .unwrap()
                .as_f64(),
            Some(403.0)
        );
        assert_eq!(
            js_sys::Reflect::get(&denied, &"code".into()).unwrap(),
            "http_status"
        );
        jobs.close().await;
        JsFuture::from(session.close()).await.unwrap();
    }
}
