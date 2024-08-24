//! A library for interacting with the Google Play API.
//!
//! # Getting Started
//!
//! To interact with the API, first you'll have to obtain an OAuth token by visiting the Google
//! [embedded setup page](https://accounts.google.com/EmbeddedSetup/identifier?flowName=EmbeddedSetupAndroid)
//! and opening the browser debugging console, logging in, and looking for the `oauth_token` cookie
//! being set on your browser.  It will be present in the last requests being made and start with
//! "oauth2_4/".  Copy this value.  It can only be used once, in order to obtain the `aas_token`,
//! which can be used subsequently.  To obtain this token:
//!
//! ```rust
//! use gpapi::Gpapi;
//!
//! #[tokio::main]
//! async fn main() {
//!     let mut api = Gpapi::new("ad_g3_pro", &email);
//!     api.request_aas_token(oauth_token).await.unwrap();
//!     println!("{:?}", api.get_aas_token());
//! }
//! ```
//!
//! Now, you can begin interacting with the API by initializing it setting the `aas_token` and
//! logging in.
//!
//! ```rust
//! use gpapi::Gpapi;
//!
//! #[tokio::main]
//! async fn main() {
//!     let mut api = Gpapi::new("px_7a", &email);
//!     api.set_aas_token(aas_token);
//!     api.login().await.unwrap();
//!     // do something
//! }
//! ```
//!
//! From here, you can get package details, get the info to download a package, or use the library to download it.
//!
//! ```rust
//! # use gpapi::Gpapi;
//! # use std::path::Path;
//! # #[tokio::main]
//! # async fn main() {
//! # let mut api = Gpapi::new("px_7a", &email);
//! # api.set_aas_token(aas_token);
//! # api.login().await.unwrap();
//! let details = api.details("com.instagram.android").await;
//! println!("{:?}", details);
//!
//! let download_info = api.get_download_info("com.instagram.android", None).await;
//! println!("{:?}", download_info);
//!
//! api.download("com.instagram.android", None, true, true, &Path::new("/tmp/testing"), None).await;
//! # }
//! ```

mod consts;
pub mod error;

use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use futures::future::TryFutureExt;
//use hyper::client::HttpConnector;
//use hyper::header::{HeaderName as HyperHeaderName, HeaderValue as HyperHeaderValue};
//use hyper::{Body, Client, Method, Request};
//use hyper_tls::HttpsConnector;
use prost::Message;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::Url;
use tokio_dl_stream_to_disk::AsyncDownload;

use crate::error::{Error as GpapiError, ErrorKind as GpapiErrorKind};

use googleplay_protobuf::{
    AcceptTosResponse, AndroidCheckinProto, AndroidCheckinRequest, AndroidCheckinResponse,
    BulkDetailsRequest, BulkDetailsResponse, DetailsResponse, DeviceConfigurationProto,
    ResponseWrapper, UploadDeviceConfigRequest, UploadDeviceConfigResponse,
};

use serde::{Deserialize, Serialize};
include!("device_properties.rs");

static DEVICES_ENCODED: &[u8] = include_bytes!("device_properties.bin");

type MainAPKDownloadURL = Option<String>;
type SplitsDownloadInfo = Vec<(Option<String>, Option<String>)>;
type AdditionalFilesDownloadInfo = Vec<(Option<String>, Option<String>)>;
type DownloadInfo = (
    MainAPKDownloadURL,
    SplitsDownloadInfo,
    AdditionalFilesDownloadInfo,
);

#[derive(Debug)]
pub struct Gpapi {
    locale: String,
    timezone: String,
    device_properties: DeviceProperties,
    email: String,
    aas_token: Option<String>,
    auth_token: Option<String>,
    device_config_token: Option<String>,
    device_checkin_consistency_token: Option<String>,
    tos_token: Option<String>,
    dfe_cookie: Option<String>,
    gsf_id: Option<i64>,
    client: Box<reqwest::Client>,
    //hyper_client: Box<hyper::Client<HttpsConnector<HttpConnector>>>,
}

impl Gpapi {
    /// Returns a Gpapi struct.
    ///
    pub fn new<S: Into<String>>(device_codename: S, email: S) -> Self {
        //let mut http = HttpConnector::new();
        //http.enforce_http(false);
        //let https = HttpsConnector::new_with_connector(http);
        //let hyper_client = Client::builder().build::<_, hyper::Body>(https);

        Gpapi {
            locale: String::from("en_US"),
            timezone: String::from("UTC"),
            device_properties: bincode::deserialize::<HashMap<String, EncodedDeviceProperties>>(
                DEVICES_ENCODED,
            )
            .unwrap()
            .remove(&device_codename.into())
            .expect("Invalid device codename")
            .to_decoded(),
            email: email.into(),
            aas_token: None,
            auth_token: None,
            device_config_token: None,
            device_checkin_consistency_token: None,
            tos_token: None,
            dfe_cookie: None,
            gsf_id: None,
            client: Box::new(reqwest::Client::new()),
            //hyper_client: Box::new(hyper_client),
        }
    }

    /// Set the locale
    pub fn set_locale<S: Into<String>>(&mut self, locale: S) {
        self.locale = locale.into();
    }

