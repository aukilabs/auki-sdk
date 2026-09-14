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
    maximum: u64,
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
    if length.is_some_and(|length| length > maximum) {
        return Err(DataError::TooLarge {
            maximum: usize::try_from(maximum).unwrap_or(usize::MAX),
        });
    }
    Ok(())
}

pub(crate) async fn send(
    request: reqwest::RequestBuilder,
    maximum: usize,
    json: bool,
) -> Result<Vec<u8>, DataError> {
    let mut response = open(request, maximum as u64, json).await?;
    let mut body = Vec::new();
    while let Some(chunk) = response.next(64 * 1024).await? {
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(not(target_arch = "wasm32"))]
fn map_error(error: reqwest::Error) -> DataError {
    if error.is_timeout() {
        DataError::TimedOut
    } else {
        DataError::Transport
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn open(
    request: reqwest::RequestBuilder,
    maximum: u64,
    json: bool,
) -> Result<ResponseBody, DataError> {
    use futures::StreamExt;
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
    Ok(ResponseBody {
        chunks: response.bytes_stream().boxed(),
        pending: None,
        received: 0,
        maximum,
    })
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct ResponseBody {
    chunks: futures::stream::BoxStream<'static, Result<bytes::Bytes, reqwest::Error>>,
    pending: Option<bytes::Bytes>,
    received: u64,
    maximum: u64,
}

#[cfg(not(target_arch = "wasm32"))]
impl ResponseBody {
    pub async fn next(&mut self, chunk_size: usize) -> Result<Option<Vec<u8>>, DataError> {
        use futures::StreamExt;
        loop {
            if let Some(bytes) = &mut self.pending {
                let length = bytes.len().min(chunk_size);
                let result = bytes.split_to(length).to_vec();
                if bytes.is_empty() {
                    self.pending = None;
                }
                return Ok(Some(result));
            }
            let Some(bytes) = self.chunks.next().await else {
                return Ok(None);
            };
            let bytes = bytes.map_err(map_error)?;
            if bytes.len() as u64 > self.maximum.saturating_sub(self.received) {
                return Err(DataError::TooLarge {
                    maximum: usize::try_from(self.maximum).unwrap_or(usize::MAX),
                });
            }
            self.received += bytes.len() as u64;
            if !bytes.is_empty() {
                self.pending = Some(bytes);
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
struct AbortOnDrop(web_sys::AbortController);
#[cfg(target_arch = "wasm32")]
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) struct ResponseBody {
    // Must remain alive until all bytes have been consumed, including sink waits.
    abort: AbortOnDrop,
    reader: Option<web_sys::ReadableStreamDefaultReader>,
    pending: Option<js_sys::Uint8Array>,
    offset: u32,
    received: u64,
    maximum: u64,
}
#[cfg(target_arch = "wasm32")]
impl Drop for ResponseBody {
    fn drop(&mut self) {
        self.abort.0.abort();
        if let Some(reader) = &self.reader {
            reader.release_lock();
        }
    }
}
#[cfg(target_arch = "wasm32")]
impl ResponseBody {
    pub async fn next(&mut self, chunk_size: usize) -> Result<Option<Vec<u8>>, DataError> {
        use wasm_bindgen::{JsCast, JsValue};
        use wasm_bindgen_futures::JsFuture;
        loop {
            if let Some(bytes) = &self.pending {
                let end = (self.offset as usize + chunk_size).min(bytes.length() as usize) as u32;
                let result = bytes.subarray(self.offset, end).to_vec();
                self.offset = end;
                if end == bytes.length() {
                    self.pending = None;
                }
                return Ok(Some(result));
            }
            let Some(reader) = &self.reader else {
                return Ok(None);
            };
            let chunk = JsFuture::from(reader.read())
                .await
                .map_err(|_| DataError::Transport)?;
            if js_sys::Reflect::get(&chunk, &JsValue::from_str("done"))
                .map_err(|_| DataError::Transport)?
                .as_bool()
                == Some(true)
            {
                return Ok(None);
            }
            let bytes: js_sys::Uint8Array =
                js_sys::Reflect::get(&chunk, &JsValue::from_str("value"))
                    .map_err(|_| DataError::Transport)?
                    .dyn_into()
                    .map_err(|_| DataError::Transport)?;
            if bytes.length() as u64 > self.maximum.saturating_sub(self.received) {
                return Err(DataError::TooLarge {
                    maximum: usize::try_from(self.maximum).unwrap_or(usize::MAX),
                });
            }
            self.received += bytes.length() as u64;
            self.offset = 0;
            if bytes.length() > 0 {
                self.pending = Some(bytes);
            }
        }
    }
}

// reqwest's WASM Fetch adapter cannot disable redirects. Mirror the SDK's
// ZITADEL transport policy here for Domain data requests.
#[cfg(target_arch = "wasm32")]
pub(crate) async fn open(
    request: reqwest::RequestBuilder,
    maximum: u64,
    json: bool,
) -> Result<ResponseBody, DataError> {
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::JsFuture;
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
    let reader = response
        .body()
        .map(|stream| {
            stream
                .get_reader()
                .dyn_into::<web_sys::ReadableStreamDefaultReader>()
        })
        .transpose()
        .map_err(|_| DataError::Transport)?;
    Ok(ResponseBody {
        abort,
        reader,
        pending: None,
        offset: 0,
        received: 0,
        maximum,
    })
}
