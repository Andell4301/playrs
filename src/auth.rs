// SPDX-FileCopyrightText: 2020-2026 Aurora OSS
// SPDX-FileCopyrightText: 2023-2025 The Calyx Institute
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::GooglePlayApi;

use crate::constants::{
    DEFAULT_ANDROID_VENDING_APP, DEFAULT_ANDROID_VENDING_PACKAGE, DEFAULT_CALLER_SIG, DEFAULT_CLIENT_SIG, DEFAULT_DFE_PHENOTYPE, DEFAULT_DFE_TARGETS,
    TokenService, URL_AUTH, URL_CHECK_IN, URL_TOC, URL_TOS_ACCEPT, URL_UPLOAD_DEVICE_CONFIG,
};
use crate::error::PlayError;
use crate::http::get_payload_field;
use crate::playproto::{AcceptTosResponse, AndroidCheckinResponse, TocResponse, UploadDeviceConfigRequest, UploadDeviceConfigResponse};
use indexmap::IndexMap;
use prost::Message;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use std::fmt;
use tracing::{debug, info, instrument};

#[derive(Clone, Default)]
pub struct AuthData {
    pub(crate) email: Option<String>,
    pub(crate) aas_token: Option<String>,
    pub(crate) gsf_id: Option<String>,
    pub(crate) auth_token: Option<String>,
    pub(crate) oauth_login_token: Option<String>,
    pub(crate) ac2dm_token: Option<String>,
    pub(crate) android_check_in_token: Option<String>,
    pub(crate) device_check_in_consistency_token: Option<String>,
    pub(crate) device_config_token: Option<String>,
    pub(crate) experiments_config_token: Option<String>,
    pub(crate) gcm_token: Option<String>,
    pub(crate) dfe_cookie: Option<String>,
}

const TOKEN_PREVIEW_EDGE_CHARS: usize = 3;
const TOKEN_PREVIEW_MIN_CHARS: usize = 12;

fn token_preview(value: &str) -> String {
    let length = value.chars().count();
    if length < TOKEN_PREVIEW_MIN_CHARS || !value.bytes().all(|byte| byte.is_ascii_graphic()) {
        return "<redacted>".to_string();
    }

    let prefix: String = value.chars().take(TOKEN_PREVIEW_EDGE_CHARS).collect();
    let suffix: String = value.chars().skip(length - TOKEN_PREVIEW_EDGE_CHARS).collect();
    format!("{prefix}…{suffix}")
}

impl fmt::Debug for AuthData {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn redacted(value: &Option<String>) -> Option<&'static str> {
            value.as_ref().map(|_| "<redacted>")
        }

        fn preview(value: &Option<String>) -> Option<String> {
            value.as_deref().map(token_preview)
        }

        formatter
            .debug_struct("AuthData")
            .field("email", &redacted(&self.email))
            .field("aas_token", &preview(&self.aas_token))
            .field("gsf_id", &redacted(&self.gsf_id))
            .field("auth_token", &preview(&self.auth_token))
            .field("oauth_login_token", &preview(&self.oauth_login_token))
            .field("ac2dm_token", &preview(&self.ac2dm_token))
            .field("android_check_in_token", &preview(&self.android_check_in_token))
            .field("device_check_in_consistency_token", &preview(&self.device_check_in_consistency_token))
            .field("device_config_token", &preview(&self.device_config_token))
            .field("experiments_config_token", &preview(&self.experiments_config_token))
            .field("gcm_token", &preview(&self.gcm_token))
            .field("dfe_cookie", &redacted(&self.dfe_cookie))
            .finish()
    }
}

fn sensitive_header_value(value: &str) -> Result<HeaderValue, PlayError> {
    let mut value = HeaderValue::from_str(value)?;
    value.set_sensitive(true);
    Ok(value)
}

