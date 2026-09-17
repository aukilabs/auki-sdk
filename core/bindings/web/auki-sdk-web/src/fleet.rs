use crate::{AukiUserSession, jobs::Cancellation};
use auki_sdk::{ComputePoolQuery, DomainFleetClient, FleetError, FleetQuery, JobMode};
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

#[wasm_bindgen(typescript_custom_section)]
const TYPES: &str = r#"
export interface FleetQuery { capabilities?: string[]; matchAllCapabilities?: boolean }
export interface ComputePoolQuery extends FleetQuery { mode: JobMode }
export type FleetPresence = "online" | "offline" | "unknown";
export type FleetWorkState = "idle" | "busy" | "unknown";
export interface FleetActivity { worker_id: string; job_id: string; task_id: string; task_status: JobTaskStatus; job_status: JobStatus; mode: JobMode; capability: string; lease_expires_at: string | null; last_heartbeat_at: string | null; updated_at: string; observed_at: string }
export interface FleetMachine { kind: "robot" | "compute"; id: string; organization_id: string; name: string; capabilities: string[]; mode: string; association: "assigned" | "active_task" | "candidate"; presence: FleetPresence; provider_status: string; presence_observed_at: string; last_seen_at: string | null; active_lease_expires_at: string | null; work_state: FleetWorkState; work_observed_at: string | null; activity: FleetActivity[] }
export interface FleetSourceReport { source: "robots" | "nodes" | "jobs" | "busy"; state: "complete" | "partial" | "denied" | "unsupported" | "unavailable"; observed_at: string; codes: string[]; http_status: number | null }
export interface FleetSnapshot { domain_id: string; view: "domain" | "compute_pool"; observed_at: string; machines: FleetMachine[]; unresolved_activity: FleetActivity[]; sources: FleetSourceReport[]; complete: boolean }
export interface AukiFleetError extends Error { kind: string; code: string; status?: number }
"#;

fn error(error: FleetError) -> JsValue {
    let result = js_sys::Error::new(&error.to_string());
    let _ = js_sys::Reflect::set(&result, &"kind".into(), &error.kind().into());
    let _ = js_sys::Reflect::set(&result, &"code".into(), &error.code().into());
    if let Some(status) = error.http_status() {
        let _ = js_sys::Reflect::set(&result, &"status".into(), &status.into());
    }
    result.into()
}

