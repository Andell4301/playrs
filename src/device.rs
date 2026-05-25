// SPDX-FileCopyrightText: 2020-2025 Aurora OSS
// SPDX-FileCopyrightText: 2023 The Calyx Institute
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::constants::{DEFAULT_DEVICE, DEFAULT_LOCALE};
use crate::error::DeviceError;
use crate::playproto::{AndroidBuildProto, AndroidCheckinProto, AndroidCheckinRequest, DeviceConfigurationProto, DeviceFeature};
use include_dir::{Dir, include_dir};
use java_properties;
use std::collections::HashMap;
use std::fmt::Display;
use std::io::Cursor;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::debug;

const DEVICES_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/res/devices");
const DEFAULT_VSTR: &str = "7.1.15";
const REQUIRED_DEVICE_FIELDS: &[&str] = &[
    "Build.HARDWARE",
    "Build.RADIO",
    "Build.BOOTLOADER",
    "Build.FINGERPRINT",
    "Build.BRAND",
    "Build.DEVICE",
    "Build.VERSION.SDK_INT",
    "Build.MODEL",
    "Build.MANUFACTURER",
    "Build.PRODUCT",
    "TouchScreen",
    "Keyboard",
    "Navigation",
    "ScreenLayout",
    "HasHardKeyboard",
    "HasFiveWayNavigation",
    "GL.Version",
    "GSF.version",
    "Vending.version",
    "Screen.Density",
    "Screen.Width",
    "Screen.Height",
    "Platforms",
    "SharedLibraries",
    "Features",
    "Locales",
    "CellOperator",
    "SimOperator",
    "Roaming",
    "Client",
    "TimeZone",
    "GL.Extensions",
];

pub fn list_devices() -> Vec<String> {
    DEVICES_DIR.files().into_iter().filter_map(|f| f.path().file_stem().and_then(|s| s.to_str()).map(String::from)).collect()
}

// Nonstandard locale
// TODO: Consider whether this needs to be expanded to support extensions etc. Ideally without an extra dependency
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locale {
    pub language: String,
    pub script: Option<String>,
    pub country: String, // must have a country
}
impl Display for Locale {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(script) = &self.script {
            write!(f, "{}_{}-{}", self.language, script, self.country)
        } else {
            write!(f, "{}_{}", self.language, self.country)
        }
    }
}

impl FromStr for Locale {
    type Err = DeviceError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let normalized = s.replace('-', "_");
        let parts: Vec<&str> = normalized.split('_').collect();

        match parts.len() {
            2 => {
                let language = parts[0].to_lowercase();
                let country = parts[1].to_uppercase();

                if language.is_empty() || country.is_empty() {
                    return Err(DeviceError::LocaleParseError("invalid locale".into()));
                }

                Ok(Locale { language, script: None, country })
            }

            3 => {
                let language = parts[0].to_lowercase();
                let script = {
                    let mut chars = parts[1].chars();
                    match chars.next() {
                        Some(first) => {
                            let mut s = first.to_uppercase().to_string();
                            s.push_str(&chars.as_str().to_lowercase());
                            s
                        }
                        None => return Err(DeviceError::LocaleParseError("invalid script".into())),
                    }
                };
                let country = parts[2].to_uppercase();

                Ok(Locale { language, script: Some(script), country })
            }

            _ => Err(DeviceError::LocaleParseError("invalid locale format".into())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    codename: String,
    locale: Locale,
    properties: HashMap<String, String>,
    raw_properties: Vec<(String, String)>,
}

impl Display for Device {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let build_model = self.get_string("Build.MODEL", "Unknown Model");
        write!(f, "<Device {} ({})> <Locale {}>", self.codename, build_model, self.locale)
    }
}
impl Device {
    pub fn codename(&self) -> &str {
        &self.codename
    }

    pub fn language(&self) -> String {
        self.locale.language.clone()
    }

    pub fn country(&self) -> String {
        self.locale.country.to_lowercase()
    }

    pub fn locale(&self) -> String {
        self.locale.to_string()
    }

    pub fn properties(&self) -> &HashMap<String, String> {
        &self.properties
    }

    pub fn raw_properties(&self) -> Vec<(String, String)> {
        self.raw_properties.iter().filter(|(k, v)| !(v.is_empty() && k.contains('[') && k.contains(']'))).cloned().collect()
    }
}

impl Device {
    fn get_string(&self, key: &str, default: &str) -> String {
        self.properties.get(&key.to_lowercase()).cloned().unwrap_or_else(|| default.to_string())
    }

    fn get_int32(&self, key: &str, default: i32) -> i32 {
        self.properties.get(&key.to_lowercase()).and_then(|v| v.parse::<i32>().ok()).unwrap_or(default)
    }

    fn get_int64(&self, key: &str, default: i64) -> i64 {
        self.properties.get(&key.to_lowercase()).and_then(|v| v.parse::<i64>().ok()).unwrap_or(default)
    }

    fn get_bool(&self, key: &str, default: bool) -> bool {
        self.properties.get(&key.to_lowercase()).map(|v| matches!(v.to_lowercase().as_str(), "true" | "1" | "yes" | "on")).unwrap_or(default)
    }

