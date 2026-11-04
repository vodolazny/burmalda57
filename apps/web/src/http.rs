// HTTP-адаптер браузера: исполняет core `ApiRequest` через fetch.
//
// CORS: сервер дневника должен разрешать запросы, либо страница открывается
// как `...?proxy=/api` — тогда запросы к mp2.obr57.ru идут через прокси
// на том же origin. Внешний адрес из URL не получает сохранённую сессию.

use std::sync::OnceLock;
use std::time::Duration;

use burmalda57_core::api::API_BASE;
use burmalda57_core::{ApiRequest, HttpResponse, HttpResult, TransportError};
use js_sys::Uint8Array;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::Response as WebResponse;
use web_sys::{window, AbortController, Headers, RequestInit};

pub(crate) struct WebHttp;

impl WebHttp {
    pub(crate) fn spawn<F>(&self, future: F)
    where
        F: std::future::Future<Output = ()> + 'static,
    {
        wasm_bindgen_futures::spawn_local(future);
    }

    pub(crate) async fn execute(&self, req: &ApiRequest) -> HttpResult {
        let win = window().ok_or(TransportError::Offline)?;

        let abort = AbortController::new().map_err(|_| TransportError::Offline)?;
        let timeout_ms = Duration::from_secs(req.timeout_secs.max(1)).as_millis() as i32;
        let timer = {
            let controller = abort.clone();
            let closure = Closure::<dyn Fn()>::new(move || controller.abort());
            win.set_timeout_with_callback_and_timeout_and_arguments_0(
                closure.as_ref().unchecked_ref(),
                timeout_ms,
            )
            .ok()
            .map(|id| (id, closure))
        };

        let result = fetch(&win, &abort, req).await;

        if let Some((id, _)) = timer {
            win.clear_timeout_with_handle(id);
        }
        result
    }
}

fn api_origin() -> &'static Result<String, String> {
    static ORIGIN: OnceLock<Result<String, String>> = OnceLock::new();
    ORIGIN.get_or_init(|| {
        if let Some(loc) = window().map(|w| w.location()) {
            if let Ok(search) = loc.search() {
                if let Ok(params) = web_sys::UrlSearchParams::new_with_str(&search) {
                    if let Some(p) = params.get("proxy") {
                        let p = p.trim();
                        if !p.is_empty() {
                            let base = loc.href().map_err(|_| "Не удалось определить адрес сайта".to_string())?;
                            let proxy = web_sys::Url::new_with_base(p, &base)
                                .map_err(|_| "Некорректный адрес прокси".to_string())?;
                            let origin = loc.origin().map_err(|_| "Не удалось определить адрес сайта".to_string())?;
                            if proxy.origin() != origin || !proxy.username().is_empty()
                                || !proxy.password().is_empty() || !proxy.search().is_empty()
                                || !proxy.hash().is_empty() {
                                return Err("Прокси должен находиться на этом же сайте. Используйте, например, ?proxy=/api".into());
                            }
                            return Ok(proxy.href().trim_end_matches('/').to_string());
                        }
                    }
                }
            }
        }
        Ok(API_BASE.to_string())
    })
}

pub(crate) fn configuration_error() -> Option<&'static str> {
    api_origin().as_ref().err().map(String::as_str)
}

fn api_url(path: &str) -> Result<String, TransportError> {
    match path.strip_prefix(API_BASE) {
        Some(rest) => api_origin()
            .as_ref()
            .map(|origin| format!("{origin}{rest}"))
            .map_err(|_| TransportError::Offline),
        None => Ok(path.to_string()),
    }
}

fn is_abort(err: &JsValue) -> bool {
    err.is_instance_of::<js_sys::Error>()
        && err.dyn_ref::<js_sys::Error>().unwrap().name() == "AbortError"
}

async fn fetch(win: &web_sys::Window, abort: &AbortController, req: &ApiRequest) -> HttpResult {
    let headers = Headers::new().map_err(|_| TransportError::Offline)?;
    for (name, value) in &req.headers {
        // Host/User-Agent управляются браузером и запрещены в fetch.
        if name.eq_ignore_ascii_case("host") || name.eq_ignore_ascii_case("user-agent") {
            continue;
        }
        let _ = headers.set(name, value);
    }

    let init = RequestInit::new();
    init.set_method(req.method);
    init.set_headers(&headers);
    init.set_signal(Some(&abort.signal()));
    // A redirect must not forward a stored SID to another endpoint.
    init.set_redirect(web_sys::RequestRedirect::Error);
    if let Some(body) = &req.body {
        let arr = Uint8Array::from(body.as_slice());
        init.set_body(&arr.into());
    }

    let promise = win.fetch_with_str_and_init(&api_url(&req.url)?, &init);
    let resp_val = match JsFuture::from(promise).await {
        Ok(v) => v,
        Err(e) => {
            return Err(if is_abort(&e) {
                TransportError::Timeout
            } else {
                TransportError::Offline
            })
        }
    };

    let resp: WebResponse = resp_val.dyn_into().map_err(|_| TransportError::Offline)?;
    let status = resp.status();
    let body = JsFuture::from(resp.array_buffer().map_err(|_| TransportError::Offline)?)
        .await
        .map_err(|e| {
            if is_abort(&e) {
                TransportError::Timeout
            } else {
                TransportError::Offline
            }
        })?;

    Ok(HttpResponse {
        status,
        body: Uint8Array::new(&body).to_vec(),
    })
}