impl AuthData {
    pub fn get_email(&self) -> Option<&str> {
        self.email.as_deref()
    }
    pub fn get_aas_token(&self) -> Option<&str> {
        self.aas_token.as_deref()
    }
    pub fn get_gsf_id(&self) -> Option<&str> {
        self.gsf_id.as_deref()
    }
    pub fn get_auth_token(&self) -> Option<&str> {
        self.auth_token.as_deref()
    }
    pub fn get_oauth_login_token(&self) -> Option<&str> {
        self.oauth_login_token.as_deref()
    }
    pub fn get_ac2dm_token(&self) -> Option<&str> {
        self.ac2dm_token.as_deref()
    }
    pub fn get_android_check_in_token(&self) -> Option<&str> {
        self.android_check_in_token.as_deref()
    }
    pub fn get_device_check_in_consistency_token(&self) -> Option<&str> {
        self.device_check_in_consistency_token.as_deref()
    }
    pub fn get_device_config_token(&self) -> Option<&str> {
        self.device_config_token.as_deref()
    }
    pub fn get_experiments_config_token(&self) -> Option<&str> {
        self.experiments_config_token.as_deref()
    }
    pub fn get_gcm_token(&self) -> Option<&str> {
        self.gcm_token.as_deref()
    }
    pub fn get_dfe_cookie(&self) -> Option<&str> {
        self.dfe_cookie.as_deref()
    }
    pub fn is_ready(&self) -> bool {
        self.gsf_id.is_some() && self.auth_token.is_some()
    }
}

impl GooglePlayApi {
    pub(crate) async fn generate_aas_token(&self) -> Result<String, PlayError> {
        debug!("requesting AAS token");
        let Some(token) = self.auth_data.oauth_login_token.as_deref() else {
            return Err(PlayError::AuthenticationError("Cannot generate AAS token without OAuth login token".to_string()));
        };

        if self.auth_data.email.is_none() {
            return Err(PlayError::AuthenticationError("Cannot generate AAS token without email".to_string()));
        }

        let mut params = self.get_default_auth_params()?;
        params.extend([
            ("service".to_string(), TokenService::AC2DM.to_string()),
            ("add_account".to_string(), "1".to_string()),
            ("get_accountid".to_string(), "1".to_string()),
            ("ACCESS_TOKEN".to_string(), "1".to_string()),
            ("callerPkg".to_string(), DEFAULT_ANDROID_VENDING_PACKAGE.to_string()),
            ("Token".to_string(), token.to_string()),
            ("droidguard_results".to_string(), "null".to_string()),
        ]);

        let mut headers = self.get_auth_headers()?;
        headers.insert(HeaderName::from_static("app"), HeaderValue::from_str(DEFAULT_ANDROID_VENDING_APP)?);

        let resp = self.post_auth(URL_AUTH, headers, params).await?;
        let bytes = resp.bytes().await?;
        let parsed = Self::parse_auth_response(&bytes);

        parsed
            .get("Token")
            .filter(|token| !token.is_empty())
            .or_else(|| parsed.get("Auth").filter(|token| !token.is_empty()))
            .cloned()
            .ok_or_else(|| PlayError::AuthenticationError("Failed to generate AAS token".to_string()))
    }