    /// Set the time zone
    pub fn set_timezone<S: Into<String>>(&mut self, timezone: S) {
        self.timezone = timezone.into();
    }

    /// Set the aas token. This can be requested via `request_aas_token`, and is required for most
    /// other actions.
    pub fn set_aas_token<S: Into<String>>(&mut self, aas_token: S) {
        self.aas_token = Some(aas_token.into());
    }

    /// Request and set the aas token given an oauth token and the associated email.
    ///
    /// # Arguments
    ///
    /// * `oauth_token` - An oauth token you previously retrieved separately
    pub async fn request_aas_token<S: Into<String>>(
        &mut self,
        oauth_token: S,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let oauth_token = oauth_token.into();
        let auth_req = AuthRequest::new(&self.email, &oauth_token);
        let mut resp = self.request_aas_token_helper(&auth_req).await?;
        self.aas_token = Some(
            resp.remove("token")
                .ok_or(Box::new(GpapiError::new(GpapiErrorKind::Authentication)))?,
        );
        Ok(())
    }

    async fn request_aas_token_helper(
        &self,
        auth_req: &AuthRequest,
    ) -> Result<HashMap<String, String>, Box<dyn Error + Send + Sync>> {
        let form_body = form_post(&auth_req.params);

        let mut headers = HashMap::new();
        headers.insert(
            "user-agent",
            String::from(consts::defaults::DEFAULT_AUTH_USER_AGENT),
        );
        headers.insert(
            "content-type",
            String::from("application/x-www-form-urlencoded"),
        );
        headers.insert("app", String::from("com.google.android.gms"));

        let body_bytes = self
            .execute_request_helper("auth", None, Some(&form_body.into_bytes()), headers, false)
            .await?;

        let reply = parse_form_reply(&std::str::from_utf8(&body_bytes.to_vec()).unwrap());
        Ok(reply)
    }

    /// Get the aas token that has been previously set by either `request_aas_token` or
    /// `set_aas_token`.
    pub fn get_aas_token(&self) -> Option<&str> {
        self.aas_token.as_ref().map(|token| token.as_str())
    }

    /// Log in to Google's Play Store API.  This is required for most other actions. The aas token
    /// has to be set via `request_aas_token` or `set_aas_token` first.
    pub async fn login(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.checkin().await?;
        if let Some(upload_device_config_token) = self.upload_device_config().await? {
            self.device_config_token = Some(
                upload_device_config_token
                    .upload_device_config_token
                    .unwrap(),
            );
            self.request_auth_token().await?;
            self.toc().await?;
            Ok(())
        } else {
            Err("No device config token".into())
        }
    }

    /// Download a package, given a package ID, optional version code, and filesystem path.
    ///
    /// # Arguments
    ///
    /// * `pkg_name` - A string type specifying the package's app ID, e.g. `com.instagram.android`
    /// * `version_code` - An optinal version code, given in i32.  If omitted, the latest version will
    /// be used
    /// * `split_if_available` - A boolean indicating whether a split APK is desired, if available
    /// * `include_additional_files` - A boolean indicating if additional files should be
    /// downloaded as well, if available
    /// * `dst_path` - A path to download the file to.
    /// * `cb` - An optional callback for reporting information about the download asynchronously.  The outer
    /// callback takes the filename (String) and total size in bytes of the file (u64) and returns an inner
    /// callback.  The inner callback takes the position of the current download.
    ///
    /// # Errors
    ///
    /// If the file already exists for this download, an Err([`Error`]) result is returned with an
    /// [`ErrorKind`] of FileExists.
    /// If additional files or a split APK is to be downloaded but the directory already exists, an
    /// Err([`Error`]) result is returned with an [`ErrorKind`] of DirectoryExists.
    /// If the specified directory is misssing, an Err([`Error`]) result is returned with an
    /// [`ErrorKind`] of DirectoryMissing.
    pub async fn download<S: Into<String>>(
        &self,
        pkg_name: S,
        version_code: Option<i32>,
        split_if_available: bool,
        include_additional_files: bool,
        dst_path: &Path,
        cb: Option<&Box<dyn Fn(String, u64) -> Box<dyn Fn(u64) -> ()>>>,
    ) -> Result<Vec<()>, Box<dyn Error + Send + Sync>> {
        let pkg_name = pkg_name.into();
        let download_info = self
            .get_download_info(pkg_name.clone(), version_code)
            .await?;

        let mut dst_path = PathBuf::from(dst_path);
        if dst_path.is_dir() {
            if (split_if_available && download_info.1.len() > 0)
                || (include_additional_files && download_info.2.len() > 0)
            {
                dst_path.push(pkg_name.clone());
                if dst_path.is_dir() {
                    return Err(Box::new(GpapiError::new(GpapiErrorKind::DirectoryExists)));
                } else {
                    fs::create_dir(&dst_path).map_err(|e| GpapiError::from(e))?;
                }
            }
        } else {
            return Err(Box::new(GpapiError::new(GpapiErrorKind::DirectoryMissing)));
        }

        let mut downloads = Vec::new();
        let err = |e| Box::new(GpapiError::from(e)) as Box<dyn Error + Send + Sync>;
        if include_additional_files && download_info.2.len() > 0 {
            for additional_file in download_info.2 {
                if let (Some(filename), Some(download_url)) = additional_file {
                    let dl = AsyncDownload::new(&download_url, &dst_path, &filename)
                        .get()
                        .await?;
                    let length = dl.length();
                    let cb = match length {
                        Some(length) => cb.map(|c| c(filename.clone(), length)),
                        None => None,
                    };
                    downloads.push((dl, cb));
                }
            }
        }

        if split_if_available && download_info.1.len() > 0 {
            for split in download_info.1 {
                if let (Some(download_name), Some(download_url)) = split {
                    let filename = format!("{}.{}.apk", pkg_name, download_name);
                    let dl = AsyncDownload::new(&download_url, &dst_path, &filename)
                        .get()
                        .await?;
                    let length = dl.length();
                    let cb = match length {
                        Some(length) => cb.map(|c| c(filename.clone(), length)),
                        None => None,
                    };
                    downloads.push((dl, cb));
                }
            }
        }

        let filename = format!("{}.apk", pkg_name);
        if let Some(download_url) = download_info.0 {
            let dl = AsyncDownload::new(&download_url, &dst_path, &filename)
                .get()
                .await?;
            let length = dl.length();
            let cb = match length {
                Some(length) => cb.map(|c| c(filename.clone(), length)),
                None => None,
            };
            downloads.push((dl, cb));
        } else {
            return Err("Could not download app - no download URL available".into());
        }
        futures::future::try_join_all(
            downloads
                .iter_mut()
                .map(|(d, c)| d.download(c).map_err(err)),
        )
        .await
    }

