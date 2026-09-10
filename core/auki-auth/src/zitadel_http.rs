use reqwest::Url;

use crate::{AuthLimits, Error, Result};

pub(crate) struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn request(
    client: &reqwest::Client,
    url: &Url,
    form: Option<&[(&str, &str)]>,
    limits: AuthLimits,
    endpoint: &'static str,
) -> Result<Response> {
    use futures::StreamExt;
    let request = match form {
        Some(form) => client.post(url.clone()).form(form),
        None => client.get(url.clone()),
    }
    .header("Accept", "application/json")
    .header("Cache-Control", "no-store");
    let response = request.send().await.map_err(|e| {
        if e.is_timeout() {
            Error::RequestTimedOut { endpoint }
        } else {
            Error::Transport { endpoint }
        }
    })?;
    check_headers(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        response.content_length(),
        limits,
        endpoint,
    )?;
    let status = response.status().as_u16();
    let mut chunks = response.bytes_stream();
    let mut body = Vec::new();
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk.map_err(|e| {
            if e.is_timeout() {
                Error::RequestTimedOut { endpoint }
            } else {
                Error::Transport { endpoint }
            }
        })?;
        if chunk.len() > limits.max_response_bytes.saturating_sub(body.len()) {
            return Err(Error::ResponseTooLarge {
                endpoint,
                maximum: limits.max_response_bytes,
            });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(Response { status, body })
}

fn check_headers(
    content_type: Option<&str>,
    length: Option<u64>,
    limits: AuthLimits,
    endpoint: &'static str,
) -> Result<()> {
    if !content_type
        .and_then(|v| v.split(';').next())
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("application/json"))
    {
        return Err(Error::invalid_response(
            endpoint,
            "Content-Type must be application/json",
        ));
    }
    if length.is_some_and(|v| v > limits.max_response_bytes as u64) {
        return Err(Error::ResponseTooLarge {
            endpoint,
            maximum: limits.max_response_bytes,
        });
    }
    Ok(())
}

// reqwest 0.12's Wasm transport cannot disable Fetch redirects. Refresh tokens
// require redirect:error BEFORE submission, not merely checking the final URL.
// Keep this adapter scoped to ZITADEL; API/DDS transport remains unchanged.
#[cfg(target_arch = "wasm32")]
pub(crate) async fn request(
    url: &Url,
    form: Option<&[(&str, &str)]>,
    limits: AuthLimits,
    endpoint: &'static str,
) -> Result<Response> {
    use futures::{
        future::{Either, select},
        pin_mut,
    };
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::JsFuture;

    struct AbortOnDrop(web_sys::AbortController);
    impl Drop for AbortOnDrop {
        fn drop(&mut self) {
            self.0.abort();
        }
    }

    let failure = |_| Error::Transport { endpoint };
    let abort = AbortOnDrop(web_sys::AbortController::new().map_err(failure)?);
    let operation = async {
        let init = web_sys::RequestInit::new();
        init.set_method(if form.is_some() { "POST" } else { "GET" });
        init.set_redirect(web_sys::RequestRedirect::Error);
        init.set_credentials(web_sys::RequestCredentials::Omit);
        init.set_cache(web_sys::RequestCache::NoStore);
        init.set_mode(web_sys::RequestMode::Cors);
        init.set_referrer_policy(web_sys::ReferrerPolicy::NoReferrer);
        init.set_signal(Some(&abort.0.signal()));
        let headers = web_sys::Headers::new().map_err(failure)?;
        headers.set("Accept", "application/json").map_err(failure)?;
        if let Some(form) = form {
            headers
                .set("Content-Type", "application/x-www-form-urlencoded")
                .map_err(failure)?;
            let params = web_sys::UrlSearchParams::new().map_err(failure)?;
            for (key, value) in form {
                params.append(key, value);
            }
            init.set_body(&params.to_string().into());
        }
        init.set_headers(&headers);
        let request =
            web_sys::Request::new_with_str_and_init(url.as_str(), &init).map_err(failure)?;
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
            limits,
            endpoint,
        )?;
        let status = response.status();
        let stream = response
            .body()
            .ok_or_else(|| Error::invalid_response(endpoint, "missing response body"))?;
        let reader: web_sys::ReadableStreamDefaultReader = stream
            .get_reader()
            .dyn_into()
            .map_err(|_| Error::Transport { endpoint })?;
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
            let value =
                js_sys::Reflect::get(&chunk, &JsValue::from_str("value")).map_err(failure)?;
            let bytes: js_sys::Uint8Array = value.dyn_into().map_err(failure)?;
            if bytes.length() as usize > limits.max_response_bytes.saturating_sub(body.len()) {
                return Err(Error::ResponseTooLarge {
                    endpoint,
                    maximum: limits.max_response_bytes,
                });
            }
            body.extend_from_slice(&bytes.to_vec());
        }
        reader.release_lock();
        Ok(Response { status, body })
    };
    let timeout = futures_timer::Delay::new(limits.request_timeout);
    pin_mut!(operation, timeout);
    match select(operation, timeout).await {
        Either::Left((result, _)) => result,
        Either::Right(((), _)) => Err(Error::RequestTimedOut { endpoint }),
    }
}