    pub(crate) async fn generate_token(&self, service: TokenService) -> Result<String, PlayError> {
        debug!(service = %service, "requesting token");
        let Some(aas_token) = self.auth_data.aas_token.as_deref() else {
            return Err(PlayError::AuthenticationError("AAS token is required to generate service tokens".to_string()));
        };

        let headers = self.get_auth_headers()?;
        let mut params = self.get_default_auth_params()?;
        params.extend([
            ("app".to_string(), DEFAULT_ANDROID_VENDING_APP.to_string()),
            ("client_sig".to_string(), DEFAULT_CLIENT_SIG.to_string()),
            ("callerPkg".to_string(), DEFAULT_ANDROID_VENDING_PACKAGE.to_string()),
            ("Token".to_string(), aas_token.to_string()),
            ("oauth2_foreground".to_string(), "1".to_string()),
            ("token_request_options".to_string(), "CAA4AVAB".to_string()),
            ("check_email".to_string(), "1".to_string()),
            ("system_partition".to_string(), "1".to_string()),
            ("droidguard_results".to_string(), "null".to_string()),
        ]);

        match service {
            TokenService::AC2DM => {
                params.insert("service".to_string(), service.to_string());
                params.shift_remove("app");
            }
            TokenService::AndroidCheckInServer => {
                params.insert("oauth2_foreground".to_string(), "0".to_string());
                params.insert("app".to_string(), DEFAULT_ANDROID_VENDING_PACKAGE.to_string());
                params.insert("service".to_string(), service.to_string());
            }
            TokenService::Oauthlogin => {
                params.insert("oauth2_foreground".to_string(), "0".to_string());
                params.insert("app".to_string(), "com.google.android.googlequicksearchbox".to_string());
                params.insert("service".to_string(), format!("oauth2:https://www.google.com/accounts/{service}"));
                params.insert("callerPkg".to_string(), "com.google.android.googlequicksearchbox".to_string());
            }
            TokenService::ExperimentalConfig => {
                params.insert("service".to_string(), format!("oauth2:https://www.googleapis.com/auth/{service}"));
            }
            TokenService::Numberer | TokenService::GCM => {
                params.insert("app".to_string(), DEFAULT_ANDROID_VENDING_PACKAGE.to_string());
                params.insert("service".to_string(), format!("oauth2:https://www.googleapis.com/auth/{service}"));
            }
            TokenService::GooglePlay => {
                params.insert("service".to_string(), format!("oauth2:https://www.googleapis.com/auth/{service}"));
            }
            TokenService::Android => {
                params.insert("service".to_string(), service.to_string());
            }
        }

        let resp = self.post_auth(URL_AUTH, headers, params).await?;
        let bytes = resp.bytes().await?;
        let parsed = Self::parse_auth_response(&bytes);

        match parsed.get("Auth") {
            Some(token) => Ok(token.to_string()),
            None => Err(PlayError::AuthenticationError(format!("Failed to retrieve token for {service}: 'Auth' field not found in response"))),
        }
    }
}

impl GooglePlayApi {
    pub(crate) fn get_default_headers(&self) -> Result<HeaderMap, PlayError> {
        let Some(gsf_id) = self.auth_data.gsf_id.as_deref() else {
            return Err(PlayError::AuthenticationError("GsfId is required to generate default headers".to_string()));
        };

        let mut headers = HeaderMap::new();

        if let Some(auth_token) = self.auth_data.auth_token.as_deref() {
            headers.insert("Authorization", sensitive_header_value(&format!("Bearer {auth_token}"))?);
        }

        headers.extend([
            (HeaderName::from_static("user-agent"), HeaderValue::from_str(&self.device.user_agent_string())?),
            (HeaderName::from_static("x-dfe-device-id"), sensitive_header_value(gsf_id)?),
            (HeaderName::from_static("accept-language"), HeaderValue::from_str(&self.device.locale().replace('_', "-"))?),
            (HeaderName::from_static("x-dfe-encoded-targets"), HeaderValue::from_static(DEFAULT_DFE_TARGETS)),
            (HeaderName::from_static("x-dfe-phenotype"), HeaderValue::from_static(DEFAULT_DFE_PHENOTYPE)),
            (HeaderName::from_static("x-dfe-client-id"), HeaderValue::from_static("am-android-google")),
            (HeaderName::from_static("x-dfe-network-type"), HeaderValue::from_static("4")),
            (HeaderName::from_static("x-dfe-content-filters"), HeaderValue::from_static("")),
            (HeaderName::from_static("x-limit-ad-tracking-enabled"), HeaderValue::from_static("false")),
            (HeaderName::from_static("x-ad-id"), HeaderValue::from_static("")),
            (HeaderName::from_static("x-dfe-userlanguages"), HeaderValue::from_str(&self.device.user_languages())?),
            (HeaderName::from_static("x-dfe-request-params"), HeaderValue::from_static("timeoutMs=4000")),
        ]);

        if let Some(consistency_token) = self.auth_data.device_check_in_consistency_token.as_deref() {
            headers.insert("x-dfe-device-checkin-consistency-token", sensitive_header_value(consistency_token)?);
        }

        if let Some(device_config_token) = self.auth_data.device_config_token.as_deref() {
            headers.insert("x-dfe-device-config-token", sensitive_header_value(device_config_token)?);
        }

        if let Some(dfe_cookie) = self.auth_data.dfe_cookie.as_deref() {
            headers.insert("x-dfe-cookie", sensitive_header_value(dfe_cookie)?);
        }

        headers.insert("x-dfe-mccmnc", HeaderValue::from_str(&self.device.mcc_mnc())?);

        Ok(headers)
    }