    /// Retrieve the download URL(s) and names for a package, given a package ID and optional
    /// version code.
    ///
    /// # Arguments
    ///
    /// * `pkg_name` - A string type specifying the package's app ID, e.g. `com.instagram.android`
    /// * `version_code` - An optinal version code, given in i32.  If omitted, the latest version will
    /// be used
    ///
    /// # Returns
    ///
    /// * An Option<String> to the full APK download URL, followed by a Vec<(Option<String>,
    /// Option<String>)> which corresponds to a list of download URLs and names for the split APK,
    /// then followed by another Vec<(Option<String>, Option<String>)> which corresponds to the
    /// download URLs and filenames for additional files.
    pub async fn get_download_info<S: Into<String>>(
        &self,
        pkg_name: S,
        mut version_code: Option<i32>,
    ) -> Result<DownloadInfo, Box<dyn Error + Send + Sync>> {
        let pkg_name = pkg_name.into();
        if self.auth_token.is_none() {
            return Err(Box::new(GpapiError::new(GpapiErrorKind::LoginRequired)));
        }
        if version_code.is_none() {
            version_code = Some(self.get_latest_version_for_pkg_name(&pkg_name).await?);
        }
        let resp = {
            let version_code_string = version_code.unwrap().to_string();
            let mut params = HashMap::new();
            params.insert("ot", String::from("1"));
            params.insert("doc", String::from(&pkg_name));
            params.insert("vc", version_code_string);

            let mut headers = self.get_default_headers()?;
            headers.insert("content-length", String::from("0"));

            self.execute_request("purchase", Some(params), Some(&[]), headers)
                .await?
        };
        if let Some(payload) = resp.payload {
            if let Some(buy_response) = payload.buy_response {
                if let Some(delivery_token) = buy_response.encoded_delivery_token {
                    return self
                        .delivery(&pkg_name, version_code.clone(), &delivery_token)
                        .await;
                }
            }
        }
        Err(Box::new(GpapiError::new(GpapiErrorKind::InvalidApp)))
    }

    async fn delivery<S: Into<String>>(
        &self,
        pkg_name: S,
        mut version_code: Option<i32>,
        delivery_token: S,
    ) -> Result<DownloadInfo, Box<dyn Error + Send + Sync>> {
        let pkg_name = pkg_name.into();
        let delivery_token = delivery_token.into();
        if self.auth_token.is_none() {
            return Err(Box::new(GpapiError::new(GpapiErrorKind::LoginRequired)));
        }
        if version_code.is_none() {
            version_code = Some(self.get_latest_version_for_pkg_name(&pkg_name).await?);
        }
        let resp = {
            let version_code_string = version_code.unwrap().to_string();
            let mut req = HashMap::new();
            req.insert("ot", String::from("1"));
            req.insert("doc", pkg_name.clone());
            req.insert("vc", version_code_string);
            req.insert("dtok", delivery_token);
            self.execute_request("delivery", Some(req), None, self.get_default_headers()?)
                .await?
        };
        if let Some(payload) = resp.payload {
            if let Some(delivery_response) = payload.delivery_response {
                if let Some(app_delivery_data) = delivery_response.app_delivery_data {
                    let mut splits = Vec::new();
                    for app_split_delivery_data in app_delivery_data.split_delivery_data {
                        splits.push((
                            app_split_delivery_data.name,
                            app_split_delivery_data.download_url,
                        ));
                    }
                    let mut additional_files: Vec<(Option<String>, Option<String>)> = Vec::new();
                    for additional_file in app_delivery_data.additional_file {
                        if let Some(file_type) = additional_file.file_type {
                            if let Some(version_code) = additional_file.version_code {
                                let main_patch = match file_type {
                                    0 => "main",
                                    _ => "patch",
                                };
                                let filename =
                                    format!("{}.{}.{}.obb", main_patch, version_code, pkg_name);
                                additional_files
                                    .push((Some(filename), additional_file.download_url));
                            }
                        }
                    }
                    return Ok((app_delivery_data.download_url, splits, additional_files));
                }
            }
        }
        Err(Box::new(GpapiError::new(GpapiErrorKind::InvalidApp)))
    }

