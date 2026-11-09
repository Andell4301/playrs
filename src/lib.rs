// SPDX-FileCopyrightText: 2020-2026 Aurora OSS
// SPDX-FileCopyrightText: 2023-2025 The Calyx Institute
// SPDX-FileCopyrightText: 2021-2026 David Weinstein, Electronic Frontier Foundation
// SPDX-License-Identifier: GPL-3.0-or-later

mod auth;
pub mod constants;
pub mod device;
pub mod error;
mod http;

#[rustfmt::skip]
pub mod playproto;

use auth::AuthData;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use constants::{
    CategoryType, ClusterType, LEGACY_USER_AGENT, ModifyWishlistAction, PatchFormat, PlayFileType, ReviewFilter, StreamCategory, StreamType,
    URL_ACQUIRE, URL_BULK_DETAILS, URL_CATEGORIES, URL_DELIVERY, URL_DETAILS, URL_FDFE, URL_MODIFY_LIBRARY, URL_PURCHASE, URL_PURCHASE_HISTORY,
    URL_REVIEW_ADD_EDIT, URL_REVIEW_DELETE, URL_REVIEWS, URL_SEARCH, URL_SEARCH_SUGGEST, URL_TESTING_PROGRAM, URL_TOP_CHART, URL_USER_PROFILE,
};
use device::Device;
use error::PlayError;
use http::{get_payload_field, next_request_id};
use indexmap::IndexMap;
use playproto::{
    AcquireRequest, AcquireResponseWrapper, BrowseResponse, BulkDetailsEntry, BulkDetailsRequest, DeliveryResponse, DetailsResponse, Field, Item,
    ListResponse, ModifyLibraryRequest, ResponseWrapper, ResponseWrapperApi, Review, ReviewResponse, SearchSuggestEntry, StreamResponse,
    TestingProgramRequest, TestingProgramResponse, UserProfile,
    acquire_request::package::Payload as AcquirePayload,
    acquire_request::{Message30, Package, Version},
};
use prost::Message;
pub use reqwest;
use reqwest::{
    Client,
    header::{HeaderName, HeaderValue},
};
use serde::Serialize;
use serde_json::Value;
use std::fmt;
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::time::Instant;
use tokio::task;
use tracing::{debug, field, info, warn};
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

fn elapsed_ms(started_at: Instant) -> u64 {
    started_at.elapsed().as_millis().try_into().unwrap_or(u64::MAX)
}

fn validate_download_size(expected: i64, actual: usize) -> Result<(), PlayError> {
    let expected =
        usize::try_from(expected).map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("invalid download size: {expected}")))?;
    if expected != actual {
        return Err(
            std::io::Error::new(std::io::ErrorKind::InvalidData, format!("download size mismatch: expected {expected} bytes, got {actual}")).into()
        );
    }
    Ok(())
}

fn fdfe_url(stream_or_next_url: &str) -> Result<String, PlayError> {
    let base = reqwest::Url::parse(&format!("{URL_FDFE}/")).expect("valid FDFE base URL");
    let relative = stream_or_next_url.trim_start_matches('/');
    let relative = relative.strip_prefix("fdfe/").unwrap_or(relative);
    let url = base.join(relative).map_err(|_| PlayError::ConversionError("stream URL".to_owned()))?;
    if url.origin() != base.origin() {
        return Err(PlayError::ConversionError("stream URL must use the Google Play origin".to_owned()));
    }
    Ok(url.into())
}

fn list_response(response: ResponseWrapper) -> Result<ListResponse, PlayError> {
    if let Some(prefetched) =
        response.pre_fetch.and_then(|prefetch| prefetch.response).and_then(|response| response.payload).and_then(|payload| payload.list_response)
    {
        return Ok(prefetched);
    }
    let payload = response.payload.ok_or_else(|| PlayError::MissingFieldError("ResponseWrapper.payload".to_owned()))?;
    payload.list_response.ok_or_else(|| PlayError::MissingFieldError("ResponseWrapper.payload.list_response".to_owned()))
}

pub trait MessageJsonExt {
    fn to_json_value(&self) -> Result<Value, PlayError>;
}

impl<T> MessageJsonExt for T
where
    T: Serialize,
{
    fn to_json_value(&self) -> Result<Value, PlayError> {
        Ok(serde_json::to_value(self)?)
    }
}

#[derive(Clone)]
pub struct PlayFile {
    name: String,
    url: String,
    size: i64,
    file_type: PlayFileType,
}

impl fmt::Debug for PlayFile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PlayFile")
            .field("name", &self.name)
            .field("url", &"<redacted>")
            .field("size", &self.size)
            .field("file_type", &self.file_type)
            .finish()
    }
}

pub struct GooglePlayApi {
    device: Device,
    auth_data: AuthData,
    client: Client,
}

impl GooglePlayApi {
    pub fn get_device(&self) -> &Device {
        &self.device
    }

    pub fn get_auth_data(&self) -> &AuthData {
        &self.auth_data
    }

    pub fn with_client(mut self, client: Client) -> Self {
        self.client = client;
        self
    }
}

/// Delivery options, including the installed copy Play needs to offer binary patches.
#[derive(Clone)]
pub struct DeliveryRequest {
    pub package_name: String,
    pub version_code: i64,
    pub offer_type: i32,
    pub certificate_hash: Option<String>,
    pub split_module: Option<String>,
    pub installed_version_code: Option<i64>,
    pub patch_formats: Vec<PatchFormat>,
    pub installed_derived_apk_id: Option<i32>,
    pub installed_frosting_id: Option<i64>,
    pub delivery_token: Option<String>,
}

