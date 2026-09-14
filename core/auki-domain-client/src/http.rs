//! Bounded response reads. Dropping the request cancels the native request or
//! aborts Fetch; redirects cannot replay bearer credentials or uploaded bytes.

use crate::DataError;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "http_tests.rs"]
mod tests;
#[cfg(all(test, target_arch = "wasm32"))]
#[path = "http_wasm_tests.rs"]
mod wasm_tests;

fn check_headers(
    status: u16,
    content_type: Option<&str>,
    length: Option<u64>,
    maximum: usize,
    json: bool,
) -> Result<(), DataError> {
    if !(200..300).contains(&status) {
        return Err(DataError::HttpStatus { status });
    }
    if json
        && !content_type
            .and_then(|v| v.split(';').next())
            .is_some_and(|v| v.trim().eq_ignore_ascii_case("application/json"))
    {
        return Err(DataError::InvalidResponse("expected application/json"));
    }
    if length.is_some_and(|length| length > maximum as u64) {
        return Err(DataError::TooLarge { maximum });
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn send(
    request: reqwest::RequestBuilder,
    maximum: usize,
    json: bool,
) -> Result<Vec<u8>, DataError> {
    use futures::StreamExt;
    let map_error = |error: reqwest::Error| {
        if error.is_timeout() {
            DataError::TimedOut
        } else {
            DataError::Transport
        }
    };
    let response = request.send().await.map_err(map_error)?;
    check_headers(
        response.status().as_u16(),
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        response.content_length(),
        maximum,
        json,
    )?;
    let mut chunks = response.bytes_stream();
    let mut body = Vec::new();
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk.map_err(map_error)?;
        if chunk.len() > maximum.saturating_sub(body.len()) {
            return Err(DataError::TooLarge { maximum });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

// reqwest's WASM Fetch adapter cannot disable redirects. Mirror the SDK's
// ZITADEL transport policy here for Domain data requests.
#[cfg(target_arch = "wasm32")]
pub(crate) async fn send(
    request: reqwest::RequestBuilder,
    maximum: usize,
    json: bool,
) -> Result<Vec<u8>, DataError> {
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::JsFuture;
    struct AbortOnDrop(web_sys::AbortController);
    impl Drop for AbortOnDrop {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let failure = |_| DataError::Transport;
    let request = request
        .build()
        .map_err(|_| DataError::InvalidInput("invalid HTTP request"))?;
    let abort = AbortOnDrop(web_sys::AbortController::new().map_err(failure)?);
    let init = web_sys::RequestInit::new();
    init.set_method(request.method().as_str());
    init.set_redirect(web_sys::RequestRedirect::Error);
    init.set_credentials(web_sys::RequestCredentials::Omit);
    init.set_cache(web_sys::RequestCache::NoStore);
    init.set_mode(web_sys::RequestMode::Cors);
    init.set_referrer_policy(web_sys::ReferrerPolicy::NoReferrer);
    init.set_signal(Some(&abort.0.signal()));
    let headers = web_sys::Headers::new().map_err(failure)?;
    for (name, value) in request.headers() {
        headers
            .set(
                name.as_str(),
                value
                    .to_str()
                    .map_err(|_| DataError::InvalidInput("invalid header"))?,
            )
            .map_err(failure)?;
    }
    init.set_headers(&headers);
    if let Some(body) = request.body() {
        let bytes = body
            .as_bytes()
            .ok_or(DataError::InvalidInput("buffered bytes required"))?;
        init.set_body(&js_sys::Uint8Array::from(bytes));
    }
    let request =
        web_sys::Request::new_with_str_and_init(request.url().as_str(), &init).map_err(failure)?;
    let global = js_sys::global();
    let fetch: js_sys::Function = js_sys::Reflect::get(&global, &JsValue::from_str("fetch"))
        .map_err(failure)?
        .dyn_into()
        .map_err(failure)?;
    let promise: js_sys::Promise = fetch
        .call1(&global, &request)
        .map_err(failure)?
        .dyn_into()
        .map_err(failure)?;
    let response: web_sys::Response = JsFuture::from(promise)
        .await
        .map_err(failure)?
        .dyn_into()
        .map_err(failure)?;
    check_headers(
        response.status(),
        response
            .headers()
            .get("content-type")
            .map_err(failure)?
            .as_deref(),
        response
            .headers()
            .get("content-length")
            .map_err(failure)?
            .and_then(|v| v.parse().ok()),
        maximum,
        json,
    )?;
    let Some(stream) = response.body() else {
        return Ok(Vec::new());
    };
    let reader: web_sys::ReadableStreamDefaultReader = stream
        .get_reader()
        .dyn_into()
        .map_err(|_| DataError::Transport)?;
    let mut body = Vec::new();
    loop {
        let chunk = JsFuture::from(reader.read()).await.map_err(failure)?;
        if js_sys::Reflect::get(&chunk, &JsValue::from_str("done"))
            .map_err(failure)?
            .as_bool()
            == Some(true)
        {
            break;
        }
        let bytes: js_sys::Uint8Array = js_sys::Reflect::get(&chunk, &JsValue::from_str("value"))
            .map_err(failure)?
            .dyn_into()
            .map_err(failure)?;
        if bytes.length() as usize > maximum.saturating_sub(body.len()) {
            return Err(DataError::TooLarge { maximum });
        }
        body.extend_from_slice(&bytes.to_vec());
    }
    reader.release_lock();
    Ok(body)
}
