// SPDX-License-Identifier: GPL-3.0-or-later

use crate::GooglePlayApi;
use crate::error::PlayError;
use crate::playproto::ResponseWrapper;
use indexmap::IndexMap;
use prost::Message;
use reqwest::{Error as ReqwestError, Response, header::HeaderMap};
use std::collections::HashMap;
use tracing::{debug, error};

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

impl GooglePlayApi {
    pub(crate) async fn post_form(&self, url: &str, headers: HeaderMap, params: IndexMap<String, String>) -> Result<Response, ReqwestError> {
        debug!("POST form request to {url}");
        let resp = self.client.post(url).headers(headers).form(&params).send().await?;
        resp.error_for_status()
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
        debug!("POST protobuf request to {url}");
        let builder = self.client.post(url).headers(headers);
        let builder = if let Some(body) = body { builder.body(body) } else { builder };
        let builder = if let Some(params) = params { builder.form(&params) } else { builder };

        let resp = builder.send().await?;
        let resp = resp.error_for_status()?;
        let resp_bytes = resp.bytes().await?;

        Resp::decode(resp_bytes.as_ref()).map_err(|e| {
            error!("Failed to decode protobuf response from {url}: {e}");
            e.into()
        })
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
        debug!("GET protobuf request to {url}");
        let builder = self.client.get(url).headers(headers);
        let builder = if let Some(params) = params { builder.query(&params) } else { builder };

        let resp = builder.send().await?;
        let resp = resp.error_for_status()?;
        let resp_bytes = resp.bytes().await?;

        Resp::decode(resp_bytes.as_ref()).map_err(|e| {
            error!("Failed to decode protobuf response from {url}: {e}");
            e.into()
        })
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