impl fmt::Debug for DeliveryRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeliveryRequest")
            .field("package_name", &self.package_name)
            .field("version_code", &self.version_code)
            .field("offer_type", &self.offer_type)
            .field("certificate_hash", &self.certificate_hash.as_ref().map(|_| "<redacted>"))
            .field("split_module", &self.split_module)
            .field("installed_version_code", &self.installed_version_code)
            .field("patch_formats", &self.patch_formats)
            .field("installed_derived_apk_id", &self.installed_derived_apk_id)
            .field("installed_frosting_id", &self.installed_frosting_id)
            .field("delivery_token", &self.delivery_token.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

impl DeliveryRequest {
    pub fn new(package_name: &str, version_code: i64, offer_type: i32) -> Self {
        Self {
            package_name: package_name.to_owned(),
            version_code,
            offer_type,
            certificate_hash: None,
            split_module: None,
            installed_version_code: None,
            patch_formats: vec![PatchFormat::GzippedBsdiff],
            installed_derived_apk_id: None,
            installed_frosting_id: None,
            delivery_token: None,
        }
    }

    pub fn certificate_hash(mut self, v: &str) -> Self {
        self.certificate_hash = Some(v.to_string());
        self
    }
    pub fn split_module(mut self, v: &str) -> Self {
        self.split_module = Some(v.to_string());
        self
    }
    pub fn installed_version_code(mut self, v: i64) -> Self {
        self.installed_version_code = Some(v);
        self
    }
    pub fn patch_format(mut self, v: PatchFormat) -> Self {
        self.patch_formats = vec![v];
        self
    }
    pub fn patch_formats(mut self, v: Vec<PatchFormat>) -> Self {
        self.patch_formats = v;
        self
    }
    pub fn installed_derived_apk_id(mut self, v: i32) -> Self {
        self.installed_derived_apk_id = Some(v);
        self
    }
    pub fn installed_frosting_id(mut self, v: i64) -> Self {
        self.installed_frosting_id = Some(v);
        self
    }
    pub fn delivery_token(mut self, v: &str) -> Self {
        self.delivery_token = Some(v.to_string());
        self
    }

    fn query(&self) -> Vec<(String, String)> {
        let mut query = vec![
            ("ot".to_owned(), self.offer_type.to_string()),
            ("doc".to_owned(), self.package_name.clone()),
            ("vc".to_owned(), self.version_code.to_string()),
        ];
        if let Some(version) = self.installed_version_code.filter(|version| *version > 0) {
            query.push(("bvc".to_owned(), version.to_string()));
            let mut formats = Vec::new();
            for format in &self.patch_formats {
                if !formats.contains(format) {
                    query.push(("pf".to_owned(), (*format as i32).to_string()));
                    formats.push(*format);
                }
            }
            if let Some(id) = self.installed_derived_apk_id.filter(|id| *id != 0) {
                query.push(("bda".to_owned(), id.to_string()));
            }
            if let Some(id) = self.installed_frosting_id.filter(|id| *id != 0) {
                query.push(("bf".to_owned(), id.to_string()));
            }
        }
        for (name, value) in [("mn", &self.split_module), ("ch", &self.certificate_hash), ("dtok", &self.delivery_token)] {
            if let Some(value) = value.as_ref().filter(|value| !value.is_empty()) {
                query.push((name.to_owned(), value.clone()));
            }
        }
        query
    }
}

/// Download options shared by the positional and request-based download methods.
#[derive(Debug, Clone)]
pub struct DownloadAppRequest {
    pub package_name: String,
    pub output_dir: PathBuf,
    pub version_code: Option<i64>,
    pub offer_type: Option<i32>,
    pub certificate_hash: Option<String>,
    pub split_module: Option<String>,
    pub installed_version_code: Option<i64>,
    pub patch_format: PatchFormat,
    /// Overrides `patch_format` when set; an empty list requests no patch formats.
    pub patch_formats: Option<Vec<PatchFormat>>,
    pub installed_derived_apk_id: Option<i32>,
    pub installed_frosting_id: Option<i64>,
    pub use_xapk: bool,
    pub custom_apk_name: Option<String>,
    pub include_dex: bool,
}

impl DownloadAppRequest {
    pub fn new(package_name: &str, output_dir: &Path) -> Self {
        Self {
            package_name: package_name.to_string(),
            output_dir: output_dir.to_path_buf(),
            version_code: None,
            offer_type: None,
            certificate_hash: None,
            split_module: None,
            installed_version_code: None,
            patch_format: PatchFormat::GzippedBsdiff,
            patch_formats: None,
            installed_derived_apk_id: None,
            installed_frosting_id: None,
            use_xapk: false,
            custom_apk_name: None,
            include_dex: false,
        }
    }

    pub fn version_code(mut self, v: i64) -> Self {
        self.version_code = Some(v);
        self
    }
    pub fn offer_type(mut self, v: i32) -> Self {
        self.offer_type = Some(v);
        self
    }
    pub fn certificate_hash(mut self, v: &str) -> Self {
        self.certificate_hash = Some(v.to_string());
        self
    }
    pub fn split_module(mut self, v: &str) -> Self {
        self.split_module = Some(v.to_string());
        self
    }
    pub fn installed_version_code(mut self, v: i64) -> Self {
        self.installed_version_code = Some(v);
        self
    }
    pub fn patch_format(mut self, v: PatchFormat) -> Self {
        self.patch_format = v;
        self.patch_formats = None;
        self
    }
    pub fn patch_formats(mut self, v: Vec<PatchFormat>) -> Self {
        self.patch_formats = Some(v);
        self
    }
    pub fn installed_derived_apk_id(mut self, v: i32) -> Self {
        self.installed_derived_apk_id = Some(v);
        self
    }
    pub fn installed_frosting_id(mut self, v: i64) -> Self {
        self.installed_frosting_id = Some(v);
        self
    }
    pub fn use_xapk(mut self) -> Self {
        self.use_xapk = true;
        self
    }
    pub fn custom_apk_name(mut self, v: &str) -> Self {
        self.custom_apk_name = Some(v.to_string());
        self
    }
    pub fn include_dex(mut self) -> Self {
        self.include_dex = true;
        self
    }

    fn delivery_request(&self, version_code: i64, offer_type: i32) -> DeliveryRequest {
        DeliveryRequest {
            package_name: self.package_name.clone(),
            version_code,
            offer_type,
            certificate_hash: self.certificate_hash.clone(),
            split_module: self.split_module.clone(),
            installed_version_code: self.installed_version_code,
            patch_formats: self.patch_formats.clone().unwrap_or_else(|| vec![self.patch_format]),
            installed_derived_apk_id: self.installed_derived_apk_id,
            installed_frosting_id: self.installed_frosting_id,
            delivery_token: None,
        }
    }
}

// Forced Constructors
// Doing it this way makes sure that all the required fields are there for at least one auth method
impl GooglePlayApi {
    pub fn new_from_aas_token(
        aas_token: impl Into<String>,
        email: impl Into<String>,
        device_name: Option<impl Into<String>>,
        device_locale: Option<&str>,
    ) -> Result<Self, PlayError> {
        let device = Device::new(device_name, device_locale)?;
        let auth_data = AuthData { aas_token: Some(aas_token.into()), email: Some(email.into()), ..Default::default() };
        Ok(Self { device, auth_data, client: Client::new() })
    }

    pub fn new_from_gsf_id(
        gsf_id: impl Into<String>,
        auth_token: impl Into<String>,
        device_name: Option<impl Into<String>>,
        device_locale: Option<&str>,
    ) -> Result<Self, PlayError> {
        let device = Device::new(device_name, device_locale)?;
        let auth_data = AuthData { gsf_id: Some(gsf_id.into()), auth_token: Some(auth_token.into()), ..Default::default() };
        Ok(Self { device, auth_data, client: Client::new() })
    }

    pub fn new_from_auth_token(
        auth_token: impl Into<String>,
        email: impl Into<String>,
        device_name: Option<impl Into<String>>,
        device_locale: Option<&str>,
    ) -> Result<Self, PlayError> {
        let device = Device::new(device_name, device_locale)?;
        let auth_data = AuthData { auth_token: Some(auth_token.into()), email: Some(email.into()), ..Default::default() };
        Ok(Self { device, auth_data, client: Client::new() })
    }

    pub fn new_from_oauth_login_token(
        oauth_login_token: impl Into<String>,
        email: impl Into<String>,
        device_name: Option<impl Into<String>>,
        device_locale: Option<&str>,
    ) -> Result<Self, PlayError> {
        let device = Device::new(device_name, device_locale)?;
        let auth_data = AuthData { oauth_login_token: Some(oauth_login_token.into()), email: Some(email.into()), ..Default::default() };
        Ok(Self { device, auth_data, client: Client::new() })
    }
}

// Public API Methods
impl GooglePlayApi {
    pub async fn get_user_profile(&self) -> Result<Option<UserProfile>, PlayError> {
        let method = "GET";
        let route = "/fdfe/api/userProfile";
        let started_at = Instant::now();
        let request_id = next_request_id();
        let headers = self.get_default_headers()?;
        debug!(http.method = method, http.route = route, request_id, "request");
        let response =
            self.client.get(URL_USER_PROFILE).headers(headers).send().await.map_err(|error| PlayError::http_request(method, route, error))?;
        let status = response.status();

        if status.is_success() {
            let bytes = response.bytes().await.map_err(|error| PlayError::http_request(method, route, error))?;
            debug!(
                http.method = method,
                http.route = route,
                request_id,
                http.status = status.as_u16(),
                elapsed_ms = elapsed_ms(started_at),
                response_bytes = bytes.len(),
                "response"
            );
            let wrapper = ResponseWrapperApi::decode(bytes.as_ref()).map_err(|source| PlayError::ProtobufResponseError {
                method,
                route: route.to_owned(),
                source,
            })?;
            let user_profile_response = get_payload_field!(wrapper, user_profile_response)?;
            return Ok(user_profile_response.user_profile);
        }

        debug!(http.method = method, http.route = route, request_id, http.status = status.as_u16(), elapsed_ms = elapsed_ms(started_at), "response");
        if status == reqwest::StatusCode::NOT_FOUND {
            Ok(None)
        } else {
            response.error_for_status_ref().map_err(|error| PlayError::http_request(method, route, error))?;
            Err(PlayError::HttpStatusError(status))
        }
    }

    pub async fn get_app_details_by_package_name(&self, package_name: &str) -> Result<DetailsResponse, PlayError> {
        let headers = self.get_default_headers()?;
        let params = IndexMap::from([("doc".to_owned(), package_name.to_owned())]);
        let response = self.get_play_wrapper_resp(URL_DETAILS, headers, Some(params)).await?;
        get_payload_field!(response, details_response)
    }

    pub async fn get_bulk_app_details_by_package_names(&self, package_names: Vec<String>) -> Result<Vec<BulkDetailsEntry>, PlayError> {
        let mut headers = self.get_default_headers()?;
        headers.insert(HeaderName::from_static("content-type"), HeaderValue::from_static("application/x-protobuf"));

        let request = BulkDetailsRequest {
            doc_id: package_names.clone(),
            include_child_docs: Some(true),
            include_details: Some(true),
            source_package_name: None,
            installed_version_code: vec![],
        };

        let response = self.post_play_wrapper_resp(URL_BULK_DETAILS, headers, None, Some(request.encode_to_vec())).await?;
        let bulk_details_response = get_payload_field!(response, bulk_details_response)?;
        Ok(bulk_details_response.entry)
    }

    pub async fn get_dev_stream(&self, dev_id: &str) -> Result<ListResponse, PlayError> {
        let headers = self.get_default_headers()?;
        let url = format!("{URL_FDFE}/getDeveloperPageStream");
        let params = IndexMap::from([("docid".to_owned(), format!("developer-{dev_id}"))]);
        let response = self.get_play_wrapper_resp(&url, headers, Some(params)).await?;
        list_response(response)
    }

    /// Read a stream response containing items and a node tree.
    pub async fn get_stream_tree_response(&self, stream_or_next_url: &str) -> Result<StreamResponse, PlayError> {
        let headers = self.get_default_headers()?;
        self.get_play_proto(&fdfe_url(stream_or_next_url)?, headers, None).await
    }

    pub async fn get_testing_program_details(&self, package_name: &str, subscribe: bool) -> Result<TestingProgramResponse, PlayError> {
        let headers = self.get_default_headers()?;
        let request = TestingProgramRequest { package_name: Some(package_name.to_owned()), subscribe: Some(subscribe) };
        let response = self.post_play_wrapper_resp(URL_TESTING_PROGRAM, headers, None, Some(request.encode_to_vec())).await?;
        get_payload_field!(response, testing_program_response)
    }

    pub async fn get_all_categories(&self, category_type: CategoryType) -> Result<Option<Item>, PlayError> {
        let mut headers = self.get_default_headers()?;
        headers.insert(HeaderName::from_static("user-agent"), HeaderValue::from_static(LEGACY_USER_AGENT));
        let params = IndexMap::from([("c".to_owned(), "3".to_owned()), ("cat".to_owned(), category_type.to_string())]);
        let response = self.get_play_wrapper_resp(URL_CATEGORIES, headers, Some(params)).await?;
        let list_response = get_payload_field!(response, list_response)?;
        Ok(list_response.item)
    }

    pub async fn get_category_stream(&self, stream_or_next_url: &str) -> Result<ListResponse, PlayError> {
        let headers = self.get_default_headers()?;
        let url = fdfe_url(stream_or_next_url)?;
        let response = self.get_play_wrapper_resp(&url, headers, None).await?;

        list_response(response)
    }

    pub async fn get_my_apps(&self, cluster_type: ClusterType) -> Result<ListResponse, PlayError> {
        let headers = self.get_default_headers()?;
        let params = IndexMap::from([("n".to_owned(), "15".to_owned()), ("tab".to_owned(), cluster_type.to_string())]);
        let url = format!("{URL_FDFE}/myAppsStream");
        let response = self.get_play_wrapper_resp(&url, headers, Some(params)).await?;
        list_response(response)
    }

    pub async fn get_next_stream_response(&self, next_url: &str) -> Result<ListResponse, PlayError> {
        let headers = self.get_default_headers()?;
        let url = fdfe_url(next_url)?;
        let response = self.get_play_wrapper_resp(&url, headers, None).await?;
        list_response(response)
    }

    pub async fn get_browse_response(&self, browse_url: &str) -> Result<BrowseResponse, PlayError> {
        let headers = self.get_default_headers()?;
        let url = fdfe_url(browse_url)?;
        let response = self.get_play_wrapper_resp(&url, headers, None).await?;
        get_payload_field!(response, browse_response)
    }

    pub async fn get_wishlist_apps(&self) -> Result<ListResponse, PlayError> {
        let headers = self.get_default_headers()?;
        let params = IndexMap::from([("c".to_owned(), "0".to_owned()), ("dt".to_owned(), "7".to_owned()), ("libid".to_owned(), "u-wl".to_owned())]);
        let url = format!("{URL_FDFE}/library");
        let response = self.get_play_wrapper_resp(&url, headers, Some(params)).await?;
        get_payload_field!(response, list_response)
    }

    pub async fn modify_wishlist(&self, action: ModifyWishlistAction, package_names: Vec<String>) -> Result<bool, PlayError> {
        let method = "POST";
        let route = "/fdfe/modifyLibrary";
        let started_at = Instant::now();
        let request_id = next_request_id();
        let headers = self.get_default_headers()?;
        let request = match action {
            ModifyWishlistAction::Add => {
                ModifyLibraryRequest { library_id: Some("u-wl".to_owned()), add_package_name: package_names, remove_package_name: vec![] }
            }
            ModifyWishlistAction::Remove => {
                ModifyLibraryRequest { library_id: Some("u-wl".to_owned()), add_package_name: vec![], remove_package_name: package_names }
            }
        };
        debug!(http.method = method, http.route = route, request_id, "request");
        let response = self
            .client
            .post(URL_MODIFY_LIBRARY)
            .headers(headers)
            .body(request.encode_to_vec())
            .send()
            .await
            .map_err(|error| PlayError::http_request(method, route, error))?;
        let status = response.status();
        debug!(http.method = method, http.route = route, request_id, http.status = status.as_u16(), elapsed_ms = elapsed_ms(started_at), "response");
        Ok(status.is_success())
    }

    pub async fn get_reviews(&self, package_name: &str, review_filter: ReviewFilter, result_num: u32) -> Result<ReviewResponse, PlayError> {
        let headers = self.get_default_headers()?;
        let mut params = IndexMap::from([("doc".to_owned(), package_name.to_owned()), ("n".to_owned(), result_num.to_string())]);

        match review_filter {
            ReviewFilter::Newest => {
                params.insert("sort".to_owned(), review_filter.to_string());
            }
            ReviewFilter::All => {
                params.insert("sfilter".to_owned(), review_filter.to_string());
            }
            ReviewFilter::Positive | ReviewFilter::Critical => {
                params.insert("sent".to_owned(), review_filter.to_string());
            }
            _ => {
                params.insert("rating".to_owned(), review_filter.to_string());
            }
        }

        let response = self.get_play_wrapper_resp(URL_REVIEWS, headers, Some(params)).await?;
        get_payload_field!(response, review_response)
    }

    pub async fn get_review_summary(&self, package_name: &str) -> Result<ReviewResponse, PlayError> {
        let headers = self.get_default_headers()?;
        let params = IndexMap::from([("doc".to_owned(), package_name.to_owned())]);
        let url = format!("{URL_FDFE}/reviewSummary");
        let response = self.get_play_wrapper_resp(&url, headers, Some(params)).await?;
        get_payload_field!(response, review_summary_response)
    }

    pub async fn get_user_review(&self, package_name: &str, testing: bool) -> Result<Option<Review>, PlayError> {
        let headers = self.get_default_headers()?;
        let params = IndexMap::from([("doc".to_owned(), package_name.to_owned()), ("itpr".to_owned(), testing.to_string())]);
        let url = format!("{URL_FDFE}/userReview");
        let response = self.get_play_wrapper_resp(&url, headers, Some(params)).await?;
        let review_response = get_payload_field!(response, review_response)?;
        if let Some(user_reviews_response) = review_response.user_reviews_response {
            if let Some(review) = user_reviews_response.review.into_iter().next() {
                return Ok(Some(review));
            }
        }
        Ok(None)
    }

    pub async fn add_or_edit_review(&self, package_name: &str, title: &str, content: &str, rating: u8, is_beta: bool) -> Result<Review, PlayError> {
        let headers = self.get_default_headers()?;
        let params = IndexMap::from([
            ("doc".to_owned(), package_name.to_owned()),
            ("title".to_owned(), title.to_owned()),
            ("content".to_owned(), content.to_owned()),
            ("rating".to_owned(), rating.to_string()),
            ("rst".to_owned(), "3".to_owned()),
            ("itpr".to_owned(), is_beta.to_string()),
        ]);
        let response = self.post_play_wrapper_resp(URL_REVIEW_ADD_EDIT, headers, Some(params), None).await?;
        let review_response = get_payload_field!(response, review_response)?;
        review_response.user_review.ok_or(PlayError::ReviewError)
    }

    pub async fn delete_review(&self, package_name: &str, is_beta: bool) -> Result<bool, PlayError> {
        let headers = self.get_default_headers()?;
        let params = IndexMap::from([("doc".to_owned(), package_name.to_owned()), ("itpr".to_owned(), is_beta.to_string())]);
        let resp = self.post_play_wrapper_resp(URL_REVIEW_DELETE, headers, Some(params), None).await;
        match resp {
            Ok(_) => Ok(true),
            Err(e) => Err(e),
        }
    }

    pub async fn get_next_reviews(&self, next_page_url: &str) -> Result<ReviewResponse, PlayError> {
        let headers = self.get_default_headers()?;
        let url = fdfe_url(next_page_url)?;
        let response = self.get_play_wrapper_resp(&url, headers, None).await?;
        get_payload_field!(response, review_response)
    }

    pub async fn search_suggestions(&self, query: &str) -> Result<Vec<SearchSuggestEntry>, PlayError> {
        let mut headers = self.get_default_headers()?;
        headers.insert(HeaderName::from_static("user-agent"), HeaderValue::from_static(LEGACY_USER_AGENT));
        let params = IndexMap::from([("q".to_owned(), query.to_owned())]);
        let url = format!("{URL_SEARCH_SUGGEST}?sb=5&sst=2&sst=3");
        let response = self.get_play_wrapper_resp(&url, headers, Some(params)).await?;
        let search_suggest_response = get_payload_field!(response, search_suggest_response)?;
        Ok(search_suggest_response.entry)
    }

    pub async fn search_results(&self, query: &str, next_page_url: Option<&str>) -> Result<ListResponse, PlayError> {
        let headers = self.get_default_headers()?;
        let (url, params) = if let Some(next_url) = next_page_url.filter(|url| !url.is_empty()) {
            (fdfe_url(next_url)?, None)
        } else {
            (
                URL_SEARCH.to_owned(),
                Some(IndexMap::from([("q".to_owned(), query.to_owned()), ("c".to_owned(), "3".to_owned()), ("ksm".to_owned(), "1".to_owned())])),
            )
        };
        let response = self.get_play_wrapper_resp(&url, headers, params).await?;
        list_response(response)
    }

    pub async fn get_list_response(&self, stream_type: StreamType, category: Option<StreamCategory>) -> Result<ListResponse, PlayError> {
        let headers = self.get_default_headers()?;
        let mut params = IndexMap::from([("c".to_owned(), "3".to_owned())]);

        match stream_type {
            StreamType::EarlyAccess => {
                params.insert("ct".to_owned(), "1".to_owned());
            }
            _ if category.is_some_and(|c| c != StreamCategory::None) => {
                params.insert("cat".to_owned(), category.unwrap().to_string());
            }
            _ => {}
        }

        let url = format!("{URL_FDFE}/{}", stream_type.to_owned());
        let response = self.get_play_wrapper_resp(&url, headers, Some(params)).await?;
        list_response(response)
    }

    pub async fn get_cluster(&self, category: &str, chart: &str) -> Result<Option<ListResponse>, PlayError> {
        let mut headers = self.get_default_headers()?;
        headers.insert(HeaderName::from_static("user-agent"), HeaderValue::from_static(LEGACY_USER_AGENT));
        let params =
            IndexMap::from([("c".to_owned(), "3".to_owned()), ("stcid".to_owned(), chart.to_owned()), ("scat".to_owned(), category.to_owned())]);
        match self.get_play_wrapper_resp(URL_TOP_CHART, headers, Some(params)).await {
            Ok(response) => list_response(response).map(Some),
            Err(error) => match error.http_status() {
                Some(status) if status == reqwest::StatusCode::NOT_FOUND => {
                    debug!(%status, "chart unavailable");
                    Ok(None)
                }
                _ => Err(error),
            },
        }
    }

    pub async fn get_purchase_history(&self, offset: u32) -> Result<ListResponse, PlayError> {
        let headers = self.get_default_headers()?;
        let params = IndexMap::from([("o".to_owned(), offset.to_string())]);
        let response = self.get_play_wrapper_resp(URL_PURCHASE_HISTORY, headers, Some(params)).await?;
        get_payload_field!(response, list_response)
    }

    pub async fn get_delivery_token(
        &self,
        package_name: &str,
        version_code: i64,
        offer_type: i32,
        certificate_hash: Option<&str>,
    ) -> Result<String, PlayError> {
        self.delivery_token_from(URL_PURCHASE, package_name, version_code, offer_type, certificate_hash).await
    }

    async fn delivery_token_from(
        &self,
        url: &str,
        package_name: &str,
        version_code: i64,
        offer_type: i32,
        certificate_hash: Option<&str>,
    ) -> Result<String, PlayError> {
        let headers = self.get_default_headers()?;
        let mut params = IndexMap::from([
            ("ot".to_owned(), offer_type.to_string()),
            ("doc".to_owned(), package_name.to_owned()),
            ("vc".to_owned(), version_code.to_string()),
        ]);
        if let Some(ch) = certificate_hash.filter(|hash| !hash.is_empty()) {
            params.insert("ch".to_owned(), ch.to_string());
        }
        let response = match self.post_play_wrapper_resp(url, headers, Some(params), None).await {
            Ok(response) => response,
            Err(error) if error.http_status() == Some(reqwest::StatusCode::BAD_REQUEST) => return Ok(String::new()),
            Err(error) => return Err(error),
        };
        Ok(response.payload.and_then(|payload| payload.buy_response).and_then(|buy| buy.encoded_delivery_token).unwrap_or_default())
    }

    pub async fn get_delivery_response(
        &self,
        package_name: &str,
        update_version_code: i64,
        offer_type: i32,
        split_module: Option<&str>,
        installed_version_code: Option<i64>,
        patch_format: PatchFormat,
        delivery_token: Option<&str>,
        certificate_hash: Option<&str>,
    ) -> Result<DeliveryResponse, PlayError> {
        let mut request = DeliveryRequest::new(package_name, update_version_code, offer_type);
        request.split_module = split_module.map(str::to_owned);
        request.installed_version_code = installed_version_code;
        request.patch_formats = vec![patch_format];
        request.delivery_token = delivery_token.map(str::to_owned);
        request.certificate_hash = certificate_hash.map(str::to_owned);
        self.get_delivery_response_request(&request).await
    }

    /// Request delivery with repeated patch formats and installed APK identity.
    pub async fn get_delivery_response_request(&self, request: &DeliveryRequest) -> Result<DeliveryResponse, PlayError> {
        let headers = self.get_default_headers()?;
        let mut url = reqwest::Url::parse(URL_DELIVERY).expect("valid delivery URL");
        url.query_pairs_mut().extend_pairs(request.query());
        let response = self.get_play_wrapper_resp(url.as_str(), headers, None).await?;
        get_payload_field!(response, delivery_response)
    }

    pub async fn acquire(&self, package_name: &str, version_code: i64, offer_type: i32) -> Result<AcquireResponseWrapper, PlayError> {
        let u64_vc: u64 = version_code.try_into().map_err(|_| PlayError::ConversionError("version_code".to_owned()))?;
        let u32_oc: u32 = offer_type.try_into().map_err(|_| PlayError::ConversionError("offer_type".to_owned()))?;

        let acquire_request = AcquireRequest {
            package: Some(Package {
                payload: Some(AcquirePayload { package_name: Some(package_name.to_owned()), f2: Some(1), f3: Some(3) }),
                f2: Some(1),
            }),
            version: Some(Version { version_code: Some(u64_vc), f3: Some(0) }),
            f8: Some(Field {}),
            f15: Some(0),
            offer_type: Some(u32_oc),
            nonce: Some(format!("nonce={}", URL_SAFE_NO_PAD.encode(&rand::random::<[u8; 256]>()))),
            f25: Some(2),
            m30: Some(Message30 { f1: Some(2), f2: Some(0) }),
        };
        let headers = self.get_default_headers()?;
        let body = acquire_request.encode_to_vec();
        self.post_play_proto(URL_ACQUIRE, headers, None, Some(body)).await
    }

    pub async fn purchase(
        &self,
        package_name: &str,
        version_code: i64,
        offer_type: i32,
        certificate_hash: Option<&str>,
        split_module: Option<&str>,
        installed_version_code: Option<i64>,
        patch_format: PatchFormat,
    ) -> Result<DeliveryResponse, PlayError> {
        let mut request = DeliveryRequest::new(package_name, version_code, offer_type);
        request.certificate_hash = certificate_hash.map(str::to_owned);
        request.split_module = split_module.map(str::to_owned);
        request.installed_version_code = installed_version_code;
        request.patch_formats = vec![patch_format];
        self.purchase_request(request).await
    }

    /// Acquire and resolve delivery, including optional installed APK patch parameters.
    #[tracing::instrument(
        name = "purchase",
        level = "debug",
        skip_all,
        fields(
            package_name = request.package_name.as_str(),
            version_code = request.version_code,
            offer_type = request.offer_type,
            status = field::Empty,
        )
    )]
    pub async fn purchase_request(&self, mut request: DeliveryRequest) -> Result<DeliveryResponse, PlayError> {
        // https://gitlab.com/AuroraOSS/gplayapi/-/commit/2180d4a50efac9b86f83e2bfb84db3366d41476d
        // dont care if this fails
        if let Err(e) = self.acquire(&request.package_name, request.version_code, request.offer_type).await {
            debug!(error = %e, "acquire failed; continuing");
        }

        let delivery_token =
            self.get_delivery_token(&request.package_name, request.version_code, request.offer_type, request.certificate_hash.as_deref()).await?;
        request.delivery_token = Some(delivery_token);
        let delivery_response = self.get_delivery_response_request(&request).await?;

        if let Some(status) = delivery_response.status {
            tracing::Span::current().record("status", status);
        }

        match delivery_response.status {
            Some(1) => {
                self.parse_delivery_response(&request.package_name, request.version_code, delivery_response.clone())?;
                debug!("purchase complete");
                Ok(delivery_response)
            }
            Some(2 | 9) => Err(PlayError::PurchaseError("App not supported for purchase".to_owned())),
            Some(3) => Err(PlayError::PurchaseError("App not purchased".to_owned())),
            Some(7) => Err(PlayError::PurchaseError("App removed from store".to_owned())),
            other => Err(PlayError::PurchaseError(format!("Unknown purchase error (status={other:?})"))),
        }
    }

    pub async fn download_app(
        &self,
        package_name: &str,
        output_dir: &Path,
        version_code: Option<i64>,
        offer_type: Option<i32>,
        certificate_hash: Option<&str>,
        split_module: Option<&str>,
        installed_version_code: Option<i64>,
        patch_format: PatchFormat,
        use_xapk: bool,
        custom_apk_name: Option<String>,
        include_dex: bool,
    ) -> Result<Vec<PathBuf>, PlayError> {
        self.download_app_request(DownloadAppRequest {
            version_code,
            offer_type,
            certificate_hash: certificate_hash.map(str::to_owned),
            split_module: split_module.map(str::to_owned),
            installed_version_code,
            patch_format,
            use_xapk,
            custom_apk_name,
            include_dex,
            ..DownloadAppRequest::new(package_name, output_dir)
        })
        .await
    }

    #[tracing::instrument(
        name = "download",
        level = "info",
        skip_all,
        fields(
            package_name = request.package_name.as_str(),
            version_code = field::Empty,
            offer_type = field::Empty,
            xapk = request.use_xapk,
            include_dex = request.include_dex,
        )
    )]
    pub async fn download_app_request(&self, request: DownloadAppRequest) -> Result<Vec<PathBuf>, PlayError> {
        let package_name = request.package_name.as_str();
        let output_dir = request.output_dir.as_path();
        let started_at = Instant::now();
        info!("download started");

        if output_dir.is_file() {
            return Err(PlayError::InvalidOutputDirectory(output_dir.to_path_buf()));
        }
        tokio::fs::create_dir_all(output_dir).await?;

        let details = self.get_app_details_by_package_name(package_name).await?;
        let app_details =
            details.item.clone().and_then(|item| item.details).and_then(|details| details.app_details).ok_or_else(|| PlayError::MissingAppDetails)?;

        let vc = request.version_code.unwrap_or(app_details.version_code.ok_or_else(|| PlayError::MissingFieldError("version_code".to_owned()))?);
        let ot = request
            .offer_type
            .unwrap_or(details.item.clone().and_then(|item| item.offer.into_iter().next()).and_then(|offer| offer.offer_type).unwrap_or(1));

        let span = tracing::Span::current();
        span.record("version_code", vc);
        span.record("offer_type", ot);

        let delivery_response = self.purchase_request(request.delivery_request(vc, ot)).await?;

        let files = self.parse_delivery_response(package_name, vc, delivery_response)?;

        let mut downloaded_paths = Vec::new();
        let mut downloaded_bytes = 0_u64;
        if files.len() == 1 {
            let file = &files[0];
            let output_path = output_dir.join(request.custom_apk_name.clone().unwrap_or_else(|| file.name.clone()));
            downloaded_bytes = downloaded_bytes.saturating_add(self.save_play_file(file, &output_path).await?);
            downloaded_paths.push(output_path);
        } else {
            for file in files.iter() {
                if file.file_type == PlayFileType::Dex && !request.include_dex {
                    continue;
                }
                let output_path = output_dir.join(&file.name);
                downloaded_bytes = downloaded_bytes.saturating_add(self.save_play_file(file, &output_path).await?);
                downloaded_paths.push(output_path);
            }
            if request.use_xapk {
                let xapk_path = self.make_xapk(package_name, details, files, output_dir, request.custom_apk_name, request.include_dex).await?;
                downloaded_paths.clear();
                downloaded_paths.push(xapk_path);
            }
        }

        info!(file_count = downloaded_paths.len(), downloaded_bytes, elapsed_ms = elapsed_ms(started_at), "download complete");
        Ok(downloaded_paths)
    }

    async fn save_play_file(&self, file: &PlayFile, output_path: &Path) -> Result<u64, PlayError> {
        let method = "GET";
        let route = "artifact";
        let started_at = Instant::now();
        let request_id = next_request_id();
        debug!(request_id, file_type = %file.file_type, expected_bytes = file.size, "file download started");

        let response = self.client.get(&file.url).send().await.map_err(|error| PlayError::http_request(method, route, error))?;
        let status = response.status();
        let response = match response.error_for_status() {
            Ok(response) => response,
            Err(error) => {
                debug!(
                    http.method = method,
                    http.route = route,
                    request_id,
                    http.status = status.as_u16(),
                    elapsed_ms = elapsed_ms(started_at),
                    "response"
                );
                return Err(PlayError::http_request(method, route, error));
            }
        };
        let data = response.bytes().await.map_err(|error| PlayError::http_request(method, route, error))?;
        let byte_count = data.len();

        validate_download_size(file.size, byte_count)?;

        tokio::fs::write(output_path, &data).await?;
        debug!(
            http.method = method,
            http.route = route,
            request_id,
            http.status = status.as_u16(),
            file_type = %file.file_type,
            byte_count,
            elapsed_ms = elapsed_ms(started_at),
            "file download complete"
        );
        Ok(byte_count as u64)
    }

    async fn make_xapk(
        &self,
        package_name: impl Into<String>,
        details: DetailsResponse,
        files: Vec<PlayFile>,
        output_dir: impl AsRef<Path>,
        custom_apk_name: Option<String>,
        include_dex: bool,
    ) -> Result<PathBuf, PlayError> {
        let package_name = package_name.into();
        let output_dir = output_dir.as_ref().to_path_buf();

        let icon_data = if let Some(base_apk) = files.iter().find(|file| file.file_type == PlayFileType::Base) {
            match tokio::fs::read(output_dir.join(&base_apk.name)).await {
                Ok(bytes) => Self::extract_icon_from_apk(bytes),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(error.into()),
            }
        } else {
            None
        };
        let mut manifest = self.make_xapk_manifest(details, files.clone(), include_dex)?;
        if icon_data.is_some() {
            manifest["icon"] = Value::String("icon.png".to_owned());
        }
        let manifest_path = output_dir.join("manifest.json");
        let manifest_json = serde_json::to_vec_pretty(&manifest)?;
        tokio::fs::write(&manifest_path, &manifest_json).await?;

        let xapk_name = custom_apk_name.unwrap_or_else(|| package_name.clone());
        let xapk_path = output_dir.join(format!("{xapk_name}.xapk"));
        let path_to_return = xapk_path.clone();

        let zip_output_dir = output_dir.clone();
        let zip_files = files.clone();

        let result = task::spawn_blocking(move || -> Result<(), PlayError> {
            use std::fs::File;
            use std::io::Write;

            let file = File::create(&xapk_path)?;
            let writer = BufWriter::new(file);
            let mut zip = ZipWriter::new(writer);
            let options = SimpleFileOptions::default();

            for play_file in &zip_files {
                if play_file.file_type == PlayFileType::Dex && !include_dex {
                    continue;
                }
                let input_path = zip_output_dir.join(&play_file.name);
                zip.start_file(&play_file.name, options)?;
                let input_file = File::open(input_path)?;
                let mut input = BufReader::new(input_file);
                std::io::copy(&mut input, &mut zip)?;
            }
            zip.start_file("manifest.json", options)?;
            zip.write_all(&manifest_json)?;

            if let Some(icon_data) = icon_data {
                zip.start_file("icon.png", options)?;
                zip.write_all(&icon_data)?;
            }
            zip.finish()?;
            Ok(())
        })
        .await
        .map_err(|e| std::io::Error::other(format!("zip task failed: {e}")))?;

        result?;

        let mut cleanup_failures = 0_usize;
        let input_file_count = files.iter().filter(|file| include_dex || file.file_type != PlayFileType::Dex).count();
        for play_file in files.iter().filter(|file| include_dex || file.file_type != PlayFileType::Dex) {
            if tokio::fs::remove_file(output_dir.join(&play_file.name)).await.is_err() {
                cleanup_failures += 1;
            }
        }
        if tokio::fs::remove_file(&manifest_path).await.is_err() {
            cleanup_failures += 1;
        }

        if cleanup_failures > 0 {
            warn!(cleanup_failures, "xapk cleanup incomplete");
        }
        debug!(input_file_count, "xapk created");
        Ok(path_to_return)
    }

    fn extract_icon_from_apk(apk_data: Vec<u8>) -> Option<Vec<u8>> {
        let icon_paths = [
            "res/mipmap-xxxhdpi-v4/app_icon.png",
            "res/mipmap-xxhdpi-v4/app_icon.png",
            "res/mipmap-xhdpi-v4/app_icon.png",
            "res/mipmap-hdpi-v4/app_icon.png",
            "res/mipmap-mdpi-v4/app_icon.png",
            "res/mipmap-ldpi-v4/app_icon.png",
        ];

        let reader = std::io::Cursor::new(apk_data);
        let mut zip = zip::ZipArchive::new(reader).ok()?;

        for icon_path in &icon_paths {
            if let Ok(mut file) = zip.by_name(icon_path) {
                let mut icon_data = Vec::with_capacity(file.size() as usize);
                std::io::copy(&mut file, &mut icon_data).ok()?;
                return Some(icon_data);
            }
        }
        None
    }

    fn make_xapk_manifest(&self, app_details: DetailsResponse, files: Vec<PlayFile>, include_dex: bool) -> Result<Value, PlayError> {
        let item = app_details.item.ok_or_else(|| PlayError::MissingFieldError("item".to_owned()))?;
        let details = item.details.and_then(|details| details.app_details).ok_or_else(|| PlayError::MissingAppDetails)?;

        let package_name = details.package_name.ok_or_else(|| PlayError::MissingFieldError("package_name".to_owned()))?;
        let title = item.title.unwrap_or("".to_owned());
        let version_code = details.version_code.ok_or_else(|| PlayError::MissingFieldError("version_code".to_owned()))?;
        let target_sdk_version = details.target_sdk_version.ok_or_else(|| PlayError::MissingFieldError("target_sdk_version".to_owned()))?;

        let mut total_size = 0;
        let mut split_apks = Vec::new();
        let mut expansions = Vec::new();

        for file in files {
            match file.file_type {
                PlayFileType::Base => {
                    total_size += file.size;
                    split_apks.push(serde_json::json!({"file": file.name, "id": "base"}));
                }
                PlayFileType::Obb | PlayFileType::Patch => {
                    total_size += file.size;
                    expansions.push(serde_json::json!({
                        "file": file.name,
                        "install_location": "EXTERNAL_STORAGE",
                        "install_path": format!("Android/obb/{package_name}/{}", file.name)
                    }));
                }
                PlayFileType::Split => {
                    total_size += file.size;
                    split_apks.push(serde_json::json!({
                        "file": file.name,
                        "id": file.name.strip_suffix(".apk").unwrap_or(&file.name)
                    }));
                }
                // I don't think XAPK manifest has a standard way to include DEX files?
                // https://openxapkfile.net/manifest.html
                PlayFileType::Dex if include_dex => {
                    warn!("dex file omitted from xapk manifest");
                }
                _ => {}
            }
        }

        Ok(serde_json::json!({
            "xapk_version": "2",
            "package_name": package_name,
            "name": title,
            "locales_name": {},
            "version_code": version_code.to_owned(),
            "version_name": details.version_string,
            "min_sdk_version": "24",
            "target_sdk_version": target_sdk_version.to_owned(),
            "permissions": details.permission,
            "total_size": total_size,
            "split_apks": split_apks,
            "expansions": expansions,
        }))
    }

    fn parse_delivery_response(&self, package_name: &str, version_code: i64, response: DeliveryResponse) -> Result<Vec<PlayFile>, PlayError> {
        let delivery_data = response.app_delivery_data.ok_or(PlayError::EmptyDownloads)?;
        let mut files = Vec::new();
        if let Some(url) = delivery_data.download_url.filter(|url| !url.is_empty()) {
            let size = delivery_data.download_size.ok_or_else(|| PlayError::MissingFieldError("app_delivery_data.download_size".to_owned()))?;
            files.push(PlayFile { name: format!("{package_name}.apk"), url, size, file_type: PlayFileType::Base });
        }
        for additional in delivery_data.additional_file {
            let Some(url) = additional.download_url.filter(|url| !url.is_empty()) else { continue };
            let size = additional.download_size.ok_or_else(|| PlayError::MissingFieldError("additional_file.download_size".to_owned()))?;
            let file_version = additional.version_code.filter(|version| *version != 0).map(i64::from).unwrap_or(version_code);
            let (file_type, suffix) =
                if additional.file_type.unwrap_or_default() == 0 { (PlayFileType::Obb, "main") } else { (PlayFileType::Patch, "patch") };
            files.push(PlayFile { name: format!("{suffix}.{file_version}.{package_name}.obb"), url, size, file_type });
        }
        for split in delivery_data.split_delivery_data {
            let Some(url) = split.download_url.filter(|url| !url.is_empty()) else { continue };
            let size = split.download_size.ok_or_else(|| PlayError::MissingFieldError("split_delivery_data.download_size".to_owned()))?;
            let name = split.name.ok_or_else(|| PlayError::MissingFieldError("split_delivery_data.name".to_owned()))?;
            files.push(PlayFile { name: format!("{name}.apk"), url, size, file_type: PlayFileType::Split });
            if let Some(metadata) = split.dex_metadata {
                if let Some(url) = metadata.download_url.filter(|url| !url.is_empty()) {
                    let size = metadata
                        .download_size
                        .ok_or_else(|| PlayError::MissingFieldError("split_delivery_data.dex_metadata.download_size".to_owned()))?;
                    files.push(PlayFile { name: format!("{name}.dm"), url, size, file_type: PlayFileType::Dex });
                }
            }
        }
        if let Some(metadata) = delivery_data.dex_metadata {
            if let Some(url) = metadata.download_url.filter(|url| !url.is_empty()) {
                let size = metadata.download_size.ok_or_else(|| PlayError::MissingFieldError("dex_metadata.download_size".to_owned()))?;
                files.push(PlayFile { name: "base.dm".to_owned(), url, size, file_type: PlayFileType::Dex });
            }
        }
        if !files.iter().any(|file| file.file_type != PlayFileType::Dex) {
            return Err(PlayError::EmptyDownloads);
        }
        Ok(files)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DeliveryRequest, DownloadAppRequest, GooglePlayApi, PatchFormat, PlayFile, PlayFileType, fdfe_url, list_response, validate_download_size,
    };
    use crate::error::PlayError;
    use crate::playproto::{
        AndroidAppDeliveryData, AppFileMetadata, DeliveryResponse, Item, ListResponse, Payload, PreFetch, ResponseWrapper, SplitDeliveryData,
    };
    use prost::Message;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::Path;
    use std::thread;
    use std::time::Duration;

    fn api() -> GooglePlayApi {
        GooglePlayApi::new_from_gsf_id("test-device-id", "test-auth-token", None::<String>, None).unwrap()
    }

    fn server(status: u16, body: Vec<u8>) -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut received = Vec::new();
            let mut buffer = [0; 4096];
            loop {
                let count = stream.read(&mut buffer).unwrap();
                received.extend_from_slice(&buffer[..count]);
                if count == 0 || received.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let header = format!("HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            stream.write_all(header.as_bytes()).unwrap();
            stream.write_all(&body).unwrap();
            String::from_utf8(received).unwrap()
        });
        (format!("http://{address}/fdfe/purchase"), handle)
    }

    #[test]
    fn play_file_debug_output_redacts_the_url() {
        let file = PlayFile {
            name: "base.apk".to_owned(),
            url: "https://example.invalid/base.apk?token=secret".to_owned(),
            size: 10,
            file_type: PlayFileType::Base,
        };

        let rendered = format!("{file:?}");
        assert!(!rendered.contains("token=secret"));
        assert!(rendered.contains("<redacted>"));
    }

    #[test]
    fn download_size_mismatches_fail() {
        let error = validate_download_size(10, 9).expect_err("size mismatch should fail");
        assert!(error.to_string().contains("expected 10 bytes, got 9"));
    }

    #[test]
    fn negative_download_sizes_fail() {
        let error = validate_download_size(-1, 0).expect_err("negative size should fail");
        assert!(error.to_string().contains("invalid download size: -1"));
    }

    #[test]
    fn delivery_patch_parameters_are_numeric_repeated_and_gated_by_installed_version() {
        let mut request = DeliveryRequest::new("com.example", 42, 1);
        request.patch_formats = vec![PatchFormat::Gdiff, PatchFormat::GzippedBsdiff, PatchFormat::Gdiff];
        request.installed_derived_apk_id = Some(7);
        request.installed_frosting_id = Some(19);
        request.delivery_token = Some(String::new());
        assert!(!request.query().iter().any(|(key, _)| ["pf", "bda", "bf", "dtok"].contains(&key.as_str())));
        request.installed_version_code = Some(40);
        let query = request.query();
        assert_eq!(query.iter().filter(|(key, _)| key == "pf").map(|(_, value)| value.as_str()).collect::<Vec<_>>(), ["1", "3"]);
        assert!(query.contains(&("bvc".into(), "40".into())));
        assert!(query.contains(&("bda".into(), "7".into())));
        assert!(query.contains(&("bf".into(), "19".into())));
        request.installed_derived_apk_id = Some(0);
        request.installed_frosting_id = Some(0);
        assert!(!request.query().iter().any(|(key, _)| ["bda", "bf"].contains(&key.as_str())));
        request.delivery_token = Some("private-delivery-token".to_owned());
        assert!(!format!("{request:?}").contains("private-delivery-token"));
    }

    #[test]
    fn download_options_reach_the_delivery_query() {
        let request = DownloadAppRequest::new("com.example", Path::new("downloads"))
            .certificate_hash("certificate")
            .split_module("feature")
            .installed_version_code(40)
            .installed_derived_apk_id(7)
            .installed_frosting_id(19)
            .patch_format(PatchFormat::GzippedGdiff)
            .patch_formats(vec![PatchFormat::Gdiff, PatchFormat::GzippedBsdiff, PatchFormat::Gdiff]);
        let query = request.delivery_request(42, 2).query();

        for (key, value) in
            [("doc", "com.example"), ("vc", "42"), ("ot", "2"), ("ch", "certificate"), ("mn", "feature"), ("bvc", "40"), ("bda", "7"), ("bf", "19")]
        {
            assert!(query.contains(&(key.to_owned(), value.to_owned())));
        }
        assert_eq!(query.iter().filter(|(key, _)| key == "pf").map(|(_, value)| value.as_str()).collect::<Vec<_>>(), ["1", "3"]);
    }

    #[test]
    fn download_patch_formats_preserve_single_format_and_explicit_empty_selection() {
        let request = DownloadAppRequest::new("com.example", Path::new("downloads")).installed_version_code(40).patch_format(PatchFormat::Gdiff);
        assert!(request.delivery_request(42, 1).query().contains(&("pf".into(), "1".into())));

        let request = request.patch_formats(Vec::new());
        assert!(!request.delivery_request(42, 1).query().iter().any(|(key, _)| key == "pf"));

        let request = request.patch_format(PatchFormat::GzippedBsdiff);
        assert!(request.delivery_request(42, 1).query().contains(&("pf".into(), "3".into())));
    }

    #[test]
    fn stream_cursors_resolve_without_losing_query_parameters() {
        for cursor in ["search?cursor=abc%2B123", "/fdfe/search?cursor=abc%2B123", "https://android.clients.google.com/fdfe/search?cursor=abc%2B123"]
        {
            assert_eq!(fdfe_url(cursor).unwrap(), "https://android.clients.google.com/fdfe/search?cursor=abc%2B123");
        }
        assert!(fdfe_url("https://example.invalid/fdfe/search").is_err());
    }

    #[test]
    fn stream_lists_prefer_prefetch_and_fall_back_to_top_level() {
        let top = ListResponse { item: Some(Item { title: Some("top".into()), ..Default::default() }), ..Default::default() };
        let prefetched = ListResponse { item: Some(Item { title: Some("prefetch".into()), ..Default::default() }), ..Default::default() };
        let mut response = ResponseWrapper { payload: Some(Payload { list_response: Some(top), ..Default::default() }), ..Default::default() };
        assert_eq!(list_response(response.clone()).unwrap().item.unwrap().title.as_deref(), Some("top"));
        response.pre_fetch = Some(Box::new(PreFetch {
            response: Some(Box::new(ResponseWrapper {
                payload: Some(Payload { list_response: Some(prefetched), ..Default::default() }),
                ..Default::default()
            })),
            ..Default::default()
        }));
        assert_eq!(list_response(response).unwrap().item.unwrap().title.as_deref(), Some("prefetch"));
    }

    #[test]
    fn obb_types_and_their_own_versions_are_preserved() {
        let response = DeliveryResponse {
            status: Some(1),
            app_delivery_data: Some(AndroidAppDeliveryData {
                additional_file: vec![
                    AppFileMetadata {
                        file_type: Some(0),
                        version_code: Some(40),
                        download_size: Some(10),
                        download_url: Some("https://example.invalid/main.obb".into()),
                        ..Default::default()
                    },
                    AppFileMetadata {
                        file_type: Some(1),
                        version_code: Some(0),
                        download_size: Some(20),
                        download_url: Some("https://example.invalid/patch.obb".into()),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }),
            ..Default::default()
        };
        let files = api().parse_delivery_response("com.example", 42, response).unwrap();
        assert_eq!(files[0].name, "main.40.com.example.obb");
        assert_eq!(files[0].file_type, PlayFileType::Obb);
        assert_eq!(files[1].name, "patch.42.com.example.obb");
        assert_eq!(files[1].file_type, PlayFileType::Patch);
    }

    #[test]
    fn split_only_delivery_is_supported_but_empty_delivery_fails() {
        let response = DeliveryResponse {
            status: Some(1),
            app_delivery_data: Some(AndroidAppDeliveryData {
                split_delivery_data: vec![SplitDeliveryData {
                    name: Some("config.en".into()),
                    download_size: Some(1),
                    download_url: Some("https://example.invalid/config.en.apk".into()),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        let files = api().parse_delivery_response("com.example", 42, response).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, "config.en.apk");
        assert!(matches!(api().parse_delivery_response("com.example", 42, DeliveryResponse::default()), Err(PlayError::EmptyDownloads)));
    }

    #[tokio::test]
    async fn rejected_purchase_only_falls_back_for_http_400_and_sends_query_without_body() {
        for status in [400, 401, 403, 429, 503] {
            let (url, received) = server(status, Vec::new());
            let result = api().delivery_token_from(&url, "com.example", 42, 1, Some("certificate")).await;
            if status == 400 {
                assert_eq!(result.unwrap(), "");
            } else {
                assert_eq!(result.unwrap_err().http_status().unwrap().as_u16(), status);
            }
            let request = received.join().unwrap();
            assert!(request.starts_with("POST /fdfe/purchase?ot=1&doc=com.example&vc=42&ch=certificate HTTP/1.1"));
            assert!(request.ends_with("\r\n\r\n"));
            assert!(!request.to_lowercase().contains("content-type: application/x-www-form-urlencoded"));
        }
    }

    #[tokio::test]
    async fn purchase_without_buy_response_uses_no_delivery_token() {
        let (url, received) = server(200, ResponseWrapper::default().encode_to_vec());
        assert_eq!(api().delivery_token_from(&url, "com.example", 42, 1, None).await.unwrap(), "");
        received.join().unwrap();
    }

    #[tokio::test]
    async fn split_only_xapk_contains_only_existing_apks_and_no_phantom_icon() {
        use crate::playproto::{AppDetails, DetailsResponse, DocumentDetails};
        let output_dir = std::env::temp_dir().join(format!("playrs-split-xapk-{}", rand::random::<u64>()));
        std::fs::create_dir(&output_dir).unwrap();
        // An unrelated path with the former hard-coded base name must never be read.
        std::fs::create_dir(output_dir.join("com.example.apk")).unwrap();
        let files = ["config.en.apk", "config.arm64.apk"]
            .into_iter()
            .map(|name| {
                std::fs::write(output_dir.join(name), b"apk").unwrap();
                PlayFile { name: name.to_owned(), url: "https://example.invalid/split.apk".to_owned(), size: 3, file_type: PlayFileType::Split }
            })
            .collect();
        let details = DetailsResponse {
            item: Some(Item {
                title: Some("Example".to_owned()),
                details: Some(DocumentDetails {
                    app_details: Some(AppDetails {
                        package_name: Some("com.example".to_owned()),
                        version_code: Some(42),
                        target_sdk_version: Some(35),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let path = api().make_xapk("com.example", details, files, &output_dir, None, false).await.unwrap();
        let mut archive = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
        assert!(archive.by_name("config.en.apk").is_ok());
        assert!(archive.by_name("config.arm64.apk").is_ok());
        assert!(archive.by_name("com.example.apk").is_err());
        assert!(archive.by_name("icon.png").is_err());
        let mut manifest = String::new();
        archive.by_name("manifest.json").unwrap().read_to_string(&mut manifest).unwrap();
        let manifest: serde_json::Value = serde_json::from_str(&manifest).unwrap();
        let apks = manifest["split_apks"].as_array().unwrap();
        assert_eq!(apks.len(), 2);
        assert!(apks.iter().all(|apk| apk["id"] != "base"));
        assert!(manifest.get("icon").is_none());
        assert_eq!(manifest["total_size"], 6);
        drop(archive);
        std::fs::remove_dir_all(output_dir).unwrap();
    }
}
