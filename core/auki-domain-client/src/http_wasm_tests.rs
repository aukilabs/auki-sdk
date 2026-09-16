use super::*;
use wasm_bindgen_test::wasm_bindgen_test;

// Test Fetch policies and streamed response bounds without contacting services.
struct RestoreFetch;
impl Drop for RestoreFetch {
    fn drop(&mut self) {
        js_sys::eval("globalThis.fetch = globalThis.__aukiOriginalFetch; delete globalThis.__aukiOriginalFetch").unwrap();
    }
}

#[wasm_bindgen_test]
async fn fetch_sets_redirect_and_credentials_policy_and_bounds_streamed_bytes() {
    js_sys::eval(r#"
        globalThis.__aukiOriginalFetch = globalThis.fetch;
        globalThis.fetch = async request => {
            if (request.redirect !== 'error' || request.credentials !== 'omit' || request.cache !== 'no-store') throw Error('unsafe Fetch policy');
            if (request.headers.get('authorization') !== 'Bearer fixture') throw Error('missing bearer');
            return new Response(new ReadableStream({start(controller) {
                controller.enqueue(new Uint8Array([1,2,3,4]));
                controller.enqueue(new Uint8Array([5,6,7,8]));
                controller.close();
            }}));
        };
    "#).unwrap();
    let _restore = RestoreFetch;
    let request = reqwest::Client::new()
        .get("https://domain.example/data")
        .bearer_auth("fixture");
    assert!(matches!(
        send(request, 6, false).await,
        Err(DataError::TooLarge { maximum: 6 })
    ));
}

#[wasm_bindgen_test]
async fn fetch_returns_bytes_and_aborts_a_cancelled_operation() {
    js_sys::eval(
        r#"
        globalThis.__aukiOriginalFetch = globalThis.fetch;
        globalThis.fetch = async request => {
            globalThis.__aukiLastSignal = request.signal;
            if (request.url.endsWith('/pending')) return new Promise(() => {});
            return new Response(new Uint8Array([1,2,3]));
        };
    "#,
    )
    .unwrap();
    let _restore = RestoreFetch;
    let client = reqwest::Client::new();
    assert_eq!(
        send(client.get("https://domain.example/data"), 8, false)
            .await
            .unwrap(),
        vec![1, 2, 3]
    );
    let mut pending = Box::pin(send(client.get("https://domain.example/pending"), 8, false));
    assert!(futures::poll!(pending.as_mut()).is_pending());
    drop(pending);
    assert_eq!(
        js_sys::eval("globalThis.__aukiLastSignal.aborted")
            .unwrap()
            .as_bool(),
        Some(true)
    );
    js_sys::eval("delete globalThis.__aukiLastSignal").unwrap();
}
