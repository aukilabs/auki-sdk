use super::*;
use wasm_bindgen_test::wasm_bindgen_test;

struct RestoreFetch;
impl Drop for RestoreFetch {
    fn drop(&mut self) {
        js_sys::eval("globalThis.fetch = globalThis.__jobsFetch; delete globalThis.__jobsFetch; delete globalThis.__jobsSignal").unwrap();
    }
}

// Exercise the actual WASM transport rather than a native substitute. The only
// Fetch implementation here is this local stub; no shared service is contacted.
#[wasm_bindgen_test]
async fn fetch_bounds_json_disables_credential_redirects_and_aborts_dropped_requests() {
    use std::future::Future;

    js_sys::eval(r#"
        globalThis.__jobsFetch = globalThis.fetch;
        globalThis.fetch = async request => {
            if (request.redirect !== 'error' || request.credentials !== 'omit' || request.cache !== 'no-store') throw Error('unsafe Fetch policy');
            if (request.referrerPolicy !== 'no-referrer') throw Error('referrer leaked');
            if (request.headers.get('authorization') !== 'Bearer fixture') throw Error('missing bearer');
            globalThis.__jobsSignal = request.signal;
            if (request.url.endsWith('/pending')) return new Promise(() => {});
            if (request.url.endsWith('/denied')) return new Response('fixture private detail', {status:403});
            return new Response(new ReadableStream({start(controller) {
                controller.enqueue(new TextEncoder().encode('{"ok":'));
                controller.enqueue(new TextEncoder().encode('true}'));
                controller.close();
            }}), {headers: {'Content-Type': 'application/json'}});
        };
    "#).unwrap();
    let _restore = RestoreFetch;
    let client = reqwest::Client::new();
    let request = |path| {
        client
            .get(format!("https://dms.example/v1/jobs/{path}"))
            .bearer_auth("fixture")
    };
    assert_eq!(send(request("ok"), 100).await.unwrap(), br#"{"ok":true}"#);
    assert!(matches!(
        send(request("large"), 7).await,
        Err(JobsError::TooLarge { maximum: 7 })
    ));
    let denied = send(request("denied"), 100).await.unwrap_err();
    assert_eq!(denied.http_status(), Some(403));
    assert!(!format!("{denied:?}").contains("fixture private detail"));
    let mut pending = Box::pin(send(request("pending"), 100));
    let waker = std::task::Waker::noop();
    let mut cx = std::task::Context::from_waker(waker);
    assert!(pending.as_mut().poll(&mut cx).is_pending());
    drop(pending);
    assert_eq!(
        js_sys::eval("globalThis.__jobsSignal.aborted")
            .unwrap()
            .as_bool(),
        Some(true)
    );
}