    pub(crate) fn get_auth_headers(&self) -> Result<HeaderMap, PlayError> {
        let mut headers = HeaderMap::new();
        headers.extend([
            (HeaderName::from_static("app"), HeaderValue::from_str(DEFAULT_ANDROID_VENDING_PACKAGE)?),
            (HeaderName::from_static("user-agent"), HeaderValue::from_str(&self.device.auth_user_agent_string())?),
        ]);

        if let Some(gsf_id) = self.auth_data.gsf_id.as_deref() {
            headers.insert("device", sensitive_header_value(gsf_id)?);
        }

        Ok(headers)
    }

    pub(crate) fn get_default_auth_params(&self) -> Result<IndexMap<String, String>, PlayError> {
        let Some(email) = self.auth_data.email.as_ref() else {
            return Err(PlayError::AuthenticationError("Email is required to generate auth params".to_string()));
        };

        let mut params = IndexMap::new();

        if let Some(gsf_id) = self.auth_data.gsf_id.as_ref() {
            params.insert("androidId".to_string(), gsf_id.clone());
        }

        params.extend([
            ("sdk_version".to_string(), self.device.sdk_version().to_string()),
            ("Email".to_string(), email.clone()),
            ("google_play_services_version".to_string(), self.device.play_services_version().to_string()),
            ("device_country".to_string(), self.device.country()),
            ("lang".to_string(), self.device.language()),
            ("callerSig".to_string(), DEFAULT_CALLER_SIG.to_string()),
        ]);

        Ok(params)
    }
}

impl GooglePlayApi {
    pub(crate) async fn check_in(&self) -> Result<AndroidCheckinResponse, PlayError> {
        debug!("device check-in");
        let mut headers = self.get_auth_headers()?;
        headers.extend([(HeaderName::from_static("content-type"), HeaderValue::from_static("application/x-protobuffer"))]);
        let checkin_request = self.device.generate_android_checkin_request();
        let encoded = checkin_request.encode_to_vec();
        let result: AndroidCheckinResponse = self.post_play_proto(URL_CHECK_IN, headers, None, Some(encoded)).await?;
        Ok(result)
    }

    pub(crate) async fn upload_device_config(&self) -> Result<UploadDeviceConfigResponse, PlayError> {
        debug!("uploading device config");
        let mut headers = self.get_default_headers()?;
        headers.insert(HeaderName::from_static("content-type"), HeaderValue::from_static("application/x-protobuf"));
        let request_proto = UploadDeviceConfigRequest {
            device_configuration: Some(self.device.device_configuration()),
            manufacturer: None,
            gcm_registration_id: None,
        };
        let proto_resp = self.post_play_wrapper_resp(URL_UPLOAD_DEVICE_CONFIG, headers, None, Some(request_proto.encode_to_vec())).await?;
        let response = get_payload_field!(proto_resp, upload_device_config_response)?;
        Ok(response)
    }

