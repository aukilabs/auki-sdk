//! Bounded JSON transport. Error bodies are never exposed. Redirects cannot
//! replay credentials or a submission; dropping a WASM request aborts Fetch.

use super::JobsError;

#[cfg(all(test, target_arch = "wasm32"))]
#[path = "http_wasm_tests.rs"]
mod tests;

fn check_headers(
    status: u16,
    content_type: Option<&str>,
    length: Option<u64>,
    maximum: usize,
) -> Result<(), JobsError> {
    if !(200..300).contains(&status) {
        return Err(JobsError::HttpStatus { status });
    }
    if !content_type
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"))
    {
        return Err(JobsError::InvalidResponse("expected application/json"));
    }
    if length.is_some_and(|length| length > maximum as u64) {
        return Err(JobsError::TooLarge { maximum });
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn map_error(error: reqwest::Error) -> JobsError {
    if error.is_timeout() {
        JobsError::TimedOut
    } else {
        JobsError::Transport
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) async fn send(
    request: reqwest::RequestBuilder,
    maximum: usize,
) -> Result<Vec<u8>, JobsError> {
    let mut response = request.send().await.map_err(map_error)?;
    check_headers(
        response.status().as_u16(),
        response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        response.content_length(),
        maximum,
    )?;
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(map_error)? {
        if chunk.len() > maximum.saturating_sub(body.len()) {
            return Err(JobsError::TooLarge { maximum });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(target_arch = "wasm32")]
struct FetchGuard {
    abort: web_sys::AbortController,
    reader: Option<web_sys::ReadableStreamDefaultReader>,
}

#[cfg(target_arch = "wasm32")]
impl Drop for FetchGuard {
    fn drop(&mut self) {
        self.abort.abort();
        if let Some(reader) = &self.reader {
            reader.release_lock();
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub(super) async fn send(
    request: reqwest::RequestBuilder,
    maximum: usize,
) -> Result<Vec<u8>, JobsError> {
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::JsFuture;

    let request = request
        .build()
        .map_err(|_| JobsError::InvalidInput("invalid HTTP request"))?;
    let mut guard = FetchGuard {
        abort: web_sys::AbortController::new().map_err(|_| JobsError::Transport)?,
        reader: None,
    };
    let init = web_sys::RequestInit::new();
    init.set_method(request.method().as_str());
    init.set_redirect(web_sys::RequestRedirect::Error);
    init.set_credentials(web_sys::RequestCredentials::Omit);
    init.set_cache(web_sys::RequestCache::NoStore);
    init.set_mode(web_sys::RequestMode::Cors);
    init.set_referrer_policy(web_sys::ReferrerPolicy::NoReferrer);
    init.set_signal(Some(&guard.abort.signal()));
    let headers = web_sys::Headers::new().map_err(|_| JobsError::Transport)?;
    for (name, value) in request.headers() {
        headers
            .set(
                name.as_str(),
                value
                    .to_str()
                    .map_err(|_| JobsError::InvalidInput("invalid header"))?,
            )
            .map_err(|_| JobsError::Transport)?;
    }
    init.set_headers(&headers);
    if let Some(body) = request.body() {
        let bytes = body
            .as_bytes()
            .ok_or(JobsError::InvalidInput("buffered bytes required"))?;
        init.set_body(&js_sys::Uint8Array::from(bytes));
    }
    let request = web_sys::Request::new_with_str_and_init(request.url().as_str(), &init)
        .map_err(|_| JobsError::Transport)?;
    let global = js_sys::global();
    let fetch: js_sys::Function = js_sys::Reflect::get(&global, &JsValue::from_str("fetch"))
        .map_err(|_| JobsError::Transport)?
        .dyn_into()
        .map_err(|_| JobsError::Transport)?;
    let promise: js_sys::Promise = fetch
        .call1(&global, &request)
        .map_err(|_| JobsError::Transport)?
        .dyn_into()
        .map_err(|_| JobsError::Transport)?;
    let response: web_sys::Response = JsFuture::from(promise)
        .await
        .map_err(|_| JobsError::Transport)?
        .dyn_into()
        .map_err(|_| JobsError::Transport)?;
    check_headers(
        response.status(),
        response
            .headers()
            .get("content-type")
            .map_err(|_| JobsError::Transport)?
            .as_deref(),
        response
            .headers()
            .get("content-length")
            .map_err(|_| JobsError::Transport)?
            .and_then(|value| value.parse().ok()),
        maximum,
    )?;
    guard.reader = response
        .body()
        .map(|stream| {
            stream
                .get_reader()
                .dyn_into::<web_sys::ReadableStreamDefaultReader>()
        })
        .transpose()
        .map_err(|_| JobsError::Transport)?;
    let mut body = Vec::new();
    if let Some(reader) = &guard.reader {
        loop {
            let chunk = JsFuture::from(reader.read())
                .await
                .map_err(|_| JobsError::Transport)?;
            if js_sys::Reflect::get(&chunk, &JsValue::from_str("done"))
                .map_err(|_| JobsError::Transport)?
                .as_bool()
                == Some(true)
            {
                break;
            }
            let bytes: js_sys::Uint8Array =
                js_sys::Reflect::get(&chunk, &JsValue::from_str("value"))
                    .map_err(|_| JobsError::Transport)?
                    .dyn_into()
                    .map_err(|_| JobsError::Transport)?;
            if bytes.length() as usize > maximum.saturating_sub(body.len()) {
                return Err(JobsError::TooLarge { maximum });
            }
            body.extend_from_slice(&bytes.to_vec());
        }
    }
    Ok(body)
}
