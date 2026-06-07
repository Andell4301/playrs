// SPDX-FileCopyrightText: 2020-2025 Aurora OSS
// SPDX-FileCopyrightText: 2023-2025 The Calyx Institute
// SPDX-License-Identifier: GPL-3.0-or-later

use clap::ValueEnum;
use const_format::concatcp;
use strum::Display;

pub const DEFAULT_DEVICE: &str = "google_pixel_9a";
pub const DEFAULT_LOCALE: &str = "en-US";

pub const IMAGE_TYPE_APP_SCREENSHOT: i32 = 1;
pub const IMAGE_TYPE_PAGE_BACKGROUND: i32 = 2;
pub const IMAGE_TYPE_YOUTUBE_VIDEO_LINK: i32 = 3;
pub const IMAGE_TYPE_APP_ICON: i32 = 4;
pub const IMAGE_TYPE_CATEGORY_ICON: i32 = 5;
pub const IMAGE_TYPE_VIDEO_THUMBNAIL: i32 = 13;
pub const IMAGE_TYPE_GOOGLE_PLUS_BACKGROUND: i32 = 15;

pub const DEFAULT_CLIENT_SIG: &str = "38918a453d07199354f8b19af05ec6562ced5788";
pub const DEFAULT_CALLER_SIG: &str = "38918a453d07199354f8b19af05ec6562ced5788";
pub const DEFAULT_ANDROID_VENDING_PACKAGE: &str = "com.google.android.gms";
pub const DEFAULT_ANDROID_VENDING_APP: &str = "com.android.vending";
pub const DEFAULT_DFE_TARGETS: &str = "CAESN/qigQYC2AMBFfUbyA7SM5Ij/CvfBoIDgxHqGP8R3xzIBvoQtBKFDZ4HAY4FrwSVMasHBO0O2Q8akgYRAQECAQO7AQEpKZ0CnwECAwRrAQYBr9PPAoK7sQMBAQMCBAkIDAgBAwEDBAICBAUZEgMEBAMLAQEBBQEBAcYBARYED+cBfS8CHQEKkAEMMxcBIQoUDwYHIjd3DQ4MFk0JWGYZEREYAQOLAYEBFDMIEYMBAgICAgICOxkCD18LGQKEAcgDBIQBAgGLARkYCy8oBTJlBCUocxQn0QUBDkkGxgNZQq0BZSbeAmIDgAEBOgGtAaMCDAOQAZ4BBIEBKUtQUYYBQscDDxPSARA1oAEHAWmnAsMB2wFyywGLAxol+wImlwOOA80CtwN26A0WjwJVbQEJPAH+BRDeAfkHK/ABASEBCSAaHQemAzkaRiu2Ad8BdXeiAwEBGBUBBN4LEIABK4gB2AFLfwECAdoENq0CkQGMBsIBiQEtiwGgA1zyAUQ4uwS8AwhsvgPyAcEDF27vApsBHaICGhl3GSKxAR8MC6cBAgItmQYG9QIeywLvAeYBDArLAh8HASI4ELICDVmVBgsY/gHWARtcAsMBpALiAdsBA7QBpAJmIArpByn0AyAKBwHTARIHAX8D+AMBcRIBBbEDmwUBMacCHAciNp0BAQF0OgQLJDuSAh54kwFSP0eeAQQ4M5EBQgMEmwFXywFo0gFyWwMcapQBBugBPUW2AVgBKmy3AR6PAbMBGQxrUJECvQR+8gFoWDsYgQNwRSczBRXQAgtRswEW0ALMAREYAUEBIG6yATYCRE8OxgER8gMBvQEDRkwLc8MBTwHZAUOnAXiiBakDIbYBNNcCIUmuArIBSakBrgFHKs0EgwV/G3AD0wE6LgECtQJ4xQFwFbUCjQPkBS6vAQqEAUZF3QIM9wEhCoYCQhXsBCyZArQDugIziALWAdIBlQHwBdUErQE6qQaSA4EEIvYBHir9AQVLmgMCApsCKAwHuwgrENsBAjNYswEVmgIt7QJnN4wDEnta+wGfAcUBxgEtEFXQAQWdAUAeBcwBAQM7rAEJATJ0LENrdh73A6UBhAE+qwEeASxLZUMhDREuH0CGARbd7K0GlQo";
pub const DEFAULT_DFE_PHENOTYPE: &str = "H4sIAAAAAAAAAB3OO3KjMAAA0KRNuWXukBkBQkAJ2MhgAZb5u2GCwQZbCH_EJ77QHmgvtDtbv-Z9_H63zXXU0NVPB1odlyGy7751Q3CitlPDvFd8lxhz3tpNmz7P92CFw73zdHU2Ie0Ad2kmR8lxhiErTFLt3RPGfJQHSDy7Clw10bg8kqf2owLokN4SecJTLoSwBnzQSd652_MOf2d1vKBNVedzg4ciPoLz2mQ8efGAgYeLou-l-PXn_7Sna1MfhHuySxt-4esulEDp8Sbq54CPPKjpANW-lkU2IZ0F92LBI-ukCKSptqeq1eXU96LD9nZfhKHdtjSWwJqUm_2r6pMHOxk01saVanmNopjX3YxQafC4iC6T55aRbC8nTI98AF_kItIQAJb5EQxnKTO7TZDWnr01HVPxelb9A2OWX6poidMWl16K54kcu_jhXw-JSBQkVcD_fPsLSZu6joIBAAA";
pub const LEGACY_USER_AGENT: &str = "Android-Finsky/29.2.15-21 [0] [PR] 426536134 (api=3,versionCode=82921510,sdk=25)";