    #[allow(dead_code)]
    pub(crate) async fn toc(&self) -> Result<TocResponse, PlayError> {
        debug!("checking terms of service");
        let headers = self.get_default_headers()?;
        let proto_resp = self.get_play_wrapper_resp(URL_TOC, headers, None).await?;
        let toc_response = get_payload_field!(proto_resp, toc_response)?;
        if let (Some(tos_token), Some(_)) = (&toc_response.tos_token, &toc_response.tos_content) {
            self.accept_tos(tos_token).await?;
        }
        Ok(toc_response)
    }

    #[allow(dead_code)]
    pub(crate) async fn accept_tos(&self, tos_token: &str) -> Result<AcceptTosResponse, PlayError> {
        debug!("accepting terms of service");
        let headers = self.get_default_headers()?;
        let params = IndexMap::from([("tost".to_string(), tos_token.to_string()), ("toscme".to_string(), "false".to_string())]);
        let proto_resp = self.post_play_wrapper_resp(URL_TOS_ACCEPT, headers, Some(params), None).await?;
        get_payload_field!(proto_resp, accept_tos_response)
    }
}

impl GooglePlayApi {
    #[instrument(name = "auth_setup", level = "info", skip(self))]
    pub async fn setup(&mut self, force: bool) -> Result<(), PlayError> {
        info!("auth setup started");
        if !force && let (Some(_), Some(_)) = (&self.auth_data.gsf_id, &self.auth_data.auth_token) {
            info!(reused = true, "auth ready");
            return Ok(());
        }

        if let (Some(_), Some(_), None) = (&self.auth_data.email, &self.auth_data.oauth_login_token, &self.auth_data.aas_token) {
            let aas_token = self.generate_aas_token().await?;
            self.auth_data.aas_token = Some(aas_token);
        }

        let checkin_response = self.check_in().await?;

        match checkin_response.android_id {
            Some(android_id) => {
                self.auth_data.gsf_id = Some(format!("{android_id:x}"));
            }
            None => {
                return Err(PlayError::AuthenticationError("Check-in response did not contain GSF ID".to_string()));
            }
        }

        if let Some(token) = checkin_response.device_checkin_consistency_token {
            self.auth_data.device_check_in_consistency_token = Some(token);
        }

        let upload_device_response = self.upload_device_config().await?;
        if let Some(token) = upload_device_response.upload_device_config_token {
            self.auth_data.device_config_token = Some(token);
        }

        match (&self.auth_data.aas_token, &self.auth_data.auth_token) {
            (Some(_), _) => {
                let auth_token = self.generate_token(TokenService::GooglePlay).await?;
                self.auth_data.auth_token = Some(auth_token);
            }
            (None, Some(_)) => debug!("using supplied auth token"),
            (None, None) => return Err(PlayError::AuthenticationError("Setup requires either an AAS token or an auth token".to_string())),
        }

        // https://gitlab.com/AuroraOSS/gplayapi/-/commit/735ec0e00ce51b6934a673f17c906e07373a9a43
        // Skip accepting Google ToS because it adds device to Google account
        // let toc_response = self.toc().await?;
        // if let Some(cookie) = toc_response.cookie {
        //     self.auth_data.dfe_cookie = Some(cookie);
        // }

        info!(reused = false, "auth ready");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{sensitive_header_value, token_preview};

    #[test]
    fn secret_headers_are_marked_sensitive() {
        let value = sensitive_header_value("secret").expect("failed to build test header");
        assert!(value.is_sensitive());
        assert_eq!(format!("{value:?}"), "Sensitive");
    }

    #[test]
    fn token_previews_keep_only_the_edges() {
        assert_eq!(token_preview("abcdefghijklmnopqrstuvwxyz"), "abc…xyz");
        assert_eq!(token_preview("short"), "<redacted>");
        assert_eq!(token_preview("abc\ndefghijklmnop"), "<redacted>");
        assert_eq!(token_preview("åßçdefghi界語字"), "<redacted>");
    }
}