    async fn get_latest_version_for_pkg_name(
        &self,
        pkg_name: &str,
    ) -> Result<i32, Box<dyn Error + Send + Sync>> {
        if let Some(details) = self.details(pkg_name).await? {
            if let Some(item) = details.item {
                if let Some(details) = item.details {
                    if let Some(app_details) = details.app_details {
                        if let Some(version_code) = app_details.version_code {
                            return Ok(version_code);
                        }
                    }
                }
            }
        }
        Err(Box::new(GpapiError::new(GpapiErrorKind::InvalidApp)))
    }

    /// Play Store package detail request (provides more detail than bulk requests).
    ///
    /// # Arguments
    ///
    /// * `pkg_name` - A string type specifying the package's app ID, e.g. `com.instagram.android`
    pub async fn details<S: Into<String>>(
        &self,
        pkg_name: S,
    ) -> Result<Option<DetailsResponse>, Box<dyn Error + Send + Sync>> {
        if self.auth_token.is_none() {
            return Err(Box::new(GpapiError::new(GpapiErrorKind::LoginRequired)));
        }
        let mut form_params = HashMap::new();
        form_params.insert("doc", pkg_name.into());

        let headers = self.get_default_headers()?;

        let resp = self
            .execute_request("details", Some(form_params), None, headers)
            .await?;

        if let Some(payload) = resp.payload {
            Ok(payload.details_response)
        } else {
            Ok(None)
        }
    }

    /// Play Store bulk detail request for multiple apps.
    ///
    /// # Arguments
    ///
    /// * `pkg_names` - An array of string types specifying package app IDs
    pub async fn bulk_details(
        &self,
        pkg_names: &[&str],
    ) -> Result<Option<BulkDetailsResponse>, Box<dyn Error + Send + Sync>> {
        if self.auth_token.is_none() {
            return Err(Box::new(GpapiError::new(GpapiErrorKind::LoginRequired)));
        }
        let mut req = BulkDetailsRequest::default();
        req.doc_id = pkg_names.into_iter().cloned().map(String::from).collect();
        req.include_child_docs = Some(false);
        let mut bytes = Vec::new();
        bytes.reserve(req.encoded_len());
        req.encode(&mut bytes).unwrap();
        let resp = self
            .execute_request(
                "bulkDetails",
                None,
                Some(&bytes),
                self.get_default_headers()?,
            )
            .await?;
        if let Some(payload) = resp.payload {
            Ok(payload.bulk_details_response)
        } else {
            Ok(None)
        }
    }

    async fn checkin(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        let mut checkin = self.device_properties.android_checkin.clone();

        checkin.build.as_mut().map(|b| {
            b.timestamp = Some(
                (SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
                    / 1000) as i64,
            )
        });

        let build_device = checkin
            .build
            .as_ref()
            .unwrap()
            .device
            .as_ref()
            .unwrap()
            .clone();

        let mut req = AndroidCheckinRequest::default();
        req.id = Some(0);
        req.checkin = Some(checkin);
        req.locale = Some(self.locale.clone());
        req.time_zone = Some(self.timezone.clone());
        req.version = Some(3);
        req.device_configuration = Some(self.device_properties.device_configuration.clone());
        req.fragment = Some(0);
        let mut bytes = Vec::new();
        bytes.reserve(req.encoded_len());
        req.encode(&mut bytes).unwrap();

        let build_id = self
            .device_properties
            .extra_info
            .get("Build.ID")
            .unwrap()
            .clone();
        let mut headers = HashMap::new();
        self.append_auth_headers(&mut headers, build_device, build_id);

        let resp = self.execute_checkin_request(&bytes, headers).await?;
        self.device_checkin_consistency_token = resp.device_checkin_consistency_token;
        self.gsf_id = resp.android_id.map(|id| id as i64);
        Ok(())
    }

    async fn execute_checkin_request(
        &self,
        msg: &[u8],
        mut auth_headers: HashMap<&str, String>,
    ) -> Result<AndroidCheckinResponse, Box<dyn Error + Send + Sync>> {
        auth_headers.insert("content-type", String::from("application/x-protobuf"));
        auth_headers.insert("host", String::from("android.clients.google.com"));
        let bytes = self
            .execute_request_helper("checkin", None, Some(msg), auth_headers, false)
            .await?;
        let resp = AndroidCheckinResponse::decode(&mut Cursor::new(bytes))?;
        Ok(resp)
    }