pub const URL_BASE: &str = "https://android.clients.google.com";
pub const URL_FDFE: &str = concatcp!(URL_BASE, "/fdfe");
pub const URL_ACQUIRE: &str = concatcp!(URL_FDFE, "/acquire");
pub const URL_CATEGORIES: &str = concatcp!(URL_FDFE, "/categoriesList");
pub const URL_CATEGORIES_2: &str = concatcp!(URL_FDFE, "/allCategoriesList");
pub const URL_DELIVERY: &str = concatcp!(URL_FDFE, "/delivery");
pub const URL_PURCHASE: &str = concatcp!(URL_FDFE, "/purchase");
pub const URL_PURCHASE_HISTORY: &str = concatcp!(URL_FDFE, "/purchaseHistory");
pub const URL_TOP_CHART: &str = concatcp!(URL_FDFE, "/listTopChartItems");
pub const URL_AUTH: &str = concatcp!(URL_BASE, "/auth");
pub const URL_BULK_DETAILS: &str = concatcp!(URL_FDFE, "/bulkDetails");
pub const URL_BULK_PREFETCH: &str = concatcp!(URL_FDFE, "/bulkPrefetch");
pub const URL_CHECK_IN: &str = concatcp!(URL_BASE, "/checkin");
pub const URL_DETAILS: &str = concatcp!(URL_FDFE, "/details");
pub const URL_DETAILS_DEVELOPER: &str = concatcp!(URL_FDFE, "/browseDeveloperPage");
pub const URL_MY_APPS: &str = concatcp!(URL_FDFE, "/myApps");
pub const URL_REVIEW_ADD_EDIT: &str = concatcp!(URL_FDFE, "/addReview");
pub const URL_REVIEW_DELETE: &str = concatcp!(URL_FDFE, "/deleteReview");
pub const URL_REVIEW_USER: &str = concatcp!(URL_FDFE, "/userReview");
pub const URL_REVIEWS: &str = concatcp!(URL_FDFE, "/rev");
pub const URL_SEARCH: &str = concatcp!(URL_FDFE, "/search");
pub const URL_SEARCH_SUGGEST: &str = concatcp!(URL_FDFE, "/searchSuggest");
pub const URL_TESTING_PROGRAM: &str = concatcp!(URL_FDFE, "/apps/testingProgram");
pub const URL_TOC: &str = concatcp!(URL_FDFE, "/toc");
pub const URL_TOS_ACCEPT: &str = concatcp!(URL_FDFE, "/acceptTos");
pub const URL_UPLOAD_DEVICE_CONFIG: &str = concatcp!(URL_FDFE, "/uploadDeviceConfig");
pub const URL_SYNC: &str = concatcp!(URL_FDFE, "/apps/contentSync");
pub const URL_SELF_UPDATE: &str = concatcp!(URL_FDFE, "/selfUpdate");
pub const URL_USER_PROFILE: &str = concatcp!(URL_FDFE, "/api/userProfile");
pub const URL_LIBRARY: &str = concatcp!(URL_FDFE, "/library");
pub const URL_MODIFY_LIBRARY: &str = concatcp!(URL_FDFE, "/modifyLibrary");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Abuse {
    SexualContent = 1,
    GraphicViolence = 3,
    HatefulOrAbusiveContent = 4,
    ImproperContentRating = 5,
    HarmfulToDeviceOrData = 7,
    Other = 8,
    IllegalPrescription = 11,
    Impersonation = 12,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Restriction {
    Generic = -1,
    NotRestricted = 1,
    GeoRestricted = 2,
    DeviceRestricted = 7,
    NotInGroup = 8,
    Unknown = 9,
    CarrierRestricted = 10,
    CountryOrCarrierRestricted = 11,
    ParentalControlRestriction = 12,
    AdminRestricted = 21,
    AdminPermissionNotAccepted = 22,
    AgeRestricted = 30,
    AppOutdated = 32,
}

impl From<i32> for Restriction {
    fn from(v: i32) -> Self {
        Self::try_from(v).unwrap_or(Restriction::Generic)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Display)]
#[strum(serialize_all = "lowercase")]
pub enum TokenService {
    AC2DM,
    Android,
    #[strum(serialize = "AndroidCheckInServer")]
    AndroidCheckInServer,
    #[strum(serialize = "experimentsandconfigs")]
    ExperimentalConfig,
    GCM,
    GooglePlay,
    Numberer,
    #[strum(serialize = "OAuthLogin")]
    Oauthlogin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Display)]
