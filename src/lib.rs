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
mod playproto;

use auth::AuthData;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use constants::{
    CategoryType, ClusterType, LEGACY_USER_AGENT, ModifyWishlistAction, PatchFormat, PlayFileType, ReviewFilter, StreamCategory, StreamType,
    URL_ACQUIRE, URL_BULK_DETAILS, URL_CATEGORIES, URL_DELIVERY, URL_DETAILS, URL_FDFE, URL_MODIFY_LIBRARY, URL_PURCHASE, URL_PURCHASE_HISTORY,
    URL_REVIEW_ADD_EDIT, URL_REVIEW_DELETE, URL_REVIEWS, URL_SEARCH, URL_SEARCH_SUGGEST, URL_TESTING_PROGRAM, URL_TOP_CHART, URL_USER_PROFILE,
};
use device::Device;
use error::PlayError;
use http::get_payload_field;
use indexmap::IndexMap;
use playproto::{
    AcquireRequest, AcquireResponseWrapper, BrowseResponse, BulkDetailsEntry, BulkDetailsRequest, DeliveryResponse, DetailsResponse, Field, Item,
    ListResponse, ModifyLibraryRequest, ResponseWrapperApi, Review, ReviewResponse, SearchSuggestEntry, TestingProgramRequest,
    TestingProgramResponse, UserProfile,
    acquire_request::package::Payload as AcquirePayload,
    acquire_request::{Message30, Package, Version},
};
use prost::Message;
use reqwest::{
    Client,
    header::{HeaderName, HeaderValue},
};
use serde::Serialize;
use serde_json::Value;
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use tokio::task;
use tracing::{debug, info, warn};
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

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

#[derive(Debug, Clone)]
pub struct PlayFile {
    name: String,
    url: String,
    size: i64,
    file_type: PlayFileType,
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
}