    fn get_default_headers(&self) -> Result<HashMap<&str, String>, Box<dyn Error + Send + Sync>> {
        let mut headers = HashMap::new();
        self.append_default_headers(&mut headers)?;
        Ok(headers)
    }

    fn append_default_headers(
        &self,
        headers: &mut HashMap<&str, String>,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        if let Some(auth_token) = &self.auth_token {
            headers.insert("Authorization", format!("Bearer {}", auth_token.clone()));
        }

        let build = self
            .device_properties
            .android_checkin
            .clone()
            .build
            .unwrap();
        let device_configuration = self.device_properties.device_configuration.clone();

        let build_configuration = BuildConfiguration::new(
            self.device_properties
                .extra_info
                .get("Vending.versionString")
                .unwrap(),
            self.device_properties
                .extra_info
                .get("Vending.version")
                .unwrap(),
            &build.sdk_version.as_ref().unwrap().to_string(),
            &build.device.as_ref().unwrap(),
            &build.product.as_ref().unwrap(),
            &build.build_product.as_ref().unwrap(),
            self.device_properties
                .extra_info
                .get("Build.VERSION.RELEASE")
                .unwrap(),
            &build.model.as_ref().unwrap(),
            self.device_properties.extra_info.get("Build.ID").unwrap(),
            &device_configuration.native_platform.join(";"),
        );

        headers.insert("user-agent", build_configuration.user_agent());

        if let Some(gsf_id) = &self.gsf_id {
            headers.insert("X-DFE-Device-Id", format!("{:x}", gsf_id));
        }
        headers.insert("accept-language", self.locale.replace("_", "-"));
        headers.insert(
            "X-DFE-Encoded-Targets",
            String::from(consts::defaults::DEFAULT_DFE_TARGETS),
        );
        headers.insert(
            "X-DFE-Phenotype",
            String::from(consts::defaults::DEFAULT_DFE_PHENOTYPE),
        );
        headers.insert("X-DFE-Client-Id", String::from("am-android-google"));
        headers.insert("X-DFE-Network-Type", String::from("4"));
        headers.insert("X-DFE-Content-Filters", String::from(""));
        headers.insert("X-Limit-Ad-Tracking-Enabled", String::from("false"));
        headers.insert("X-Ad-Id", String::from(""));
        headers.insert("X-DFE-UserLanguages", String::from(&self.locale));
        headers.insert("X-DFE-Request-Params", String::from("timeoutMs=4000"));
        if let Some(device_checkin_consistency_token) = &self.device_checkin_consistency_token {
            headers.insert(
                "X-DFE-Device-Checkin-Consistency-Token",
                device_checkin_consistency_token.clone(),
            );
        }
        if let Some(device_config_token) = &self.device_config_token {
            headers.insert("X-DFE-Device-Config-Token", device_config_token.clone());
        }
        if let Some(dfe_cookie) = &self.dfe_cookie {
            headers.insert("X-DFE-Cookie", dfe_cookie.clone());
        }
        if let Some(mcc_mcn) = self.device_properties.extra_info.get("SimOperator") {
            headers.insert("X-DFE-MCCMCN", mcc_mcn.clone());
        }
        Ok(())
    }

    fn append_auth_headers<S: Into<String>>(
        &self,
        headers: &mut HashMap<&str, String>,
        build_device: S,
        build_id: S,
    ) {
        headers.insert(
            "app",
            String::from(consts::defaults::DEFAULT_ANDROID_VENDING),
        );
        headers.insert(
            "User-Agent",
            format!(
                "GoogleAuth/1.4 ({} {})",
                build_device.into(),
                build_id.into()
            ),
        );
        if let Some(gsf_id) = self.gsf_id {
            headers.insert("device", format!("{:x}", gsf_id));
        }
    }

    fn append_default_auth_params(&self, params: &mut HashMap<&str, String>) {
        if let Some(gsf_id) = self.gsf_id {
            params.insert("androidId", format!("{:x}", gsf_id));
        }

        let build = self
            .device_properties
            .android_checkin
            .clone()
            .build
            .unwrap();
        params.insert(
            "sdk_version",
            build.sdk_version.as_ref().unwrap().to_string(),
        );
        params.insert("Email", self.email.clone());
        params.insert(
            "google_play_services_version",
            build.google_services.as_ref().unwrap().to_string(),
        );
        params.insert(
            "device_country",
            String::from(consts::defaults::DEFAULT_COUNTRY_CODE).to_ascii_lowercase(),
        );
        params.insert(
            "lang",
            String::from(consts::defaults::DEFAULT_LANGUAGE).to_ascii_lowercase(),
        );
        params.insert(
            "callerSig",
            String::from(consts::defaults::DEFAULT_CALLER_SIG),
        );
    }

    fn append_auth_params(&self, params: &mut HashMap<&str, String>) {
        params.insert("app", String::from("com.android.vending"));
        params.insert(
            "client_sig",
            String::from(consts::defaults::DEFAULT_CLIENT_SIG),
        );
        params.insert(
            "callerPkg",
            String::from(consts::defaults::DEFAULT_ANDROID_VENDING),
        );
        params.insert("Token", self.aas_token.as_ref().unwrap().clone());
        params.insert("oauth2_foreground", String::from("1"));
        params.insert("token_request_options", String::from("CAA4AVAB"));
        params.insert("check_email", String::from("1"));
        params.insert("system_partition", String::from("1"));
    }