#[strum(serialize_all = "UPPERCASE")]
pub enum CategoryType {
    Application,
    Game,
    Family,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CategoryWebType {
    Application = 0,
    Game = 1,
    Family = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Display)]
pub enum ClusterType {
    #[strum(serialize = "INSTALLED")]
    MyAppsInstalled,
    #[strum(serialize = "LIBRARY")]
    MyAppsLibrary,
    #[strum(serialize = "UPDATES")]
    MyAppsUpdates,
}

// Google actually overloads the enums like this (e.g., positive & one both map to "1")
// They get added to different keys based on the context
#[derive(Debug, Clone, Copy, PartialEq, Eq, Display)]
pub enum ReviewFilter {
    #[strum(serialize = "ALL")]
    All,
    #[strum(serialize = "0")]
    Newest,
    #[strum(serialize = "1")]
    Positive,
    #[strum(serialize = "2")]
    Critical,
    #[strum(serialize = "5")]
    Five,
    #[strum(serialize = "4")]
    Four,
    #[strum(serialize = "3")]
    Three,
    #[strum(serialize = "2")]
    Two,
    #[strum(serialize = "1")]
    One,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Display)]
pub enum ModifyWishlistAction {
    #[strum(serialize = "add")]
    Add,
    #[strum(serialize = "remove")]
    Remove,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Display)]
pub enum StreamCategory {
    #[strum(serialize = "APPLICATION")]
    Application,
    #[strum(serialize = "GAME")]
    Game,
    #[strum(serialize = "NONE")]
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Display)]
pub enum StreamType {
    #[strum(serialize = "appsEarlyAccessStream")]
    EarlyAccess,
    #[strum(serialize = "getAppsEditorsChoiceStream")]
    EditorChoice,
    #[strum(serialize = "getHomeStream")]
    Home,
    #[strum(serialize = "myAppsStream?tab=LIBRARY")]
    MyAppsLibrary,
    #[strum(serialize = "getAppsPremiumGameStream")]
    PremiumGames,
    #[strum(serialize = "subnavHome")]
    SubNav,
    #[strum(serialize = "topChartsStream")]
    TopChart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Display, ValueEnum)]
pub enum PatchFormat {
    Gdiff = 1,
    GzippedGdiff = 2,
    GzippedBsdiff = 3,
    Unknown4 = 4,
    Unknown5 = 5,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Display)]
pub enum PlayFileType {
    Base = 0,
    Obb = 1,
    Patch = 2,
    Split = 3,
    Dex = 4,
}
