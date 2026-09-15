//! Domain HTTP bindings share the existing session and never start a peer.
use crate::AukiUserSession;
use auki_sdk::{
    AukiDomainData as DataFactory, AukiDomains as Domains, DataError, DataListQuery, DataWrite,
    DomainDataClient, DomainListQuery, PortalId, TransferOptions,
};
use js_sys::{Function, Promise, Uint8Array};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

#[wasm_bindgen(typescript_custom_section)]
const TYPES: &str = r#"
export interface DomainQuery { organization?: string; domainServerId?: string; limit?: number; offset?: number }
export interface DomainSummary { id: string; name: string; organization_id: string | null }
export interface DomainPage { domains: DomainSummary[]; total: number; limit: number; offset: number }
export interface PortalDomain extends DomainSummary { is_default: boolean; added_to_domain_at: string }
export interface Portal { id: string; short_id: string; name: string; size: number; organization_id: string | null; default_domain_id: string | null; redirect_url: string | null; created_at: string; updated_at: string }
export interface PortalPose { id: string; short_id: string; domain_id: string; reported_size: number; px: number; py: number; pz: number; rx: number; ry: number; rz: number; rw: number; latitude: number | null; longitude: number | null; altitude: number | null; vertical_accuracy: number | null; horizontal_accuracy: number | null; gps_timestamp: number | null; scanner_device_id: string; scanner_device_name: string; scanner_device_model: string; placed_at: string }
export interface DataMetadata { id: string; domain_id: string; name: string; data_type: string; size: number; created_at: string; updated_at: string }
export interface DataQuery { ids?: string[]; name?: string; dataType?: string }
export type DataTarget = { name: string; dataType: string; id?: never } | { id: string; name?: never; dataType?: never };
export interface TransferOptions { maxBytes?: number; maxChunkBytes?: number }
export type DataSource = (maximumBytes: number, signal: AbortSignal) => Uint8Array | Promise<Uint8Array>;
export type DataSink = (chunk: Uint8Array, signal: AbortSignal) => void | Promise<void>;
/** Auth failures also carry a code; retry a persistence failure on the retained session. */
export interface DomainDataError extends Error { kind: string; status?: number; code?: AukiAuthFailureCode }
"#;

pub(crate) fn error(error: DataError) -> JsValue {
    let result = js_sys::Error::new(&error.to_string());
    let kind = match &error {
        DataError::Cancelled => "cancelled",
        DataError::Closed => "closed",
        DataError::TimedOut => "timeout",
        DataError::Auth(_) => "auth",
        DataError::HttpStatus { .. } => "http",
        DataError::InvalidInput(_) => "input",
        DataError::InvalidResponse(_) => "response",
        DataError::TooLarge { .. } => "limit",
        DataError::Callback => "callback",
        DataError::Cleanup { .. } => "cleanup",
        DataError::Transport => "transport",
    };
    let _ = js_sys::Reflect::set(&result, &"kind".into(), &kind.into());
    if let Some(status) = error.status() {
        let _ = js_sys::Reflect::set(&result, &"status".into(), &status.into());
    }
    let mut original = &error;
    while let DataError::Cleanup { operation, .. } = original {
        original = operation;
    }
    if let DataError::Auth(auth) = original {
        let auth = crate::zitadel::auth_error(auth.kind());
        if let Ok(code) = js_sys::Reflect::get(&auth, &"code".into()) {
            let _ = js_sys::Reflect::set(&result, &"code".into(), &code);
        }
    }
    result.into()
}
fn encode(value: &impl Serialize) -> Result<JsValue, JsValue> {
    value
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .map_err(|_| error(DataError::InvalidResponse("cannot convert metadata")))
}
fn parse<T: for<'de> Deserialize<'de> + Default>(value: JsValue) -> Result<T, JsValue> {
    if value.is_null() || value.is_undefined() {
        return Ok(T::default());
    }
    serde_wasm_bindgen::from_value(value)
        .map_err(|_| error(DataError::InvalidInput("invalid options")))
}
fn id(value: &str) -> Result<Uuid, JsValue> {
    Uuid::parse_str(value).map_err(|_| error(DataError::InvalidInput("expected UUID")))
}
// A source/sink can stop its own pending work when the SDK drops its callback.
struct CallbackSignal {
    controller: web_sys::AbortController,
    completed: bool,
}
impl Drop for CallbackSignal {
    fn drop(&mut self) {
        if !self.completed {
            self.controller.abort();
        }
    }
}
fn invoke_callback(
    function: &Function,
    argument: &JsValue,
) -> Result<(JsValue, CallbackSignal), DataError> {
    let signal = CallbackSignal {
        controller: web_sys::AbortController::new().map_err(|_| DataError::Callback)?,
        completed: false,
    };
    let value = function
        .call2(&JsValue::UNDEFINED, argument, &signal.controller.signal())
        .map_err(|_| DataError::Callback)?;
    Ok((value, signal))
}