// download_app_request is a structured alias to download_app that uses this instead of a bunch of params
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
        let headers = self.get_default_headers()?;
        let response = self.client.get(URL_USER_PROFILE).headers(headers).send().await?;

        if response.status().is_success() {
            let bytes = response.bytes().await?;
            let wrapper = ResponseWrapperApi::decode(bytes.as_ref())?;
            let user_profile_response = get_payload_field!(wrapper, user_profile_response)?;
            return Ok(user_profile_response.user_profile);
        }

        Ok(None)
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
        let url = format!("{URL_FDFE}/getDeveloperPageStream?docid=developer-{dev_id}");
        let response = self.get_play_wrapper_resp(&url, headers, None).await?;
        get_payload_field!(response, list_response)
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
        let url = format!("{URL_FDFE}/{stream_or_next_url}");
        let response = self.get_play_wrapper_resp(&url, headers, None).await?;

        if let Some(pf) = response.pre_fetch.and_then(|pf| pf.response).and_then(|r| r.payload).and_then(|p| p.list_response) {
            return Ok(pf);
        }
        get_payload_field!(response, list_response)
    }

    pub async fn get_my_apps(&self, cluster_type: ClusterType) -> Result<ListResponse, PlayError> {
        let headers = self.get_default_headers()?;
        let params = IndexMap::from([("n".to_owned(), "15".to_owned()), ("tab".to_owned(), cluster_type.to_string())]);
        let url = format!("{URL_FDFE}/myAppsStream");
        let response = self.get_play_wrapper_resp(&url, headers, Some(params)).await?;
        get_payload_field!(response, list_response)
    }

    pub async fn get_next_stream_response(&self, next_url: &str) -> Result<ListResponse, PlayError> {
        let headers = self.get_default_headers()?;
        let url = format!("{URL_FDFE}/{next_url}");
        let response = self.get_play_wrapper_resp(&url, headers, None).await?;
        get_payload_field!(response, list_response)
    }

    pub async fn get_browse_response(&self, browse_url: &str) -> Result<BrowseResponse, PlayError> {
        let headers = self.get_default_headers()?;
        let url = format!("{URL_FDFE}/{browse_url}");
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
        let headers = self.get_default_headers()?;
        let request = match action {
            ModifyWishlistAction::Add => {
                ModifyLibraryRequest { library_id: Some("u-wl".to_owned()), add_package_name: package_names, remove_package_name: vec![] }
            }
            ModifyWishlistAction::Remove => {
                ModifyLibraryRequest { library_id: Some("u-wl".to_owned()), add_package_name: vec![], remove_package_name: package_names }
            }
        };
        let resp = self.client.post(URL_MODIFY_LIBRARY).headers(headers).body(request.encode_to_vec()).send().await?;
        Ok(resp.status().is_success())
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
        if let Some(user_reviews_response) = review_response.user_reviews_response {
            if let Some(review) = user_reviews_response.review.into_iter().next() {
                return Ok(review);
            }
        }
        Err(PlayError::ReviewError)
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
        let url = format!("{URL_FDFE}/{next_page_url}");
        let response = self.get_play_wrapper_resp(&url, headers, None).await?;
        get_payload_field!(response, review_response)
    }

    pub async fn search_suggestions(&self, query: &str) -> Result<Vec<SearchSuggestEntry>, PlayError> {
        let mut headers = self.get_default_headers()?;
        headers.insert(HeaderName::from_static("user-agent"), HeaderValue::from_static(LEGACY_USER_AGENT));
        let params = IndexMap::from([
            ("q".to_owned(), query.to_owned()),
            ("sb".to_owned(), "5".to_owned()),
            ("sst".to_owned(), "2".to_owned()),
            ("sdt".to_owned(), "3".to_owned()),
        ]);
        let response = self.get_play_wrapper_resp(URL_SEARCH_SUGGEST, headers, Some(params)).await?;
        let search_suggest_response = get_payload_field!(response, search_suggest_response)?;
        Ok(search_suggest_response.entry)
    }

    pub async fn search_results(&self, query: &str, next_page_url: Option<&str>) -> Result<ListResponse, PlayError> {
        let mut headers = self.get_default_headers()?;
        headers.insert(HeaderName::from_static("user-agent"), HeaderValue::from_static(LEGACY_USER_AGENT));
        let params = IndexMap::from([("q".to_owned(), query.to_owned()), ("c".to_owned(), "3".to_owned()), ("ksm".to_owned(), "1".to_owned())]);
        let url = if let Some(next_url) = next_page_url { format!("{URL_FDFE}/{}", next_url) } else { URL_SEARCH.to_owned() };
        let response = self.get_play_wrapper_resp(&url, headers, Some(params)).await?;

        if let Some(pf) = response.pre_fetch.and_then(|pf| pf.response).and_then(|r| r.payload).and_then(|p| p.list_response) {
            return Ok(pf);
        }
        get_payload_field!(response, list_response)
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
        get_payload_field!(response, list_response)
    }

    pub async fn get_cluster(&self, category: &str, chart: &str) -> Result<Option<ListResponse>, PlayError> {
        let mut headers = self.get_default_headers()?;
        headers.insert(HeaderName::from_static("user-agent"), HeaderValue::from_static(LEGACY_USER_AGENT));
        let params =
            IndexMap::from([("c".to_owned(), "3".to_owned()), ("stcid".to_owned(), chart.to_owned()), ("scat".to_owned(), category.to_owned())]);
        match self.get_play_wrapper_resp(URL_TOP_CHART, headers, Some(params)).await {
            Ok(response) => get_payload_field!(response, list_response).map(Some),
            Err(PlayError::HttpStatusError(status)) if status.is_client_error() => Ok(None),
            Err(e) => Err(e),
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
        let headers = self.get_default_headers()?;
        let mut params = IndexMap::from([
            ("ot".to_owned(), offer_type.to_string()),
            ("doc".to_owned(), package_name.to_owned()),
            ("vc".to_owned(), version_code.to_string()),
        ]);
        if let Some(ch) = certificate_hash {
            params.insert("ch".to_owned(), ch.to_string());
        }
        let response = self.post_play_wrapper_resp(URL_PURCHASE, headers, Some(params), None).await?;
        let buy_response = get_payload_field!(response, buy_response)?;
        buy_response.encoded_delivery_token.ok_or_else(|| PlayError::MissingFieldError("encoded_delivery_token".to_owned()))
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
        let headers = self.get_default_headers()?;
        let mut params = IndexMap::from([
            ("ot".to_owned(), offer_type.to_string()),
            ("doc".to_owned(), package_name.to_owned()),
            ("vc".to_owned(), update_version_code.to_string()),
        ]);

        if let Some(ivc) = installed_version_code {
            if ivc > 0 {
                params.insert("bvc".to_owned(), ivc.to_string());
                params.insert("pf".to_owned(), patch_format.to_string());
            }
        }
        if let Some(sm) = split_module {
            params.insert("mn".to_owned(), sm.to_owned());
        }
        if let Some(ch) = certificate_hash {
            params.insert("ch".to_owned(), ch.to_owned());
        }
        if let Some(dt) = delivery_token {
            params.insert("dtok".to_owned(), dt.to_owned());
        }

        let response = self.get_play_wrapper_resp(URL_DELIVERY, headers, Some(params)).await?;
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
            nonce: Some(format!("nonce={}", URL_SAFE_NO_PAD.encode(&rand::random::<[u8; 32]>()))),
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
        // https://gitlab.com/AuroraOSS/gplayapi/-/commit/2180d4a50efac9b86f83e2bfb84db3366d41476d
        // dont care if this fails
        if let Err(e) = self.acquire(&package_name, version_code, offer_type).await {
            debug!("Acquire failed (non-fatal): {e}");
        }

        let delivery_token = self.get_delivery_token(&package_name, version_code, offer_type, certificate_hash).await?;

        let delivery_response = self
            .get_delivery_response(
                &package_name,
                version_code,
                offer_type,
                split_module,
                installed_version_code,
                patch_format,
                Some(&delivery_token),
                certificate_hash,
            )
            .await?;

        match delivery_response.status {
            Some(1) => Ok(delivery_response),
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
        info!("Downloading {package_name}...");

        if output_dir.is_file() {
            return Err(PlayError::InvalidOutputDirectory(output_dir.to_path_buf()));
        }
        tokio::fs::create_dir_all(output_dir).await?;

        let details = self.get_app_details_by_package_name(&package_name).await?;
        let app_details =
            details.item.clone().and_then(|item| item.details).and_then(|details| details.app_details).ok_or_else(|| PlayError::MissingAppDetails)?;

        let vc = version_code.unwrap_or(app_details.version_code.ok_or_else(|| PlayError::MissingFieldError("version_code".to_owned()))?);
        let ot =
            offer_type.unwrap_or(details.item.clone().and_then(|item| item.offer.into_iter().next()).and_then(|offer| offer.offer_type).unwrap_or(1));

        let delivery_response = self.purchase(&package_name, vc, ot, certificate_hash, split_module, installed_version_code, patch_format).await?;

        let files = self.parse_delivery_response(&package_name, delivery_response)?;

        let mut downloaded_paths = Vec::new();
        if files.len() == 1 {
            let file = &files[0];
            let data = self.client.get(&file.url).send().await?.bytes().await?;
            let output_path = output_dir.join(custom_apk_name.clone().unwrap_or_else(|| file.name.clone()));
            tokio::fs::write(&output_path, &data).await?;
            downloaded_paths.push(output_path);
        } else {
            for file in files.iter() {
                if file.file_type == PlayFileType::Dex && !include_dex {
                    continue;
                }
                let data = self.client.get(&file.url).send().await?.bytes().await?;
                let output_path = output_dir.join(&file.name);
                tokio::fs::write(&output_path, &data).await?;
                downloaded_paths.push(output_path);
            }
            if use_xapk {
                let xapk_path = self.make_xapk(package_name, details, files, output_dir, custom_apk_name, include_dex).await?;
                downloaded_paths.clear();
                downloaded_paths.push(xapk_path);
            }
        }

        info!("Download complete for {package_name}: {} file(s)", downloaded_paths.len());
        debug!("Downloaded files: {:#?}", downloaded_paths);
        Ok(downloaded_paths)
    }

    pub async fn download_app_request(&self, request: DownloadAppRequest) -> Result<Vec<PathBuf>, PlayError> {
        self.download_app(
            &request.package_name,
            &request.output_dir,
            request.version_code,
            request.offer_type,
            request.certificate_hash.as_deref(),
            request.split_module.as_deref(),
            request.installed_version_code,
            request.patch_format,
            request.use_xapk,
            request.custom_apk_name,
            request.include_dex,
        )
        .await
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

        let manifest = self.make_xapk_manifest(details, files.clone(), include_dex)?;
        let manifest_path = output_dir.join("manifest.json");
        let manifest_json = serde_json::to_vec_pretty(&manifest)?;
        tokio::fs::write(&manifest_path, &manifest_json).await?;

        let xapk_name = custom_apk_name.unwrap_or_else(|| package_name.clone());
        let xapk_path = output_dir.join(format!("{xapk_name}.xapk"));
        let path_to_return = xapk_path.clone();

        let icon_data = match tokio::fs::read(output_dir.join(format!("{package_name}.apk"))).await {
            Ok(base_apk) => Self::extract_icon_from_apk(base_apk),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };

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

        for play_file in &files {
            let _ = tokio::fs::remove_file(output_dir.join(&play_file.name)).await;
        }
        let _ = tokio::fs::remove_file(&manifest_path).await;

        result?;

        info!("XAPK created: {}", path_to_return.display());
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
        let details =
            app_details.item.and_then(|item| item.details).and_then(|details| details.app_details).ok_or_else(|| PlayError::MissingAppDetails)?;

        let package_name = details.package_name.ok_or_else(|| PlayError::MissingFieldError("package_name".to_owned()))?;
        let title = details.title.unwrap_or("".to_owned());
        let version_code = details.version_code.ok_or_else(|| PlayError::MissingFieldError("version_code".to_owned()))?;
        let target_sdk_version = details.target_sdk_version.ok_or_else(|| PlayError::MissingFieldError("target_sdk_version".to_owned()))?;

        let mut total_size = 0;
        let mut split_apks = vec![serde_json::json!({
            "file": format!("{package_name}.apk"),
            "id": "base"
        })];
        let mut expansions = Vec::new();

        for file in files {
            match file.file_type {
                PlayFileType::Base => {
                    total_size += file.size;
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
                    warn!("Cannot include DEX files in XAPK manifest");
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
            "icon": "icon.png",
            "split_apks": split_apks,
            "expansions": expansions,
        }))
    }

    fn parse_delivery_response(&self, package_name: &str, response: DeliveryResponse) -> Result<Vec<PlayFile>, PlayError> {
        let Some(delivery_data) = response.app_delivery_data else {
            return Err(PlayError::MissingFieldError("app_delivery_data".into()));
        };

        let mut files = Vec::new();

        let Some(download_url) = delivery_data.download_url else {
            return Err(PlayError::MissingFieldError("app_delivery_data.download_url".into()));
        };
        let Some(download_size) = delivery_data.download_size else {
            return Err(PlayError::MissingFieldError("app_delivery_data.download_size".into()));
        };

        files.push(PlayFile { name: format!("{package_name}.apk"), url: download_url, size: download_size, file_type: PlayFileType::Base });

        for additional_file in delivery_data.additional_file.iter() {
            let Some(download_url) = additional_file.download_url.clone() else {
                return Err(PlayError::MissingFieldError("additional_file.download_url".into()));
            };
            let Some(download_size) = additional_file.download_size else {
                return Err(PlayError::MissingFieldError("additional_file.download_size".into()));
            };
            let Some(raw_file_type) = additional_file.file_type else {
                return Err(PlayError::MissingFieldError("additional_file.file_type".into()));
            };
            let Some(version_code) = additional_file.version_code else {
                return Err(PlayError::MissingFieldError("additional_file.version_code".into()));
            };

            let (file_type, name_suffix) = if raw_file_type == 1 { (PlayFileType::Obb, "main") } else { (PlayFileType::Patch, "patch") };
            let name = format!("{name_suffix}.{version_code}.{package_name}.obb");

            files.push(PlayFile { name, url: download_url, size: download_size, file_type });
        }

        for split in &delivery_data.split_delivery_data {
            let Some(download_url) = split.download_url.clone() else {
                return Err(PlayError::MissingFieldError("split_delivery_data.download_url".into()));
            };
            let Some(download_size) = split.download_size else {
                return Err(PlayError::MissingFieldError("split_delivery_data.download_size".into()));
            };
            let Some(ref name) = split.name else {
                return Err(PlayError::MissingFieldError("split_delivery_data.name".into()));
            };

            let filename = format!("{name}.apk");
            files.push(PlayFile { name: filename, url: download_url, size: download_size, file_type: PlayFileType::Split });
        }

        if let Some(ref dex_metadata) = delivery_data.dex_metadata {
            let Some(download_url) = dex_metadata.download_url.clone() else {
                return Err(PlayError::MissingFieldError("dex_metadata.download_url".into()));
            };
            let Some(download_size) = dex_metadata.download_size else {
                return Err(PlayError::MissingFieldError("dex_metadata.download_size".into()));
            };

            files.push(PlayFile { name: "base.dm".to_owned(), url: download_url, size: download_size, file_type: PlayFileType::Dex });
        }

        Ok(files)
    }
}