    async fn upload_device_config(
        &self,
    ) -> Result<Option<UploadDeviceConfigResponse>, Box<dyn Error + Send + Sync>> {
        let mut req = UploadDeviceConfigRequest::default();
        req.device_configuration = Some(self.device_properties.device_configuration.clone());
        let mut bytes = Vec::new();
        bytes.reserve(req.encoded_len());
        req.encode(&mut bytes).unwrap();

        let mut headers = self.get_default_headers()?;
        headers.insert("content-type", String::from("application/x-protobuf"));

        let resp = self
            .execute_request("uploadDeviceConfig", None, Some(&bytes), headers)
            .await?;
        if let Some(payload) = resp.payload {
            Ok(payload.upload_device_config_response)
        } else {
            Ok(None)
        }
    }

    async fn request_auth_token(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        let form_params = {
            let mut params = HashMap::new();
            self.append_default_auth_params(&mut params);
            self.append_auth_params(&mut params);
            params.insert(
                "service",
                String::from("oauth2:https://www.googleapis.com/auth/googleplay"),
            );
            params
        };

        let headers = {
            let mut headers = HashMap::new();
            let build_device = self
                .device_properties
                .android_checkin
                .clone()
                .build
                .as_ref()
                .unwrap()
                .device
                .as_ref()
                .unwrap()
                .clone();
            let build_id = self
                .device_properties
                .extra_info
                .get("Build.ID")
                .unwrap()
                .clone();
            self.append_auth_headers(&mut headers, build_device, build_id);
            headers.insert("content-length", String::from("0"));
            headers
        };

        let bytes = self
            .execute_request_helper("auth", Some(form_params), Some(&[]), headers, false)
            .await?;

        let reply = parse_form_reply(&std::str::from_utf8(&bytes.to_vec()).unwrap());
        self.auth_token = reply.get("auth").map(|a| a.clone());
        Ok(())
    }

    async fn toc(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        let resp = self
            .execute_request("toc", None, None, self.get_default_headers()?)
            .await?;
        let toc_response = resp
            .payload
            .ok_or(Box::new(GpapiError::from("Invalid payload.")))?
            .toc_response
            .ok_or(Box::new(GpapiError::from("Invalid toc response.")))?;
        if toc_response.tos_token.is_some() || toc_response.tos_content.is_some() {
            self.tos_token = toc_response.tos_token.clone();
            return Err(Box::new(GpapiError::new(GpapiErrorKind::TermsOfService)));
        }
        if let Some(cookie) = toc_response.cookie {
            self.dfe_cookie = Some(cookie.clone());
            Ok(())
        } else {
            Err("No DFE cookie found.".into())
        }
    }

    /// Accept the play store terms of service.
    pub async fn accept_tos(
        &mut self,
    ) -> Result<Option<AcceptTosResponse>, Box<dyn Error + Send + Sync>> {
        if let Some(tos_token) = &self.tos_token {
            let form_body = {
                let mut params = HashMap::new();
                params.insert(String::from("tost"), tos_token.clone());
                params.insert(String::from("toscme"), String::from("false"));
                form_post(&params)
            };

            let resp = self
                .execute_request(
                    "acceptTos",
                    None,
                    Some(&form_body.into_bytes()),
                    self.get_default_headers()?,
                )
                .await?;
            if let Some(payload) = resp.payload {
                Ok(payload.accept_tos_response)
            } else {
                Ok(None)
            }
        } else {
            Err("ToS token must be set by `toc` call first.".into())
        }
    }

    /// Lower level Play Store request, used by APIs but exposed for specialized
    /// requests. Returns a `ResponseWrapper` which depending on the request
    /// populates different fields/values.
    async fn execute_request(
        &self,
        endpoint: &str,
        query: Option<HashMap<&str, String>>,
        msg: Option<&[u8]>,
        headers: HashMap<&str, String>,
    ) -> Result<ResponseWrapper, Box<dyn Error + Send + Sync>> {
        let bytes = self
            .execute_request_helper(endpoint, query, msg, headers, true)
            .await?;
        let resp = ResponseWrapper::decode(&mut Cursor::new(bytes))?;
        Ok(resp)
    }

    //async fn execute_request_helper_hyper(
    //    &self,
    //    endpoint: &str,
    //    query: Option<HashMap<&str, String>>,
    //    msg: Option<&[u8]>,
    //    headers: HashMap<&str, String>,
    //    fdfe: bool,
    //) -> Result<Bytes, Box<dyn Error>> {
    //    let query = if let Some(query) = query {
    //        format!("?{}", query
    //            .iter()
    //            .map(|(k, v)| format!("{}={}", k, v))
    //            .collect::<Vec<String>>()
    //            .join("&")
    //        )
    //    } else {
    //        String::from("")
    //    };

    //    let url = if fdfe {
    //        format!("{}/fdfe/{}{}", consts::defaults::DEFAULT_BASE_URL, endpoint, query)
    //    } else {
    //        format!("{}/{}{}", consts::defaults::DEFAULT_BASE_URL, endpoint, query)
    //    };