struct Cancellation {
    token: CancellationToken,
    signal: Option<web_sys::AbortSignal>,
    listener: Closure<dyn FnMut()>,
}
impl Cancellation {
    fn new(signal: Option<web_sys::AbortSignal>) -> Result<Self, JsValue> {
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
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct Query {
    organization: Option<String>,
    domain_server_id: Option<Uuid>,
    limit: Option<u32>,
    offset: Option<u32>,
}
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct DataQuery {
    ids: Vec<Uuid>,
    name: Option<String>,
    data_type: Option<String>,
}
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct Options {
    max_bytes: Option<u64>,
    max_chunk_bytes: Option<usize>,
}
impl From<Options> for TransferOptions {
    fn from(value: Options) -> Self {
        let defaults = Self::default();
        Self {
            max_bytes: value.max_bytes.unwrap_or(defaults.max_bytes),
            max_chunk_bytes: value.max_chunk_bytes.unwrap_or(defaults.max_chunk_bytes),
        }
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Target {
    id: Option<Uuid>,
    name: Option<String>,
    data_type: Option<String>,
}
impl Target {
    fn parse(value: JsValue) -> Result<Self, JsValue> {
        serde_wasm_bindgen::from_value(value)
            .map_err(|_| error(DataError::InvalidInput("invalid write target")))
    }
    fn as_write(&self) -> Result<DataWrite<'_>, JsValue> {
        match (&self.id, &self.name, &self.data_type) {
            (Some(id), None, None) => Ok(DataWrite::ById(*id)),
            (None, Some(name), Some(data_type)) => Ok(DataWrite::Named { name, data_type }),
            _ => Err(error(DataError::InvalidInput(
                "provide id or both name and dataType",
            ))),
        }
    }
}

#[wasm_bindgen]
impl AukiUserSession {
    pub fn domains(&self) -> AukiDomains {
        AukiDomains {
            inner: Domains::new(self.bootstrap.session().clone()),
        }
    }
    /// Select data access without starting networking.
    pub fn data(&self, domain_id: String) -> Result<AukiDomainData, JsValue> {
        Ok(AukiDomainData {
            inner: DataFactory::new(self.bootstrap.session().clone())
                .map_err(error)?
                .in_domain(id(&domain_id)?),
        })
    }
}
#[wasm_bindgen]
pub struct AukiDomains {
    inner: Domains,
}
#[wasm_bindgen]
impl AukiDomains {
    #[wasm_bindgen(unchecked_return_type = "DomainPage")]
    pub async fn list(
        &self,
        #[wasm_bindgen(unchecked_param_type = "DomainQuery | undefined")] query: JsValue,
        signal: Option<web_sys::AbortSignal>,
    ) -> Result<JsValue, JsValue> {
        let query: Query = parse(query)?;
        let cancel = Cancellation::new(signal)?;
        encode(
            &self
                .inner
                .list_with_cancellation(
                    &DomainListQuery {
                        organization: query.organization.unwrap_or_else(|| "own".into()),
                        domain_server_id: query.domain_server_id,
                        limit: query.limit.unwrap_or(50),
                        offset: query.offset.unwrap_or(0),
                    },
                    &cancel.token,
                )
                .await
                .map_err(error)?,
        )
    }
    #[wasm_bindgen(js_name = forPortal, unchecked_return_type = "PortalDomain[]")]
    pub async fn for_portal(
        &self,
        portal: String,
        organization: Option<String>,
        signal: Option<web_sys::AbortSignal>,
    ) -> Result<JsValue, JsValue> {
        let cancel = Cancellation::new(signal)?;
        encode(
            &self
                .inner
                .for_portal(
                    &PortalId::parse(&portal).map_err(|e| error(e.into()))?,
                    organization.as_deref().unwrap_or("own"),
                    &cancel.token,
                )
                .await
                .map_err(error)?,
        )
    }
    #[wasm_bindgen(unchecked_return_type = "Portal[]")]
    pub async fn portals(
        &self,
        domain: String,
        signal: Option<web_sys::AbortSignal>,
    ) -> Result<JsValue, JsValue> {
        let cancel = Cancellation::new(signal)?;
        encode(
            &self
                .inner
                .portals(id(&domain)?, &cancel.token)
                .await
                .map_err(error)?,
        )
    }
    #[wasm_bindgen(unchecked_return_type = "Portal")]
    pub async fn portal(
        &self,
        domain: String,
        portal: String,
        signal: Option<web_sys::AbortSignal>,
    ) -> Result<JsValue, JsValue> {
        let cancel = Cancellation::new(signal)?;
        encode(
            &self
                .inner
                .portal(
                    id(&domain)?,
                    &PortalId::parse(&portal).map_err(|e| error(e.into()))?,
                    &cancel.token,
                )
                .await
                .map_err(error)?,
        )
    }
}
#[wasm_bindgen]
pub struct AukiDomainData {
    inner: DomainDataClient,
}
#[wasm_bindgen]
impl AukiDomainData {
    pub async fn close(&self) {
        self.inner.close().await;
    }
    #[wasm_bindgen(unchecked_return_type = "DataMetadata[]")]
    pub async fn list(
        &self,
        #[wasm_bindgen(unchecked_param_type = "DataQuery | undefined")] query: JsValue,
        signal: Option<web_sys::AbortSignal>,
    ) -> Result<JsValue, JsValue> {
        let query: DataQuery = parse(query)?;
        let cancel = Cancellation::new(signal)?;
        encode(
            &self
                .inner
                .list_with_cancellation(
                    &DataListQuery {
                        ids: query.ids,
                        name: query.name,
                        data_type: query.data_type,
                    },
                    &cancel.token,
                )
                .await
                .map_err(error)?,
        )
    }
    #[wasm_bindgen(unchecked_return_type = "DataMetadata")]
    pub async fn get(
        &self,
        data_id: String,
        signal: Option<web_sys::AbortSignal>,
    ) -> Result<JsValue, JsValue> {
        let cancel = Cancellation::new(signal)?;
        encode(
            &self
                .inner
                .get_with_cancellation(id(&data_id)?, &cancel.token)
                .await
                .map_err(error)?,
        )
    }
    pub async fn read(
        &self,
        data_id: String,
        signal: Option<web_sys::AbortSignal>,
    ) -> Result<Uint8Array, JsValue> {
        let cancel = Cancellation::new(signal)?;
        Ok(Uint8Array::from(
            self.inner
                .read_with_cancellation(id(&data_id)?, &cancel.token)
                .await
                .map_err(error)?
                .as_slice(),
        ))
    }
    #[wasm_bindgen(unchecked_return_type = "DataMetadata")]
    pub async fn write(
        &self,
        #[wasm_bindgen(unchecked_param_type = "DataTarget")] target: JsValue,
        bytes: Uint8Array,
        signal: Option<web_sys::AbortSignal>,
    ) -> Result<JsValue, JsValue> {
        if bytes.length() > 8 * 1024 * 1024 {
            return Err(error(DataError::TooLarge {
                maximum: 8 * 1024 * 1024,
            }));
        }
        let target = Target::parse(target)?;
        let cancel = Cancellation::new(signal)?;
        encode(
            &self
                .inner
                .write_with_cancellation(target.as_write()?, &bytes.to_vec(), &cancel.token)
                .await
                .map_err(error)?,
        )
    }
    pub async fn delete(
        &self,
        data_id: String,
        signal: Option<web_sys::AbortSignal>,
    ) -> Result<(), JsValue> {
        let cancel = Cancellation::new(signal)?;
        self.inner
            .delete_with_cancellation(id(&data_id)?, &cancel.token)
            .await
            .map_err(error)
    }
    #[wasm_bindgen(unchecked_return_type = "PortalPose[]")]
    pub async fn poses(&self, signal: Option<web_sys::AbortSignal>) -> Result<JsValue, JsValue> {
        let cancel = Cancellation::new(signal)?;
        encode(&self.inner.poses(&cancel.token).await.map_err(error)?)
    }
    #[wasm_bindgen(unchecked_return_type = "PortalPose")]
    pub async fn pose(
        &self,
        portal: String,
        signal: Option<web_sys::AbortSignal>,
    ) -> Result<JsValue, JsValue> {
        let cancel = Cancellation::new(signal)?;
        encode(
            &self
                .inner
                .pose(
                    &PortalId::parse(&portal).map_err(|e| error(e.into()))?,
                    &cancel.token,
                )
                .await
                .map_err(error)?,
        )
    }
    #[wasm_bindgen(js_name = readTo)]
    pub async fn read_to(
        &self,
        data_id: String,
        #[wasm_bindgen(unchecked_param_type = "DataSink")] sink: Function,
        #[wasm_bindgen(unchecked_param_type = "TransferOptions | undefined")] options: JsValue,
        signal: Option<web_sys::AbortSignal>,
    ) -> Result<f64, JsValue> {
        let cancel = Cancellation::new(signal)?;
        self.inner
            .read_to(
                id(&data_id)?,
                parse::<Options>(options)?.into(),
                &cancel.token,
                |bytes| {
                    let result = invoke_callback(&sink, &Uint8Array::from(bytes.as_slice()));
                    async move {
                        let (value, mut signal) = result?;
                        JsFuture::from(Promise::resolve(&value))
                            .await
                            .map_err(|_| DataError::Callback)?;
                        signal.completed = true;
                        Ok(())
                    }
                },
            )
            .await
            .map(|n| n as f64)
            .map_err(error)
    }
    /// Named multipart uploads can replace existing names. Use a unique name or ID.
    #[wasm_bindgen(js_name = writeStream, unchecked_return_type = "DataMetadata")]
    pub async fn write_stream(
        &self,
        #[wasm_bindgen(unchecked_param_type = "DataTarget")] target: JsValue,
        size: f64,
        #[wasm_bindgen(unchecked_param_type = "DataSource")] source: Function,
        #[wasm_bindgen(unchecked_param_type = "TransferOptions | undefined")] options: JsValue,
        signal: Option<web_sys::AbortSignal>,
    ) -> Result<JsValue, JsValue> {
        if !size.is_finite()
            || size.fract() != 0.0
            || !(1.0..=9_007_199_254_740_991.0).contains(&size)
        {
            return Err(error(DataError::InvalidInput(
                "size must be a positive safe integer",
            )));
        }
        let target = Target::parse(target)?;
        let cancel = Cancellation::new(signal)?;
        encode(
            &self
                .inner
                .write_stream(
                    target.as_write()?,
                    size as u64,
                    parse::<Options>(options)?.into(),
                    &cancel.token,
                    |maximum| {
                        let result = invoke_callback(&source, &(maximum as u32).into());
                        async move {
                            let (value, mut signal) = result?;
                            let bytes = JsFuture::from(Promise::resolve(&value))
                                .await
                                .map_err(|_| DataError::Callback)?
                                .dyn_into::<Uint8Array>()
                                .map_err(|_| DataError::Callback)?;
                            if bytes.length() as usize > maximum {
                                return Err(DataError::InvalidInput(
                                    "source exceeded requested chunk size",
                                ));
                            }
                            signal.completed = true;
                            Ok(bytes.to_vec())
                        }
                    },
                )
                .await
                .map_err(error)?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test;
    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            js_sys::eval("globalThis.fetch = globalThis.__originalDataFetch; delete globalThis.__originalDataFetch").unwrap();
        }
    }
    const DOMAIN: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    const DATA: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
    fn fixture() -> Restore {
        js_sys::eval(r#"
            globalThis.__originalDataFetch = globalThis.fetch;
            globalThis.__dataCalls = [];
            globalThis.fetch = async (input, init) => {
                const request = input instanceof Request ? input : new Request(input, init);
                const url = new URL(request.url); __dataCalls.push(url.pathname);
                const domain = 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa';
                const data = 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb';
                const json = body => { const response = new Response(JSON.stringify(body), {headers:{'content-type':'application/json'}}); Object.defineProperty(response, 'url', {value:request.url}); return response; };
                const meta = {id:data,domain_id:domain,name:'report',data_type:'report.v1',size:7,created_at:'2026-09-01T00:00:00Z',updated_at:'2026-09-01T00:00:00Z'};
                if(url.pathname === '/user/login') return json({access_token:'user',refresh_token:'refresh'});
                if(url.pathname === '/service/domains-access-token') return json({access_token:'service'});
                if(url.pathname === '/api/v1/domains') {
                    if(url.searchParams.get('issue_token') !== 'false' || request.headers.get('posemesh-client-id') !== 'web-fixture') throw Error('invalid discovery headers');
                    return json({domains:[{id:domain,name:'Domain',organization_id:null}],total:1,limit:50,offset:0});
                }
                if(url.pathname.endsWith('/auth')) {
                    const claims={iss:'dds',domain_id:domain,aud:['dds','https://server.example'],exp:Math.floor(Date.now()/1000)+3600};
                    const token='e30.'+btoa(JSON.stringify(claims)).replaceAll('=','').replaceAll('+','-').replaceAll('/','_')+'.sig';
                    return json({id:domain,domain_server:{url:'https://server.example'},access_token:token});
                }
                if(request.redirect !== 'error' || request.credentials !== 'omit') throw Error('unsafe data request');
                if(url.pathname === '/api/v1/info') return json({upload:{domain_data_max_bytes:10000,request_max_bytes:4096,multipart:{enabled:true,part_size_bytes:4}}});
                if(url.pathname.endsWith('/multipart')) {
                    if(url.searchParams.has('uploads')) return json({upload_id:'cccccccc-cccc-4ccc-8ccc-cccccccccccc',data_id:data,part_size:4,expires_at:new Date(Date.now()+3600000).toISOString()});
                    if(request.method === 'PUT') return json({etag:'part'+url.searchParams.get('partNumber')});
                    if(request.method === 'DELETE') { globalThis.__dataAborted=true; return new Response(); }
                    return json(meta);
                }
                if(url.searchParams.get('raw') === 'true') return new Response(new Uint8Array([1,2,3,4,5,6,7]));
                return json(meta);
            };
        "#).unwrap();
        Restore
    }
    async fn login() -> AukiUserSession {
        AukiUserSession::login_with_environment_and_client_id(
            "https://api.example".into(),
            "https://dds.example".into(),
            "https://dms.example".into(),
            "fixture@example.com".into(),
            "fixture".into(),
            Some("web-fixture".into()),
        )
        .await
        .unwrap()
    }
    #[wasm_bindgen_test]
    async fn shared_session_exposes_metadata_and_bounded_streams_without_a_peer() {
        let _restore = fixture();
        let session = login().await;
        let page = session
            .domains()
            .list(JsValue::UNDEFINED, None)
            .await
            .unwrap();
        assert_eq!(
            js_sys::Reflect::get(&page, &"total".into())
                .unwrap()
                .as_f64(),
            Some(1.0)
        );
        let data = session.data(DOMAIN.into()).unwrap();
        let sink = Function::new_with_args(
            "bytes",
            "globalThis.__downloaded = (globalThis.__downloaded || 0) + bytes.length",
        );
        assert_eq!(
            data.read_to(
                DATA.into(),
                sink,
                js_sys::eval("({maxChunkBytes:2})").unwrap(),
                None
            )
            .await
            .unwrap(),
            7.0
        );
        js_sys::eval("globalThis.__remaining = [1,2,3,4,5,6,7]").unwrap();
        let source = Function::new_with_args(
            "maximum,signal",
            "globalThis.__completedSignal=signal; return Promise.resolve(new Uint8Array(globalThis.__remaining.splice(0, maximum)))",
        );
        data.write_stream(
            js_sys::eval(&format!("({{id:'{DATA}'}})")).unwrap(),
            7.0,
            source,
            JsValue::UNDEFINED,
            None,
        )
        .await
        .unwrap();
        data.close().await;
        assert_eq!(
            js_sys::eval("globalThis.__completedSignal.aborted")
                .unwrap()
                .as_bool(),
            Some(false)
        );
        // A second client retains the same session.
        assert_eq!(
            session
                .data(DOMAIN.into())
                .unwrap()
                .read(DATA.into(), None)
                .await
                .unwrap()
                .length(),
            7
        );
        JsFuture::from(session.close()).await.unwrap();
    }
    #[wasm_bindgen_test]
    async fn abort_signal_cleans_up_multipart_and_rejects_with_structured_error() {
        let _restore = fixture();
        let session = login().await;
        let data = session.data(DOMAIN.into()).unwrap();
        let controller = web_sys::AbortController::new().unwrap();
        js_sys::Reflect::set(
            &js_sys::global(),
            &"__cancelData".into(),
            controller.as_ref(),
        )
        .unwrap();
        let source = Function::new_with_args(
            "maximum,signal",
            "globalThis.__pendingSourceSignal=signal; globalThis.__cancelData.abort(); return new Promise(() => {})",
        );
        let error = data
            .write_stream(
                js_sys::eval(&format!("({{id:'{DATA}'}})")).unwrap(),
                7.0,
                source,
                JsValue::UNDEFINED,
                Some(controller.signal()),
            )
            .await
            .unwrap_err();
        assert_eq!(
            js_sys::Reflect::get(&error, &"kind".into())
                .unwrap()
                .as_string()
                .as_deref(),
            Some("cancelled")
        );
        assert_eq!(
            js_sys::eval("globalThis.__pendingSourceSignal.aborted")
                .unwrap()
                .as_bool(),
            Some(true)
        );
        assert_eq!(
            js_sys::eval("globalThis.__dataAborted").unwrap().as_bool(),
            Some(true)
        );
        data.close().await;
        JsFuture::from(session.close()).await.unwrap();
    }

    #[wasm_bindgen_test]
    fn multipart_cleanup_preserves_authentication_failure_code() {
        let failure = error(DataError::Cleanup {
            operation: Box::new(DataError::Auth(auki_sdk::AuthError::Persistence)),
            cleanup: Box::new(DataError::HttpStatus { status: 503 }),
        });
        assert_eq!(
            js_sys::Reflect::get(&failure, &"kind".into()).unwrap(),
            "cleanup"
        );
        assert_eq!(
            js_sys::Reflect::get(&failure, &"code".into()).unwrap(),
            "persistence"
        );
    }

    fn imported_fixture(store: &str) -> (Restore, AukiUserSession) {
        let restore = fixture();
        js_sys::eval(r#"
            globalThis.__importedRefreshes = 0;
            globalThis.__importedSaves = 0;
            globalThis.__importedSaved = false;
            globalThis.__importedSnapshots = [];
            const dataFetch = globalThis.fetch;
            globalThis.fetch = async (input, init) => {
                const request = input instanceof Request ? input : new Request(input, init);
                const url = new URL(request.url);
                const json = body => {
                    const response = new Response(JSON.stringify(body), {headers:{'content-type':'application/json'}});
                    Object.defineProperty(response, 'url', {value:request.url});
                    return response;
                };
                if (url.pathname === '/.well-known/openid-configuration') return json({
                    issuer:'https://issuer.example', token_endpoint:'https://issuer.example/oauth/v2/token',
                });
                if (url.pathname === '/oauth/v2/token') {
                    __importedRefreshes++;
                    return json({access_token:'rotated-access',refresh_token:'rotated-refresh',expires_in:3600,token_type:'Bearer'});
                }
                if (url.pathname === '/service/domains-access-token') {
                    if (url.search || request.headers.get('authorization') !== 'Bearer rotated-access' || !__importedSaved)
                        throw Error('exchange before persisted rotation or wrong credential profile');
                }
                if (url.pathname.endsWith('/auth') && request.headers.get('authorization') !== 'Bearer service')
                    throw Error('wrong data service bearer');
                if (url.searchParams.get('raw') === 'true' && !request.headers.get('authorization').endsWith('.sig'))
                    throw Error('wrong Domain grant');
                return dataFetch(request);
            };
        "#).unwrap();
        let credentials = js_sys::eval(r#"({accessToken:'old-access',refreshToken:'old-refresh',
            clientId:'browser-client',issuer:'https://issuer.example',accessTokenExpiresAt:'2000-01-01T00:00:00Z'})"#).unwrap();
        let session = AukiUserSession::import_zitadel_with_environment(
            "https://api.example".into(),
            "https://dds.example".into(),
            "https://dms.example".into(),
            credentials.unchecked_into(),
            Function::new_with_args("credentials", store).unchecked_into(),
        )
        .unwrap();
        (restore, session)
    }

    #[wasm_bindgen_test]
    async fn imported_data_retains_rotation_and_reports_persistence_for_retry() {
        let (_restore, session) = imported_fixture(
            r#"
            __importedSnapshots.push([credentials.exposeAccessToken(), credentials.exposeRefreshToken(),
                credentials.clientId, credentials.issuer, credentials.accessTokenExpiresAt]);
            if (++__importedSaves === 1) return Promise.reject(Error('private storage detail'));
            return Promise.resolve().then(() => { __importedSaved = true; });
        "#,
        );
        let data = session.data(DOMAIN.into()).unwrap();
        let error = data.read(DATA.into(), None).await.unwrap_err();
        assert_eq!(
            js_sys::Reflect::get(&error, &"code".into()).unwrap(),
            "persistence"
        );
        assert!(
            !js_sys::Error::from(error)
                .message()
                .includes("private storage", 0)
        );
        assert_eq!(data.read(DATA.into(), None).await.unwrap().length(), 7);
        assert_eq!(js_sys::eval("__importedRefreshes === 1 && __importedSaves === 2 && JSON.stringify(__importedSnapshots[0]) === JSON.stringify(__importedSnapshots[1])").unwrap().as_bool(), Some(true));
        // Unsupported listing neither refreshes nor attempts the broader org route.
        assert!(
            session
                .domains()
                .list(JsValue::UNDEFINED, None)
                .await
                .is_err()
        );
        assert_eq!(
            js_sys::eval("__dataCalls.includes('/api/v1/domains')")
                .unwrap()
                .as_bool(),
            Some(false)
        );
        data.close().await;
        JsFuture::from(session.close()).await.unwrap();
    }

    #[wasm_bindgen_test]
    async fn imported_data_cancellation_keeps_save_owned_until_session_close() {
        let (_restore, session) = imported_fixture(
            r#"
            __importedSaves++;
            globalThis.__importedCancel.abort();
            return new Promise(resolve => { globalThis.__releaseImportedSave = () => { __importedSaved = true; resolve(); }; });
        "#,
        );
        let controller = web_sys::AbortController::new().unwrap();
        js_sys::Reflect::set(
            &js_sys::global(),
            &"__importedCancel".into(),
            controller.as_ref(),
        )
        .unwrap();
        let data = session.data(DOMAIN.into()).unwrap();
        let error = data
            .read(DATA.into(), Some(controller.signal()))
            .await
            .unwrap_err();
        assert_eq!(
            js_sys::Reflect::get(&error, &"kind".into()).unwrap(),
            "cancelled"
        );
        let closing = session.close();
        js_sys::Reflect::set(
            &js_sys::global(),
            &"__importedClosing".into(),
            closing.as_ref(),
        )
        .unwrap();
        let done = js_sys::eval(
            r#"(async () => {
            let closed = false; __importedClosing.then(() => { closed = true; });
            await Promise.resolve(); if (closed) throw Error('close abandoned credential save');
            __releaseImportedSave(); await __importedClosing; return true;
        })()"#,
        )
        .unwrap();
        assert_eq!(
            JsFuture::from(done.unchecked_into::<Promise>())
                .await
                .unwrap()
                .as_bool(),
            Some(true)
        );
        assert_eq!(
            js_sys::eval(
                "__dataCalls.length === 0 && __importedRefreshes === 1 && __importedSaved"
            )
            .unwrap()
            .as_bool(),
            Some(true)
        );
        data.close().await;
    }
}
