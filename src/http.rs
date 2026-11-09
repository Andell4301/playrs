// SPDX-License-Identifier: GPL-3.0-or-later

use crate::GooglePlayApi;
use crate::error::PlayError;
use crate::playproto::ResponseWrapper;
use indexmap::IndexMap;
use prost::Message;
use reqwest::{Response, header::HeaderMap};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tracing::debug;

static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

macro_rules! get_payload_field {
    ($response:expr, $field:ident) => {{
        let payload = $response.payload.ok_or_else(|| PlayError::MissingFieldError("ResponseWrapper.payload".into()))?;
        payload.$field.ok_or_else(|| {
            let field_name = concat!("ResponseWrapper.payload.", stringify!($field));
            PlayError::MissingFieldError(field_name.into())
        })
    }};
}
pub(crate) use get_payload_field;

pub(crate) struct FormResponse {
    response: Response,
    route: String,
    started: Instant,
    request_id: u64,
}

impl FormResponse {
    pub(crate) async fn bytes(self) -> Result<Vec<u8>, PlayError> {
        let status = self.response.status();
        let bytes = self.response.bytes().await.map_err(|error| PlayError::http_request("POST", self.route.clone(), error))?;

        debug!(
            http.method = "POST",
            http.route = %self.route,
            request_id = self.request_id,
            http.status = status.as_u16(),
            elapsed_ms = elapsed_ms(self.started.elapsed()),
            response_bytes = bytes.len(),
            "response"
        );

        Ok(bytes.to_vec())
    }
}

fn safe_route(url: &str) -> String {
    reqwest::Url::parse(url).ok().map(|url| url.path().to_owned()).filter(|path| !path.is_empty()).unwrap_or_else(|| "<invalid>".to_owned())
}

fn elapsed_ms(elapsed: Duration) -> u64 {
    elapsed.as_millis().try_into().unwrap_or(u64::MAX)
}

pub(crate) fn next_request_id() -> u64 {
    NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed)
}

fn log_request(method: &'static str, route: &str, request_id: u64) {
    debug!(http.method = method, http.route = %route, request_id, "request");
}

fn log_response(method: &'static str, route: &str, status: reqwest::StatusCode, started: Instant, request_id: u64) {
    debug!(
        http.method = method,
        http.route = %route,
        request_id,
        http.status = status.as_u16(),
        elapsed_ms = elapsed_ms(started.elapsed()),
        "response"
    );
}

impl GooglePlayApi {
    pub(crate) async fn post_form(&self, url: &str, headers: HeaderMap, params: IndexMap<String, String>) -> Result<FormResponse, PlayError> {
        let method = "POST";
        let route = safe_route(url);
        let started = Instant::now();
        let request_id = next_request_id();
        log_request(method, &route, request_id);

        let response = self
            .client
            .post(url)
            .headers(headers)
            .form(&params)
            .send()
            .await
            .map_err(|error| PlayError::http_request(method, route.clone(), error))?;
        let status = response.status();
        let response = match response.error_for_status() {
            Ok(response) => response,
            Err(error) => {
                log_response(method, &route, status, started, request_id);
                return Err(PlayError::http_request(method, route, error));
            }
        };

        Ok(FormResponse { response, route, started, request_id })
    }

    pub(crate) fn parse_form_response(response: &[u8]) -> HashMap<String, String> {
        let text = String::from_utf8_lossy(response);
        text.lines().filter_map(|line| line.split_once('=')).map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    pub(crate) async fn post_play_proto<Resp>(
        &self,
        url: &str,
        headers: HeaderMap,
        params: Option<IndexMap<String, String>>,
        body: Option<Vec<u8>>,
    ) -> Result<Resp, PlayError>
    where
        Resp: Message + Default,
    {
        let method = "POST";
        let route = safe_route(url);
        let started = Instant::now();
        let request_id = next_request_id();
        log_request(method, &route, request_id);

        let builder = self.client.post(url).headers(headers);
        let builder = if let Some(body) = body { builder.body(body) } else { builder };
        let builder = if let Some(params) = params { builder.form(&params) } else { builder };

        let response = builder.send().await.map_err(|error| PlayError::http_request(method, route.clone(), error))?;
        let status = response.status();
        let response = match response.error_for_status() {
            Ok(response) => response,
            Err(error) => {
                log_response(method, &route, status, started, request_id);
                return Err(PlayError::http_request(method, route, error));
            }
        };
        let response_bytes = response.bytes().await.map_err(|error| PlayError::http_request(method, route.clone(), error))?;

        debug!(
            http.method = method,
            http.route = %route,
            request_id,
            http.status = status.as_u16(),
            elapsed_ms = elapsed_ms(started.elapsed()),
            response_bytes = response_bytes.len(),
            "response"
        );

        Resp::decode(response_bytes.as_ref()).map_err(|source| PlayError::ProtobufResponseError { method, route, source })
    }

    pub(crate) async fn get_play_proto<Resp>(
        &self,
        url: &str,
        headers: HeaderMap,
        params: Option<IndexMap<String, String>>,
    ) -> Result<Resp, PlayError>
    where
        Resp: Message + Default,
    {
        let method = "GET";
        let route = safe_route(url);
        let started = Instant::now();
        let request_id = next_request_id();
        log_request(method, &route, request_id);

        let builder = self.client.get(url).headers(headers);
        let builder = if let Some(params) = params { builder.query(&params) } else { builder };

        let response = builder.send().await.map_err(|error| PlayError::http_request(method, route.clone(), error))?;
        let status = response.status();
        let response = match response.error_for_status() {
            Ok(response) => response,
            Err(error) => {
                log_response(method, &route, status, started, request_id);
                return Err(PlayError::http_request(method, route, error));
            }
        };
        let response_bytes = response.bytes().await.map_err(|error| PlayError::http_request(method, route.clone(), error))?;

        debug!(
            http.method = method,
            http.route = %route,
            request_id,
            http.status = status.as_u16(),
            elapsed_ms = elapsed_ms(started.elapsed()),
            response_bytes = response_bytes.len(),
            "response"
        );

        Resp::decode(response_bytes.as_ref()).map_err(|source| PlayError::ProtobufResponseError { method, route, source })
    }

    pub(crate) async fn post_play_wrapper_resp(
        &self,
        url: &str,
        headers: HeaderMap,
        params: Option<IndexMap<String, String>>,
        body: Option<Vec<u8>>,
    ) -> Result<ResponseWrapper, PlayError> {
        self.post_play_proto::<ResponseWrapper>(url, headers, params, body).await
    }

    pub(crate) async fn get_play_wrapper_resp(
        &self,
        url: &str,
        headers: HeaderMap,
        params: Option<IndexMap<String, String>>,
    ) -> Result<ResponseWrapper, PlayError> {
        self.get_play_proto::<ResponseWrapper>(url, headers, params).await
    }
}

#[cfg(test)]
mod tests {
    use super::{next_request_id, safe_route};

    #[test]
    fn safe_route_drops_query_and_fragment() {
        assert_eq!(safe_route("https://example.com/fdfe/delivery?token=secret#part"), "/fdfe/delivery");
    }

    #[test]
    fn safe_route_does_not_echo_invalid_urls() {
        assert_eq!(safe_route("not a url?token=secret"), "<invalid>");
    }

    #[test]
    fn request_ids_are_unique() {
        assert_ne!(next_request_id(), next_request_id());
    }
}