    //    let mut req = if let Some(msg) = msg {
    //        Request::builder()
    //            .method(Method::POST)
    //            .uri(url)
    //            .body(Body::from(msg.to_owned()))
    //            .unwrap()
    //    } else {
    //        Request::builder()
    //            .method(Method::GET)
    //            .uri(url)
    //            .body(Body::empty())
    //            .unwrap()
    //    };
    //    let hyper_headers = req.headers_mut();

    //    for (key, val) in headers {
    //        hyper_headers.insert(HyperHeaderName::from_bytes(key.as_bytes())?, HyperHeaderValue::from_str(&val)?);
    //    }

    //    let res = self.hyper_client.request(req).await?;

    //    let body_bytes = hyper::body::to_bytes(res.into_body()).await?;
    //    Ok(body_bytes)
    //}

    async fn execute_request_helper(
        &self,
        endpoint: &str,
        query: Option<HashMap<&str, String>>,
        msg: Option<&[u8]>,
        headers: HashMap<&str, String>,
        fdfe: bool,
    ) -> Result<Bytes, Box<dyn Error + Send + Sync>> {
        let mut url = if fdfe {
            Url::parse(&format!(
                "{}/fdfe/{}",
                consts::defaults::DEFAULT_BASE_URL,
                endpoint
            ))?
        } else {
            Url::parse(&format!(
                "{}/{}",
                consts::defaults::DEFAULT_BASE_URL,
                endpoint
            ))?
        };

        if let Some(query) = query {
            let mut queries = url.query_pairs_mut();
            for (key, val) in query {
                queries.append_pair(key, &val);
            }
        }

        let mut reqwest_headers = HeaderMap::new();
        for (key, val) in headers {
            reqwest_headers.insert(
                HeaderName::from_bytes(key.as_bytes())?,
                HeaderValue::from_str(&val)?,
            );
        }

        let res = {
            if let Some(msg) = msg {
                (*self.client)
                    .post(url)
                    .headers(reqwest_headers)
                    .body(msg.to_owned())
                    .send()
                    .await?
            } else {
                (*self.client)
                    .get(url)
                    .headers(reqwest_headers)
                    .send()
                    .await?
            }
        };

        Ok(res.bytes().await?)
    }
}

fn parse_form_reply(data: &str) -> HashMap<String, String> {
    let mut form_resp = HashMap::new();
    let lines: Vec<&str> = data.split_terminator('\n').collect();
    for line in lines.iter() {
        let kv: Vec<&str> = line.split_terminator('=').collect();
        form_resp.insert(
            String::from(kv[0]).to_lowercase(),
            String::from(kv[1..].join("=")),
        );
    }
    form_resp
}

#[derive(Debug, Clone)]
struct AuthRequest {
    params: HashMap<String, String>,
}

impl AuthRequest {
    fn new(email: &str, oauth_token: &str) -> Self {
        let mut auth_request = Self::default();
        auth_request
            .params
            .insert(String::from("Email"), String::from(email));
        auth_request
            .params
            .insert(String::from("Token"), String::from(oauth_token));
        auth_request
    }
}

impl Default for AuthRequest {
    fn default() -> Self {
        let mut params = HashMap::new();
        params.insert(
            String::from("lang"),
            String::from(consts::defaults::DEFAULT_LANGUAGE),
        );
        params.insert(
            String::from("google_play_services_version"),
            String::from(consts::defaults::DEFAULT_GOOGLE_PLAY_SERVICES_VERSION),
        );
        params.insert(
            String::from("sdk_version"),
            String::from(consts::defaults::api_user_agent::DEFAULT_SDK),
        );
        params.insert(
            String::from("device_country"),
            String::from(consts::defaults::DEFAULT_COUNTRY_CODE),
        );
        params.insert(String::from("Email"), String::from(""));
        params.insert(
            String::from("service"),
            String::from(consts::defaults::DEFAULT_SERVICE),
        );
        params.insert(String::from("get_accountid"), String::from("1"));
        params.insert(String::from("ACCESS_TOKEN"), String::from("1"));
        params.insert(
            String::from("callerPkg"),
            String::from(consts::defaults::DEFAULT_ANDROID_VENDING),
        );
        params.insert(String::from("add_account"), String::from("1"));
        params.insert(String::from("Token"), String::from(""));
        params.insert(
            String::from("callerSig"),
            String::from(consts::defaults::DEFAULT_CALLER_SIG),
        );
        AuthRequest { params }
    }
}

fn form_post(params: &HashMap<String, String>) -> String {
    params
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<String>>()
        .join("&")
}

#[derive(Debug, Clone)]
struct BuildConfiguration {
    pub finsky_agent: String,
    pub finsky_version: String,
    pub api: String,
    pub version_code: String,
    pub sdk: String,
    pub device: String,
    pub hardware: String,
    pub product: String,
    pub platform_version_release: String,
    pub model: String,
    pub build_id: String,
    pub is_wide_screen: String,
    pub supported_abis: String,
}