    fn get_list(&self, key: &str) -> Vec<String> {
        self.properties
            .get(&key.to_lowercase())
            .map(|v| v.split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect())
            .unwrap_or_default()
    }

    fn contains_key(&self, key: &str) -> bool {
        self.properties.contains_key(&key.to_lowercase())
    }

    fn insert(&mut self, key: &str, value: String) {
        self.properties.insert(key.to_lowercase(), value);
    }

    fn is_missing_or_empty(&self, key: &str) -> bool {
        self.properties.get(&key.to_lowercase()).map(|v| v.trim().is_empty()).unwrap_or(true)
    }

    fn set_if_missing_or_empty(&mut self, key: &str, value: String) {
        let k = key.to_lowercase();
        match self.properties.get(&k) {
            Some(existing) if !existing.trim().is_empty() => {}
            _ => {
                self.properties.insert(k, value);
            }
        }
    }

    fn get_timestamp(&self) -> Result<i64, DeviceError> {
        Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64)
    }

    pub fn override_properties<I, K, V>(&mut self, overrides: I)
    where
        I: IntoIterator<Item = (K, Option<V>)>,
        K: AsRef<str>,
        V: Into<String>,
    {
        for (key, value) in overrides {
            let key_str = key.as_ref();
            let k_lower = key_str.to_lowercase();

            match value {
                Some(v) => {
                    let v_str = v.into();
                    self.properties.insert(k_lower, v_str.clone());
                    if let Some(entry) = self.raw_properties.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(key_str)) {
                        entry.1 = v_str;
                    } else {
                        self.raw_properties.push((key_str.to_string(), v_str));
                    }
                }
                None => {
                    self.properties.remove(&k_lower);
                    self.raw_properties.retain(|(k, _)| !k.eq_ignore_ascii_case(key_str));
                }
            }
        }
    }
}

impl Device {
    pub fn new(codename: Option<impl Into<String>>, locale: Option<&str>) -> Result<Self, DeviceError> {
        let locale_str = locale.unwrap_or(DEFAULT_LOCALE);
        let locale: Locale = locale_str.parse()?;
        let codename = codename.map(Into::into).unwrap_or_else(|| DEFAULT_DEVICE.to_string());

        let device_file = DEVICES_DIR.get_file(format!("{codename}.properties")).ok_or_else(|| DeviceError::NotFound(codename.clone()))?;

        let properties: HashMap<String, String> =
            java_properties::read(Cursor::new(device_file.contents()))?.into_iter().map(|(k, v)| (k.to_lowercase(), v)).collect();
        let mut raw_properties: Vec<(String, String)> = Vec::new();
        java_properties::PropertiesIter::new(Cursor::new(device_file.contents())).read_into(|k, v| {
            raw_properties.push((k, v));
        })?;

        let mut device = Device { codename, locale, properties, raw_properties };

        device.check_compatibility()?;
        debug!("Device initialized: {device}");
        Ok(device)
    }

    fn check_compatibility(&mut self) -> Result<(), DeviceError> {
        let missing_fields: Vec<String> =
            REQUIRED_DEVICE_FIELDS.iter().filter(|&field| !self.contains_key(field)).map(|&field| field.to_string()).collect();

        if !missing_fields.is_empty() {
            return Err(DeviceError::MissingFields(missing_fields));
        }

        if self.is_missing_or_empty("Vending.versionString") {
            let version = self.get_string("Vending.version", "");
            let mut vstr = DEFAULT_VSTR.to_string();
            if version.len() > 6 {
                let chars: Vec<char> = version.chars().skip(2).take(4).collect();
                vstr = format!("{}.{}.{}{}", chars[0], chars[1], chars[2], chars[3]);
            }
            self.insert("Vending.versionString", vstr);
        }

        if self.is_missing_or_empty("Build.ID") || self.is_missing_or_empty("Build.VERSION.RELEASE") {
            let fingerprint = self.get_string("Build.FINGERPRINT", "");
            let parts: Vec<&str> = fingerprint.split('/').collect();

            let (mut release, mut build_id) = (String::new(), String::new());

            if parts.len() > 5 {
                if let Some(i) = parts.iter().position(|comp| comp.contains(':')) {
                    if let Some((_, r)) = parts[i].split_once(':') {
                        release = r.to_string();
                    }
                    if i + 1 < parts.len() {
                        build_id = parts[i + 1].to_string();
                    }
                }
            }

            self.set_if_missing_or_empty("Build.ID", build_id);
            self.set_if_missing_or_empty("Build.VERSION.RELEASE", release);
        }

        Ok(())
    }
}

impl Device {
    pub fn sdk_version(&self) -> i32 {
        self.get_int32("Build.VERSION.SDK_INT", 0)
    }

    pub fn play_services_version(&self) -> i32 {
        self.get_int32("GSF.version", 0)
    }

    pub(crate) fn mcc_mnc(&self) -> String {
        self.get_string("SimOperator", "")
    }

