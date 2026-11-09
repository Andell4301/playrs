// SPDX-License-Identifier: GPL-3.0-or-later

use java_properties::PropertiesError;
use prost::DecodeError as ProtobufDecodeError;
use reqwest::{Error as ReqwestError, StatusCode, header::InvalidHeaderValue};
use serde_json::Error as SerdeError;
use std::io::Error as IoError;
use std::path::PathBuf;
use std::time::SystemTimeError;
use thiserror::Error;
use zip::result::ZipError;

#[derive(Debug, Error)]
pub enum DeviceError {
    #[error("Device with codename '{0}' not found")]
    NotFound(String),
    #[error(transparent)]
    ParseError(#[from] PropertiesError),
    #[error("Device is missing required fields: {0:?}")]
    MissingFields(Vec<String>),
    #[error(transparent)]
    TimestampError(#[from] SystemTimeError),
    #[error("Locale parse error: {0}")]
    LocaleParseError(String),
}

#[derive(Debug, Error)]
pub enum PlayError {
    #[error("JSON serialization/deserialization error: {0}")]
    SerdeError(#[from] SerdeError),
    #[error("Device error: {0}")]
    DeviceError(#[from] DeviceError),
    #[error("HTTP request error: {0}")]
    ReqwestError(#[source] ReqwestError),
    #[error("{method} {route} failed: {source}")]
    HttpRequestError {
        method: &'static str,
        route: String,
        #[source]
        source: ReqwestError,
    },
    #[error("Header error: {0}")]
    HeaderError(#[from] InvalidHeaderValue),
    #[error("I/O error: {0}")]
    IoError(#[from] IoError),
    #[error("Protobuf decode error: {0}")]
    ProtobufDecodeError(#[from] ProtobufDecodeError),
    #[error("{method} {route} returned invalid protobuf: {source}")]
    ProtobufResponseError {
        method: &'static str,
        route: String,
        #[source]
        source: ProtobufDecodeError,
    },
    #[error("ZIP error: {0}")]
    ZipError(#[from] ZipError),
    #[error("Authentication error: {0}")]
    AuthenticationError(String),
    #[error("Conversion error: {0}")]
    ConversionError(String),
    #[error("HTTP status error: {0}")]
    HttpStatusError(StatusCode),
    #[error("Invalid output directory")]
    InvalidOutputDirectory(PathBuf),
    #[error("Missing app details")]
    MissingAppDetails,
    #[error("Missing field error: {0}")]
    MissingFieldError(String),
    #[error("Purchase error: {0}")]
    PurchaseError(String),
    #[error("Failed to add or edit review")]
    ReviewError,
    #[error("No downloadable files returned")]
    EmptyDownloads,
}

impl PlayError {
    pub(crate) fn http_request(method: &'static str, route: impl Into<String>, source: ReqwestError) -> Self {
        Self::HttpRequestError { method, route: route.into(), source: source.without_url() }
    }

    pub(crate) fn http_status(&self) -> Option<StatusCode> {
        match self {
            Self::ReqwestError(source) | Self::HttpRequestError { source, .. } => source.status(),
            Self::HttpStatusError(status) => Some(*status),
            _ => None,
        }
    }
}

impl From<ReqwestError> for PlayError {
    fn from(source: ReqwestError) -> Self {
        Self::ReqwestError(source.without_url())
    }
}

#[cfg(test)]
mod tests {
    use super::PlayError;

    #[test]
    fn reqwest_errors_drop_the_url() {
        let secret = "delivery-token-that-must-not-leak";
        let url = reqwest::Url::parse(&format!("https://example.invalid/delivery?dtok={secret}")).expect("failed to build test URL");
        let source = reqwest::Client::new()
            .get("https://example.invalid")
            .header("invalid\nheader", "value")
            .build()
            .expect_err("invalid header should fail")
            .with_url(url);
        let error = PlayError::from(source);

        assert!(!error.to_string().contains(secret));
        assert!(!format!("{error:?}").contains(secret));
    }
}