impl BuildConfiguration {
    pub fn user_agent(&self) -> String {
        format!("{}/{} (api={},versionCode={},sdk={},device={},hardware={},product={},platformVersionRelease={},model={},buildId={},isWideScreen={},supportedAbis={})",
          self.finsky_agent, self.finsky_version, self.api, self.version_code, self.sdk,
          self.device, self.hardware, self.product,
          self.platform_version_release, self.model, self.build_id,
          self.is_wide_screen, self.supported_abis
        )
    }
}

impl BuildConfiguration {
    fn new(
        finsky_version: &str,
        version_code: &str,
        sdk: &str,
        device: &str,
        hardware: &str,
        product: &str,
        platform_version_release: &str,
        model: &str,
        build_id: &str,
        supported_abis: &str,
    ) -> Self {
        use consts::defaults::api_user_agent::{DEFAULT_API, DEFAULT_IS_WIDE_SCREEN};
        use consts::defaults::DEFAULT_FINSKY_AGENT;

        BuildConfiguration {
            finsky_agent: DEFAULT_FINSKY_AGENT.to_string(),
            finsky_version: finsky_version.to_string(),
            api: DEFAULT_API.to_string(),
            version_code: version_code.to_string(),
            sdk: sdk.to_string(),
            device: device.to_string(),
            hardware: hardware.to_string(),
            product: product.to_string(),
            platform_version_release: platform_version_release.to_string(),
            model: model.to_string(),
            build_id: build_id.to_string(),
            is_wide_screen: DEFAULT_IS_WIDE_SCREEN.to_string(),
            supported_abis: supported_abis.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_form() {
        let form_reply = "FOO=BAR\nbaz=qux";
        let mut expected_reply = HashMap::new();
        expected_reply.insert("baz".to_string(), "qux".to_string());
        expected_reply.insert("foo".to_string(), "BAR".to_string());
        let parsed_form_reply = parse_form_reply(&form_reply);
        assert_eq!(expected_reply, parsed_form_reply);
    }

    mod gpapi {
        use std::env;

        use super::*;
        use googleplay_protobuf::BulkDetailsRequest;

        #[tokio::test]
        async fn test_request_aas_token() {
            if let (Ok(email), Ok(oauth_token)) = (env::var("EMAIL"), env::var("OAUTH_TOKEN")) {
                let mut api = Gpapi::new("ad_g3_pro", &email);
                assert!(api.request_aas_token(oauth_token).await.is_ok());
                assert!(api.aas_token.is_some());
            }
        }

        #[tokio::test]
        async fn test_login() {
            if let (Ok(email), Ok(aas_token)) = (env::var("EMAIL"), env::var("AAS_TOKEN")) {
                let mut api = Gpapi::new("px_7a", &email);
                api.set_aas_token(aas_token);
                assert!(api.login().await.is_ok());
                assert!(api.device_checkin_consistency_token.is_some());
                assert!(api.gsf_id.is_some());
                assert!(api.device_config_token.is_some());
                assert!(api.auth_token.is_some());
                assert!(api.dfe_cookie.is_some() || api.tos_token.is_some());
            }
        }

        #[tokio::test]
        async fn test_details() {
            if let (Ok(email), Ok(aas_token)) = (env::var("EMAIL"), env::var("AAS_TOKEN")) {
                let mut api = Gpapi::new("px_7a", &email);
                api.set_aas_token(aas_token);
                if api.login().await.is_ok() {
                    assert!(api.details("com.viber.voip").await.is_ok());
                }
            }
        }

        #[tokio::test]
        async fn test_bulk_details() {
            if let (Ok(email), Ok(aas_token)) = (env::var("EMAIL"), env::var("AAS_TOKEN")) {
                let mut api = Gpapi::new("px_7a", &email);
                api.set_aas_token(aas_token);
                if api.login().await.is_ok() {
                    let pkg_names = ["com.viber.voip", "com.instagram.android"];
                    assert!(api.bulk_details(&pkg_names).await.is_ok());
                }
            }
        }

        #[tokio::test]
        async fn test_get_download_info() {
            if let (Ok(email), Ok(aas_token)) = (env::var("EMAIL"), env::var("AAS_TOKEN")) {
                let mut api = Gpapi::new("px_7a", &email);
                api.set_aas_token(aas_token);
                if api.login().await.is_ok() {
                    assert!(api.get_download_info("com.viber.voip", None).await.is_ok());
                }
            }
        }

        #[tokio::test]
        async fn test_download() {
            if let (Ok(email), Ok(aas_token)) = (env::var("EMAIL"), env::var("AAS_TOKEN")) {
                let mut api = Gpapi::new("px_7a", &email);
                api.set_aas_token(aas_token);
                if api.login().await.is_ok() {
                    assert!(api
                        .download(
                            "com.instagram.android",
                            None,
                            true,
                            true,
                            &Path::new("/tmp/testing"),
                            None
                        )
                        .await
                        .is_ok());
                }
            }
        }

        #[test]
        fn test_protobuf() {
            let mut bdr = BulkDetailsRequest::default();
            bdr.doc_id = vec!["test".to_string()].into();
            bdr.include_child_docs = Some(true);
        }
    }
}