    pub fn auth_user_agent_string(&self) -> String {
        let device = self.get_string("Build.DEVICE", "");
        let build_id = self.get_string("Build.ID", "");
        format!("GoogleAuth/1.4 ({device} {build_id})")
    }

    pub(crate) fn user_agent_string(&self) -> String {
        let params = [
            format!("api={}", 3),
            format!("versionCode={}", self.get_string("Vending.version", "")),
            format!("sdk={}", self.get_string("Build.VERSION.SDK_INT", "")),
            format!("device={}", self.get_string("Build.DEVICE", "")),
            format!("hardware={}", self.get_string("Build.HARDWARE", "")),
            format!("product={}", self.get_string("Build.PRODUCT", "")),
            format!("platformVersionRelease={}", self.get_string("Build.VERSION.RELEASE", "")),
            format!("model={}", self.get_string("Build.MODEL", "")),
            format!("buildId={}", self.get_string("Build.ID", "")),
            format!("isWideScreen={}", 0),
            format!("supportedAbis={}", self.get_list("Platforms").join(";")),
        ];

        let version_str = self.get_string("Vending.versionString", "");
        format!("Android-Finsky/{version_str} ({})", params.join(","))
    }

    pub(crate) fn device_configuration(&self) -> DeviceConfigurationProto {
        DeviceConfigurationProto {
            touch_screen: Some(self.get_int32("TouchScreen", 0)),
            keyboard: Some(self.get_int32("Keyboard", 0)),
            navigation: Some(self.get_int32("Navigation", 0)),
            screen_layout: Some(self.get_int32("ScreenLayout", 0)),
            has_hard_keyboard: Some(self.get_bool("HasHardKeyboard", false)),
            has_five_way_navigation: Some(self.get_bool("HasFiveWayNavigation", false)),
            low_ram_device: Some(self.get_int32("LowRamDevice", 0)),
            max_num_of_cpu_cores: Some(self.get_int32("MaxNumOfCPUCores", 8)),
            total_memory_bytes: Some(self.get_int64("TotalMemoryBytes", 8_589_935_000)),
            device_class: Some(0),
            screen_density: Some(self.get_int32("Screen.Density", 0)),
            screen_width: Some(self.get_int32("Screen.Width", 0)),
            screen_height: Some(self.get_int32("Screen.Height", 0)),
            native_platform: self.get_list("Platforms"),
            system_shared_library: self.get_list("SharedLibraries"),
            system_available_feature: self.get_list("Features"),
            system_supported_locale: self.get_list("Locales"),
            gl_es_version: Some(self.get_int32("GL.Version", 0)),
            gl_extension: self.get_list("GL.Extensions"),
            device_feature: self.get_list("Features").into_iter().map(|name| DeviceFeature { name: Some(name), value: Some(0) }).collect(),
            max_apk_download_size_mb: None,
            smallest_screen_width_dp: None,
            unknown28: None,
            unknown30: None,
        }
    }

    pub(crate) fn generate_android_checkin_request(&self) -> AndroidCheckinRequest {
        let build = AndroidBuildProto {
            id: Some(self.get_string("Build.FINGERPRINT", "")),
            product: Some(self.get_string("Build.HARDWARE", "")),
            carrier: Some(self.get_string("Build.BRAND", "")),
            radio: Some(self.get_string("Build.RADIO", "")),
            bootloader: Some(self.get_string("Build.BOOTLOADER", "")),
            device: Some(self.get_string("Build.DEVICE", "")),
            sdk_version: Some(self.get_int32("Build.VERSION.SDK_INT", 0)),
            model: Some(self.get_string("Build.MODEL", "")),
            manufacturer: Some(self.get_string("Build.MANUFACTURER", "")),
            build_product: Some(self.get_string("Build.PRODUCT", "")),
            client: Some(self.get_string("Client", "")),
            ota_installed: Some(self.get_bool("OtaInstalled", false)),
            timestamp: Some(self.get_timestamp().unwrap_or(0)),
            google_services: Some(self.get_int32("GSF.version", 0)),
        };

        let checkin = AndroidCheckinProto {
            build: Some(build),
            last_checkin_msec: Some(0),
            cell_operator: Some(self.get_string("CellOperator", "")),
            sim_operator: Some(self.get_string("SimOperator", "")),
            roaming: Some(self.get_string("Roaming", "")),
            user_number: Some(0),
            event: vec![],
            stat: vec![],
            requested_group: vec![],
        };

        AndroidCheckinRequest {
            id: Some(0),
            checkin: Some(checkin),
            locale: Some(self.locale()),
            time_zone: Some(self.get_string("TimeZone", "")),
            version: Some(3),
            device_configuration: Some(self.device_configuration()),
            fragment: Some(0),
            imei: None,

            digest: None,
            desired_build: None,
            logging_id: None,
            market_checkin: None,
            mac_addr: vec![],
            meid: None,
            account_cookie: vec![],
            security_token: None,
            ota_cert: vec![],
            serial_number: None,
            esn: None,
            mac_addr_type: vec![],
            user_name: None,
            user_serial_number: None,
        }
    }
}