fn encode(value: &impl Serialize) -> Result<JsValue, JsValue> {
    value
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .map_err(|_| error(FleetError::InvalidResponse("cannot convert fleet snapshot")))
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct Query {
    capabilities: Vec<String>,
    match_all_capabilities: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PoolQuery {
    mode: JobMode,
    #[serde(default)]
    capabilities: Vec<String>,
    #[serde(default)]
    match_all_capabilities: bool,
}

#[wasm_bindgen]
impl AukiUserSession {
    pub fn fleet(&self, domain_id: String) -> Result<AukiFleet, JsValue> {
        let id = domain_id
            .parse()
            .map_err(|_| error(FleetError::InvalidInput("expected Domain UUID")))?;
        Ok(AukiFleet {
            inner: self.bootstrap.fleet().map_err(error)?.in_domain(id),
        })
    }
}

#[wasm_bindgen]
pub struct AukiFleet {
    inner: DomainFleetClient,
}

#[wasm_bindgen]
impl AukiFleet {
    #[wasm_bindgen(getter,js_name=domainId)]
    pub fn domain_id(&self) -> String {
        self.inner.domain_id().to_string()
    }
    pub async fn close(&self) {
        self.inner.close().await;
    }

    #[wasm_bindgen(unchecked_return_type = "FleetSnapshot")]
    pub async fn list(
        &self,
        #[wasm_bindgen(unchecked_param_type = "FleetQuery | undefined")] query: JsValue,
        signal: Option<web_sys::AbortSignal>,
    ) -> Result<JsValue, JsValue> {
        let query: Query = if query.is_undefined() || query.is_null() {
            Query::default()
        } else {
            serde_wasm_bindgen::from_value(query)
                .map_err(|_| error(FleetError::InvalidInput("invalid fleet query")))?
        };
        let cancel = Cancellation::new(signal)?;
        encode(
            &self
                .inner
                .list_with_cancellation(
                    &FleetQuery {
                        capabilities: query.capabilities,
                        match_all_capabilities: query.match_all_capabilities,
                    },
                    &cancel.token,
                )
                .await
                .map_err(error)?,
        )
    }

    #[wasm_bindgen(js_name=computePool,unchecked_return_type="FleetSnapshot")]
    pub async fn compute_pool(
        &self,
        #[wasm_bindgen(unchecked_param_type = "ComputePoolQuery")] query: JsValue,
        signal: Option<web_sys::AbortSignal>,
    ) -> Result<JsValue, JsValue> {
        let query: PoolQuery = serde_wasm_bindgen::from_value(query)
            .map_err(|_| error(FleetError::InvalidInput("invalid compute pool query")))?;
        let cancel = Cancellation::new(signal)?;
        encode(
            &self
                .inner
                .compute_pool_with_cancellation(
                    &ComputePoolQuery {
                        mode: query.mode,
                        capabilities: query.capabilities,
                        match_all_capabilities: query.match_all_capabilities,
                    },
                    &cancel.token,
                )
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

    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            js_sys::eval(
                "globalThis.fetch=globalThis.__fleetFetch; delete globalThis.__fleetFetch",
            )
            .unwrap();
        }
    }
    fn fixture() -> Restore {
        js_sys::eval(r#"
          globalThis.__fleetFetch=globalThis.fetch;
          globalThis.__fleetDeny=false; globalThis.__fleetBlock=false;
          globalThis.__fleetStarted=false; globalThis.__fleetAborted=false;
          globalThis.fetch=async (input,init) => {
            const request=input instanceof Request ? input : new Request(input,init);
            const url=new URL(request.url);
            const domain='aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa';
            const org='dddddddd-dddd-4ddd-8ddd-dddddddddddd';
            const json=(value,status=200) => {
              const response=new Response(JSON.stringify(value),{status,headers:{'content-type':'application/json'}});
              Object.defineProperty(response,'url',{value:request.url}); return response;
            };
            if(url.pathname==='/user/login') return json({access_token:'fixture-user',refresh_token:'fixture-refresh'});
            if(url.pathname==='/service/domains-access-token') return json({access_token:'fixture-service'});
            if(url.pathname.endsWith('/auth')) {
              const claims={iss:'dds',type:'user-access',org,domain_id:domain,aud:['dds','https://server.example'],exp:Math.floor(Date.now()/1000)+3600};
              const token='e30.'+btoa(JSON.stringify(claims)).replaceAll('=','').replaceAll('+','-').replaceAll('/','_')+'.fixture';
              return json({id:domain,domain_server:{url:'https://server.example'},access_token:token});
            }
            if(request.method!=='GET') throw Error('mutating fleet request');
            if(url.pathname.startsWith('/v1/') && (request.credentials!=='omit' || request.redirect!=='error')) throw Error('unsafe DMS request');
            if(!request.headers.get('authorization')?.startsWith('Bearer ')) throw Error('missing authorization');
            if(url.pathname.endsWith('/robots')) return json({robots:[{id:'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb',organization_id:org,assigned_domain_id:domain,name:'inspector',capabilities:['vendor/v7'],status:'online',last_seen_at:null,active_lease_expires_at:null}]});
            if(url.pathname==='/api/v1/nodes') return json({nodes:[{id:'cccccccc-cccc-4ccc-8ccc-cccccccccccc',organization_id:org,name:'compute',capabilities:['vendor/v7'],status:'online',mode:'dedicated'}]});
            if(url.pathname==='/v1/jobs') {
              if(__fleetBlock) {
                globalThis.__fleetStarted=true;
                return new Promise((_,reject)=>request.signal.addEventListener('abort',()=>{globalThis.__fleetAborted=true;reject(new DOMException('aborted','AbortError'));}));
              }
              return json({items:[],next_cursor:null});
            }
            if(url.pathname==='/v1/nodes/busy') return json({nodes:[]},__fleetDeny?403:200);
            return json({},404);
          };
        "#).unwrap();
        Restore
    }
    async fn login() -> AukiUserSession {
        AukiUserSession::login_with_environment_and_client_id(
            "https://api.example".into(),
            "https://dds.example".into(),
            "https://dms.example/v1/".into(),
            "fixture@example.test".into(),
            "fixture-password".into(),
            None,
        )
        .await
        .unwrap()
    }
    fn object(value: JsValue) -> auki_sdk::FleetSnapshot {
        serde_wasm_bindgen::from_value(value).unwrap()
    }

    #[wasm_bindgen_test]
    async fn fleet_inventory_pool_and_partial_sources_use_shared_session() {
        let _restore = fixture();
        let session = login().await;
        let fleet = session.fleet(DOMAIN.into()).unwrap();
        let inventory = object(
            fleet
                .list(
                    js_sys::eval("({capabilities:['vendor/v7'],matchAllCapabilities:true})")
                        .unwrap(),
                    None,
                )
                .await
                .unwrap(),
        );
        assert!(inventory.complete, "{:?}", inventory.sources);
        assert_eq!(
            inventory.machines[0].association,
            auki_sdk::FleetAssociation::Assigned
        );
        assert_eq!(
            inventory.machines[0].work_state,
            auki_sdk::FleetWorkState::Idle
        );
        assert_eq!(inventory.machines.len(), 1);
        let pool = object(
            fleet
                .compute_pool(js_sys::eval("({mode:'dedicated'})").unwrap(), None)
                .await
                .unwrap(),
        );
        assert_eq!(
            pool.machines[0].association,
            auki_sdk::FleetAssociation::Candidate
        );
        assert!(pool.machines[0].last_seen_at.is_none());
        js_sys::eval("globalThis.__fleetDeny=true").unwrap();
        let partial = object(fleet.list(JsValue::UNDEFINED, None).await.unwrap());
        assert!(!partial.complete);
        assert_eq!(
            partial.machines[0].work_state,
            auki_sdk::FleetWorkState::Unknown
        );
        assert!(
            partial
                .sources
                .iter()
                .any(|s| s.source == auki_sdk::FleetSource::Busy
                    && s.state == auki_sdk::FleetSourceState::Denied)
        );
        fleet.close().await;
        let error = fleet.list(JsValue::UNDEFINED, None).await.unwrap_err();
        assert_eq!(
            js_sys::Reflect::get(&error, &"code".into()).unwrap(),
            "closed"
        );
        let second = session.fleet(DOMAIN.into()).unwrap();
        second.list(JsValue::UNDEFINED, None).await.unwrap();
        second.close().await;
        JsFuture::from(session.close()).await.unwrap();
    }

    #[wasm_bindgen_test]
    async fn fleet_abort_reaches_fetch_and_close_drains_requests() {
        let _restore = fixture();
        let session = login().await;
        let fleet = session.fleet(DOMAIN.into()).unwrap();
        js_sys::eval("globalThis.__fleetBlock=true").unwrap();
        let controller = web_sys::AbortController::new().unwrap();
        js_sys::Reflect::set(
            &js_sys::global(),
            &"__fleetAbort".into(),
            controller.as_ref(),
        )
        .unwrap();
        js_sys::eval("{let ticks=0;const timer=setInterval(()=>{if(__fleetStarted || ++ticks>1000){clearInterval(timer);globalThis.__fleetAbort.abort();}},1);}").unwrap();
        let result = fleet
            .list(JsValue::UNDEFINED, Some(controller.signal()))
            .await;
        let error = result.unwrap_err();
        assert_eq!(
            js_sys::Reflect::get(&error, &"code".into()).unwrap(),
            "cancelled"
        );
        fleet.close().await;
        assert_eq!(js_sys::eval("__fleetAborted").unwrap(), true);
        JsFuture::from(session.close()).await.unwrap();
    }
}
