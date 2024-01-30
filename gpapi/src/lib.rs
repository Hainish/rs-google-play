//! A library for interacting with the Google Play API, strongly following [google play python API](https://github.com/NoMore200/googleplay-api.git) patterns.
//!
//! # Getting Started
//!
//! Interacting with the API starts off with initializing the API and logging in.
//!
//! ```rust
//! use gpapi::Gpapi;
//!
//! #[tokio::main]
//! async fn main() {
//!     let mut gpa = Gpapi::new("en_US", "UTC", "hero2lte");
//!     gpa.login("someone@gmail.com", "somepass").await;
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
//! # let mut gpa = Gpapi::new("en_US", "UTC", "hero2lte");
//! # gpa.login("someone@gmail.com", "somepass").await;
//! let details = gpa.details("com.instagram.android").await;
//! println!("{:?}", details);
//!
//! let download_info = gpa.get_download_info("com.instagram.android", None).await;
//! println!("{:?}", download_info);
//!
//! gpa.download("com.instagram.android", None, true, true, &Path::new("/tmp/testing"), None).await;
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

use base64::{Engine as _, engine::general_purpose as b64_general_purpose};
use bytes::Bytes;
use futures::future::TryFutureExt;
use hyper::client::HttpConnector;
use hyper::header::{HeaderName as HyperHeaderName, HeaderValue as HyperHeaderValue};
use hyper::{Body, Client, Method, Request};
use hyper_openssl::HttpsConnector;
use openssl::ssl::{SslConnector, SslMethod};
use prost::Message;
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::Url;
use tokio_dl_stream_to_disk::AsyncDownload;

use crate::error::{Error as GpapiError, ErrorKind as GpapiErrorKind};

use googleplay_protobuf::{
    AndroidCheckinProto, AndroidCheckinRequest, AndroidCheckinResponse, BulkDetailsRequest,
    BulkDetailsResponse, DetailsResponse, DeviceConfigurationProto, ResponseWrapper,
    UploadDeviceConfigRequest, UploadDeviceConfigResponse,
};

#[macro_use]
extern crate lazy_static;

static DEVICES_ENCODED: &[u8] = include_bytes!("device_properties.bin");
static CHECKINS_ENCODED: &[u8] = include_bytes!("android_checkins.bin");
lazy_static! {
    static ref DEVICE_CONFIGURATIONS: HashMap<String, Vec<u8>> =
        bincode::deserialize(DEVICES_ENCODED).unwrap();
    static ref ANDROID_CHECKINS: HashMap<String, Vec<u8>> =
        bincode::deserialize(CHECKINS_ENCODED).unwrap();
}

type MainAPKDownloadURL = Option<String>;
type SplitsDownloadInfo = Vec<(Option<String>, Option<String>)>;
type AdditionalFilesDownloadInfo = Vec<(Option<String>, Option<String>)>;
type DownloadInfo = (MainAPKDownloadURL, SplitsDownloadInfo, AdditionalFilesDownloadInfo);

/// The Gpapi object is the sole way to interact with the Play Store API.  It abstracts the logic
/// of low-level communication with Google's Play Store servers.
#[derive(Debug)]
pub struct Gpapi {
    locale: String,
    timezone: String,
    device_codename: String,
    pub(crate) auth_subtoken: Option<String>,
    pub(crate) device_config_token: Option<String>,
    pub(crate) device_checkin_consistency_token: Option<String>,
    dfe_cookie: Option<String>,
    gsf_id: Option<i64>,
    client: Box<reqwest::Client>,
    hyper_client: Box<hyper::Client<HttpsConnector<HttpConnector>>>,
}

impl Gpapi {
    /// Returns a Gpapi struct with locale, timezone, and the device codename specified.
    ///
    /// # Arguments
    ///
    /// * `locale` - A string type specifying the device locale, e.g. "en_US"
    /// * `timezone` - A string type specifying the timezone , e.g. "UTC"
    /// * `device_codename` - A string type specifying the device codename, e.g. "hero2lte"
    pub fn new<S: Into<String>>(locale: S, timezone: S, device_codename: S) -> Self {
        let mut http = HttpConnector::new();
        http.enforce_http(false);
        let mut connector = SslConnector::builder(SslMethod::tls()).unwrap();
        connector
            .set_cipher_list(consts::GOOGLE_ACCEPTED_CIPHERS)
            .unwrap();
	// using just required supported elliptic curve groups
	connector
            .set_groups_list(consts::SUPPORTED_ELLIPTIC_CURVE_GROUPS)
            .unwrap();
        let https = HttpsConnector::with_connector(http, connector).unwrap();
        let hyper_client = Client::builder().build::<_, hyper::Body>(https);

        Gpapi {
            locale: locale.into(),
            timezone: timezone.into(),
            device_codename: device_codename.into(),
            auth_subtoken: None,
            device_config_token: None,
            device_checkin_consistency_token: None,
            dfe_cookie: None,
            gsf_id: None,
            client: Box::new(reqwest::Client::new()),
            hyper_client: Box::new(hyper_client),
        }
    }

    /// Log in to Google's Play Store API.  This is required for most other actions.
    ///
    /// # Arguments
    ///
    /// * `username` - A string type specifying the login username, usually a full email
    /// * `password` - A string type specifying an app password, created from your Google account
    /// settings.
    pub async fn login<S: Into<String> + Clone>(
        &mut self,
        username: S,
        password: S,
    ) -> Result<(), GpapiError> {
        let username = username.into();
        let login = encrypt_login(&username, &password.into())?;
        let encrypted_password = b64_general_purpose::URL_SAFE_NO_PAD.encode(&login);
        let form = self.authenticate(&username, &encrypted_password).await?;
        if let Some(err) = form.get("error") {
            if err == "NeedsBrowser" {
                return Err(GpapiError::new(GpapiErrorKind::SecurityCheck));
            }
        }
        if let Some(token) = form.get("auth") {
            let token = token.to_string();
            self.gsf_id = self.checkin(&username, &token).await?;
            self.get_auth_subtoken(&username, &encrypted_password)
                .await?;
            if let Some(upload_device_config_token) = self.upload_device_config().await? {
                self.device_config_token =
                    Some(upload_device_config_token.upload_device_config_token.unwrap());
                Ok(())
            } else {
                Err("No device config token".into())
            }
        } else {
            Err("No GSF auth token".into())
        }
    }

    async fn checkin(
        &mut self,
        username: &str,
        ac2dm_token: &str,
    ) -> Result<Option<i64>, Box<dyn Error>> {
        let mut checkin = ANDROID_CHECKINS
            .get(&self.device_codename)
            .map(|raw| {
                let raw = raw.clone();
                AndroidCheckinProto::decode(&mut Cursor::new(raw)).unwrap()
            })
            .expect("Invalid device codename");

        checkin.build.as_mut().map(|mut b| {
            b.timestamp = Some(
                (SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
                    / 1000) as i64,
            )
        });

        let mut req = AndroidCheckinRequest::default();
        req.id = Some(0);
        req.checkin = Some(checkin);
        req.locale = Some(self.locale.clone());
        req.time_zone = Some(self.timezone.clone());
        req.version = Some(3);
        req.device_configuration = DEVICE_CONFIGURATIONS
            .get(&self.device_codename)
            .map(|raw| {
                let raw = raw.clone();
                DeviceConfigurationProto::decode(&mut Cursor::new(raw)).unwrap()
            });
        req.fragment = Some(0);
        let mut req_followup = req.clone();
        let mut bytes = Vec::new();
        bytes.reserve(req.encoded_len());
        req.encode(&mut bytes).unwrap();
        let resp = self.execute_checkin_request(&bytes).await?;
        self.device_checkin_consistency_token = resp.device_checkin_consistency_token;

        // checkin again to upload gfsid
        req_followup.id = resp.android_id.map(|id| id as i64);
        req_followup.security_token = resp.security_token;
        req_followup.account_cookie.push(format!("[{}]", username));
        req_followup.account_cookie.push(ac2dm_token.to_string());
        let mut bytes = Vec::new();
        bytes.reserve(req_followup.encoded_len());
        req_followup.encode(&mut bytes).unwrap();
        let resp = self.execute_checkin_request(&bytes).await?;
        Ok(resp.android_id.map(|id| id as i64))
    }

    async fn upload_device_config(
        &self,
    ) -> Result<Option<UploadDeviceConfigResponse>, Box<dyn Error>> {
        let mut req = UploadDeviceConfigRequest::default();
        req.device_configuration = DEVICE_CONFIGURATIONS
            .get(&self.device_codename)
            .map(|raw| {
                let raw = raw.clone();
                DeviceConfigurationProto::decode(&mut Cursor::new(raw)).unwrap()
            });
        let mut bytes = Vec::new();
        bytes.reserve(req.encoded_len());
        req.encode(&mut bytes).unwrap();

        let mut headers = HeaderMap::new();
        headers.insert(
            "X-DFE-Enabled-Experiments",
            HeaderValue::from_static("cl:billing.select_add_instrument_by_default"),
        );
        headers.insert(
            "X-DFE-Unsupported-Experiments",
            HeaderValue::from_static("nocache:billing.use_charging_poller,market_emails,buyer_currency,prod_baseline,checkin.set_asset_paid_app_field,shekel_test,content_ratings,buyer_currency_in_app,nocache:encrypted_apk,recent_changes"));
        headers.insert(
            "X-DFE-SmallestScreenWidthDp",
            HeaderValue::from_static("320"),
        );
        headers.insert("X-DFE-Filter-Level", HeaderValue::from_static("3"));
        let resp = self
            .execute_request_v2("uploadDeviceConfig", None, Some(&bytes), headers)
            .await?;
        if let Some(payload) = resp.payload {
            Ok(payload.upload_device_config_response)
        } else {
            Ok(None)
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
    ) -> Result<Vec<()>, GpapiError> {
        let pkg_name = pkg_name.into();
        let download_info = self
            .get_download_info(pkg_name.clone(), version_code)
            .await?;

        let mut dst_path = PathBuf::from(dst_path);
        if dst_path.is_dir() {
            if (split_if_available && download_info.1.len() > 0) ||
               (include_additional_files && download_info.2.len() > 0){
                dst_path.push(pkg_name.clone());
                if dst_path.is_dir() {
                    return Err(GpapiError::new(GpapiErrorKind::DirectoryExists));
                } else {
                    fs::create_dir(&dst_path).map_err(|e| GpapiError::from(e))?;
                }
            }
        } else {
            return Err(GpapiError::new(GpapiErrorKind::DirectoryMissing));
        }

        let mut downloads = Vec::new();
        let err = |e| GpapiError::from(e);
        if include_additional_files && download_info.2.len() > 0 {
            for additional_file in download_info.2 {
                if let (Some(filename), Some(download_url)) = additional_file {
                    let dl = AsyncDownload::new(&download_url, &dst_path, &filename).get().await?;
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
                    let dl = AsyncDownload::new(&download_url, &dst_path, &filename).get().await?;
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
            let dl = AsyncDownload::new(&download_url, &dst_path, &filename).get().await?;
            let length = dl.length();
            let cb = match length {
                Some(length) => cb.map(|c| c(filename.clone(), length)),
                None => None,
            };
            downloads.push((dl, cb));
        } else {
            return Err("Could not download app - no download URL available".into())
        }
        futures::future::try_join_all(downloads.iter_mut().map(|(d, c)| d.download(c).map_err(err))).await
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
    ) -> Result<DownloadInfo, GpapiError> {
        let pkg_name = pkg_name.into();
        if self.auth_subtoken.is_none() {
            return Err("Logging in is required for this action".into());
        }
        if version_code.is_none() {
            version_code = Some(self.get_latest_version_for_pkg_name(&pkg_name).await?);
        }
        let resp = {
            let version_code_str = version_code.unwrap().to_string();
            let mut req = HashMap::new();
            req.insert("ot", "1");
            req.insert("doc", &pkg_name);
            req.insert("vc", &version_code_str);
            self.execute_request_v2("purchase", Some(req), None, HeaderMap::new())
                .await?
        };
        if let Some(payload) = resp.payload {
            if let Some(buy_response) = payload.buy_response {
                if let Some(download_token) = buy_response.download_token {
                    return self
                        .delivery(&pkg_name, version_code.clone(), &download_token)
                        .await;
                }
            }
        }
        Err(GpapiError::new(GpapiErrorKind::InvalidApp))
    }

    async fn delivery<S: Into<String>>(
        &self,
        pkg_name: S,
        mut version_code: Option<i32>,
        download_token: S,
    ) -> Result<DownloadInfo, GpapiError> {
        let pkg_name = pkg_name.into();
        let download_token = download_token.into();
        if self.auth_subtoken.is_none() {
            return Err("Logging in is required for this action".into());
        }
        if version_code.is_none() {
            version_code = Some(self.get_latest_version_for_pkg_name(&pkg_name).await?);
        }
        let resp = {
            let version_code_str = version_code.unwrap().to_string();
            let mut req = HashMap::new();
            req.insert("ot", "1");
            req.insert("doc", &pkg_name);
            req.insert("vc", &version_code_str);
            req.insert("dtok", &download_token);
            self.execute_request_v2("delivery", Some(req), None, HeaderMap::new())
                .await?
        };
        if let Some(payload) = resp.payload {
            if let Some(delivery_response) = payload.delivery_response {
                if let Some(app_delivery_data) = delivery_response.app_delivery_data {
                    let mut splits = Vec::new();
                    for app_split in app_delivery_data.split {
                        splits.push((app_split.name, app_split.download_url));
                    }
                    let mut additional_files: Vec<(Option<String>, Option<String>)> = Vec::new();
                    for additional_file in app_delivery_data.additional_file {
                        if let Some(file_type) = additional_file.file_type {
                            if let Some(version_code) = additional_file.version_code {
                                let main_patch = match file_type {
                                    0 => "main",
                                    _ => "patch",
                                };
                                let filename = format!("{}.{}.{}.obb", main_patch, version_code, pkg_name);
                                additional_files.push((Some(filename), additional_file.download_url));
                            }
                        }
                    }
                    return Ok((app_delivery_data.download_url, splits, additional_files));
                }
            }
        }
        Err(GpapiError::new(GpapiErrorKind::InvalidApp))
    }

    /// Play Store package detail request (provides more detail than bulk requests).
    ///
    /// # Arguments
    ///
    /// * `pkg_name` - A string type specifying the package's app ID, e.g. `com.instagram.android`
    pub async fn details<S: Into<String>>(
        &self,
        pkg_name: S,
    ) -> Result<Option<DetailsResponse>, GpapiError> {
        let pkg_name = pkg_name.into();
        let mut req = HashMap::new();
        req.insert("doc", &pkg_name[..]);
        let resp = self
            .execute_request_v2("details", Some(req), None, HeaderMap::new())
            .await?;
        if let Some(payload) = resp.payload {
            Ok(payload.details_response)
        } else {
            Ok(None)
        }
    }

    async fn get_latest_version_for_pkg_name(&self, pkg_name: &str) -> Result<i32, GpapiError> {
        if let Some(details) = self.details(pkg_name).await? {
            if let Some(doc_v2) = details.doc_v2 {
                if let Some(details) = doc_v2.details {
                    if let Some(app_details) = details.app_details {
                        if let Some(version_code) = app_details.version_code {
                            return Ok(version_code);
                        }
                    }
                }
            }
        }
        Err(GpapiError::new(GpapiErrorKind::InvalidApp))
    }

    /// Play Store bulk detail request for multiple apps.
    ///
    /// # Arguments
    ///
    /// * `pkg_names` - An array of string types specifying package app IDs
    pub async fn bulk_details(
        &self,
        pkg_names: &[&str],
    ) -> Result<Option<BulkDetailsResponse>, GpapiError> {
        let mut req = BulkDetailsRequest::default();
        req.docid = pkg_names.into_iter().cloned().map(String::from).collect();
        req.include_child_docs = Some(false);
        let mut bytes = Vec::new();
        bytes.reserve(req.encoded_len());
        req.encode(&mut bytes).unwrap();
        let resp = self
            .execute_request_v2("bulkDetails", None, Some(&bytes), HeaderMap::new())
            .await?;
        if let Some(payload) = resp.payload {
            Ok(payload.bulk_details_response)
        } else {
            Ok(None)
        }
    }

    async fn get_auth_subtoken(
        &mut self,
        username: &str,
        encrypted_password: &str,
    ) -> Result<(), Box<dyn Error>> {
        let mut login_req = build_login_request(username, encrypted_password);
        login_req
            .params
            .insert(String::from("service"), String::from("androidmarket"));
        login_req
            .params
            .insert(String::from("app"), String::from("com.android.vending"));
        let second_login_req = login_req.clone();

        let reply = self.authenticate_helper(&login_req).await?;
        if let Some(master_token) = reply.get("token") {
            self.auth_subtoken = self
                .get_second_round_token(master_token, second_login_req)
                .await?;
        }
        Ok(())
    }

    async fn get_second_round_token(
        &self,
        master_token: &str,
        mut login_req: LoginRequest,
    ) -> Result<Option<String>, Box<dyn Error>> {
        if let Some(gsf_id) = self.gsf_id {
            login_req
                .params
                .insert(String::from("androidId"), format!("{:x}", gsf_id));
        }
        login_req
            .params
            .insert(String::from("Token"), String::from(master_token));
        login_req
            .params
            .insert(String::from("check_email"), String::from("1"));
        login_req.params.insert(
            String::from("token_request_options"),
            String::from("CAA4AQ=="),
        );
        login_req
            .params
            .insert(String::from("system_partition"), String::from("1"));
        login_req.params.insert(
            String::from("_opt_is_called_from_account_manager"),
            String::from("1"),
        );
        login_req.params.remove("Email");
        login_req.params.remove("EncryptedPasswd");
        let reply = self.authenticate_helper(&login_req).await?;
        Ok(reply.get("auth").map(|a| String::from(a)))
    }

    /// Handles authenticating with Google Play Store, retrieving a set of tokens from
    /// the server that can be used for future requests.
    async fn authenticate(
        &self,
        username: &str,
        encrypted_password: &str,
    ) -> Result<HashMap<String, String>, Box<dyn Error>> {
        let login_req = build_login_request(username, encrypted_password);

        self.authenticate_helper(&login_req).await
    }

    async fn authenticate_helper(
        &self,
        login_req: &LoginRequest,
    ) -> Result<HashMap<String, String>, Box<dyn Error>> {
        let form_body = login_req.form_post();

        let mut req = Request::builder()
            .method(Method::POST)
            .uri(format!("{}/{}", consts::defaults::DEFAULT_BASE_URL, "auth"))
            .body(Body::from(form_body))
            .unwrap();
        let headers = req.headers_mut();
        headers.insert(
            hyper::header::USER_AGENT,
            HyperHeaderValue::from_str(&consts::defaults::DEFAULT_AUTH_USER_AGENT)?,
        );
        headers.insert(
            hyper::header::CONTENT_TYPE,
            HyperHeaderValue::from_static("application/x-www-form-urlencoded; charset=UTF-8"),
        );
        if let Some(gsf_id) = &self.gsf_id {
            headers.insert(
                HyperHeaderName::from_static("device"),
                HyperHeaderValue::from_str(&format!("{:x}", gsf_id))?,
            );
            headers.insert(
                HyperHeaderName::from_static("app"),
                HyperHeaderValue::from_static("com.android.vending"),
            );
        }

        let res = self.hyper_client.request(req).await?;

        let body_bytes = hyper::body::to_bytes(res.into_body()).await?;
        let reply = parse_form_reply(&std::str::from_utf8(&body_bytes.to_vec()).unwrap());
        Ok(reply)
    }

    /// Lower level Play Store request, used by APIs but exposed for specialized
    /// requests. Returns a `ResponseWrapper` which depending on the request
    /// populates different fields/values.
    async fn execute_request_v2(
        &self,
        endpoint: &str,
        query: Option<HashMap<&str, &str>>,
        msg: Option<&[u8]>,
        headers: HeaderMap,
    ) -> Result<ResponseWrapper, Box<dyn Error>> {
        let bytes = self
            .execute_request_helper(endpoint, query, msg, headers, true)
            .await?;
        let resp = ResponseWrapper::decode(&mut Cursor::new(bytes))?;
        Ok(resp)
    }

    async fn execute_checkin_request(
        &self,
        msg: &[u8],
    ) -> Result<AndroidCheckinResponse, Box<dyn Error>> {
        let bytes = self
            .execute_request_helper("checkin", None, Some(msg), HeaderMap::new(), false)
            .await?;
        let resp = AndroidCheckinResponse::decode(&mut Cursor::new(bytes))?;
        Ok(resp)
    }

    async fn execute_request_helper(
        &self,
        endpoint: &str,
        query: Option<HashMap<&str, &str>>,
        msg: Option<&[u8]>,
        mut headers: HeaderMap,
        fdfe: bool,
    ) -> Result<Bytes, Box<dyn Error>> {
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

        let config = BuildConfiguration {
            ..Default::default()
        };

        headers.insert(
            reqwest::header::ACCEPT_LANGUAGE,
            HeaderValue::from_str(&self.locale.replace("_", "-"))?,
        );
        headers.insert(
            reqwest::header::USER_AGENT,
            HeaderValue::from_str(&config.user_agent())?,
        );
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            HeaderValue::from_static("application/x-protobuf"),
        );
        headers.insert(
            "X-DFE-Encoded-Targets",
            HeaderValue::from_static(consts::defaults::DEFAULT_DFE_TARGETS),
        );
        headers.insert(
            "X-DFE-Client-Id",
            HeaderValue::from_static("am-android-google"),
        );
        headers.insert(
            "X-DFE-MCCMCN",
            HeaderValue::from_str(
                &ANDROID_CHECKINS
                    .get(&self.device_codename)
                    .map(|raw| {
                        let raw = raw.clone();
                        let checkin = AndroidCheckinProto::decode(&mut Cursor::new(raw)).unwrap();
                        checkin.cell_operator.clone().unwrap()
                    })
                    .unwrap(),
            )?,
        );
        headers.insert("X-DFE-Network-Type", HeaderValue::from_static("4"));
        headers.insert("X-DFE-Content-Filters", HeaderValue::from_static(""));
        headers.insert(
            "X-DFE-Request-Params",
            HeaderValue::from_static("timeoutMs=4000"),
        );
        if let Some(gsf_id) = &self.gsf_id {
            headers.insert(
                "X-DFE-Device-Id",
                HeaderValue::from_str(&format!("{:x}", gsf_id))?,
            );
        }
        if let Some(auth_subtoken) = &self.auth_subtoken {
            headers.insert(
                "Authorization",
                HeaderValue::from_str(&format!("GoogleLogin auth={}", auth_subtoken))?,
            );
        }
        if let Some(device_config_token) = &self.device_config_token {
            headers.insert(
                "X-DFE-Device-Config-Token",
                HeaderValue::from_str(&device_config_token)?,
            );
        }
        if let Some(device_checkin_consistency_token) = &self.device_checkin_consistency_token {
            headers.insert(
                "X-DFE-Device-Checkin-Consistency-Token",
                HeaderValue::from_str(&device_checkin_consistency_token)?,
            );
        }
        if let Some(dfe_cookie) = &self.dfe_cookie {
            headers.insert("X-DFE-Cookie", HeaderValue::from_str(&dfe_cookie)?);
        }

        let query2 = query.clone();
        if let Some(query) = query {
            let mut queries = url.query_pairs_mut();
            for (key, val) in query {
                queries.append_pair(key, val);
            }
        }

        let res = if endpoint == "purchase" {
            (*self.client)
                .post(url)
                .headers(headers)
                .form(&query2.unwrap())
                .send()
                .await?
        } else {
            if let Some(msg) = msg {
                (*self.client)
                    .post(url)
                    .headers(headers)
                    .body(msg.to_owned())
                    .send()
                    .await?
            } else {
                (*self.client).get(url).headers(headers).send().await?
            }
        };

        Ok(res.bytes().await?)
    }
}

#[derive(Debug)]
struct PubKey {
    pub modulus: Vec<u8>,
    pub exp: Vec<u8>,
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

/// Handles encrypting your login/password using Google's public key
/// Produces something of the format:
/// |00|4 bytes of sha1(publicKey)|rsaEncrypt(publicKeyPem, "login\x00password")|
fn encrypt_login(login: &str, password: &str) -> Result<Vec<u8>, GpapiError> {
    let raw = b64_general_purpose::STANDARD.decode(consts::GOOGLE_PUB_KEY_B64).unwrap();
    let pubkey = extract_pubkey(&raw)?.ok_or("Could not extract public key")?;
    let rsa = build_openssl_rsa(&pubkey);

    let data = format!("{login}\x00{password}", login = login, password = password);
    if data.as_bytes().len() >= 87 {
        return Err(GpapiError::new(GpapiErrorKind::EncryptLogin));
    }

    let mut to = vec![0u8; rsa.size() as usize];
    let padding = openssl::rsa::Padding::PKCS1_OAEP;

    rsa.public_encrypt(data.as_bytes(), &mut to, padding)
        .unwrap();
    let sha1 = openssl::sha::sha1(&raw);
    let mut res = vec![];
    res.push(0x00);
    res.extend(&sha1[0..4]);
    res.extend(&to);
    Ok(res)
}

///
/// Gen up an `openssl::rsa::Rsa` from a `PubKey`.
///
fn build_openssl_rsa(p: &PubKey) -> openssl::rsa::Rsa<openssl::pkey::Public> {
    use openssl::bn::BigNum;
    use openssl::rsa::Rsa;

    let modulus = BigNum::from_hex_str(&hex::encode(&p.modulus)).unwrap();
    let exp = BigNum::from_hex_str(&hex::encode(&p.exp)).unwrap();
    let rsa = Rsa::from_public_components(modulus, exp).unwrap();

    rsa
}

///
/// Extract public key (PEM) from a raw buffer.
///
fn extract_pubkey(buf: &[u8]) -> Result<Option<PubKey>, Box<dyn Error>> {
    use byteorder::{NetworkEndian, ReadBytesExt};
    use std::io::Read;
    let mut cur = Cursor::new(&buf);

    let sz = cur.read_u32::<NetworkEndian>()?;
    let mut modulus = vec![0u8; sz as usize];
    cur.read_exact(&mut modulus)?;

    let sz = cur.read_u32::<NetworkEndian>()?;
    let mut exp = vec![0u8; sz as usize];
    cur.read_exact(&mut exp)?;

    Ok(Some(PubKey { modulus, exp }))
}

#[derive(Debug, Clone)]
struct LoginRequest {
    params: HashMap<String, String>,
    build_config: Option<BuildConfiguration>,
}

impl LoginRequest {
    pub fn form_post(&self) -> String {
        self.params
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<String>>()
            .join("&")
    }
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

impl Default for BuildConfiguration {
    fn default() -> BuildConfiguration {
        use consts::defaults::api_user_agent::{
            DEFAULT_API, DEFAULT_BUILD_ID, DEFAULT_DEVICE, DEFAULT_HARDWARE,
            DEFAULT_IS_WIDE_SCREEN, DEFAULT_MODEL, DEFAULT_PLATFORM_VERSION_RELEASE,
            DEFAULT_PRODUCT, DEFAULT_SDK, DEFAULT_SUPPORTED_ABIS, DEFAULT_VERSION_CODE,
        };
        use consts::defaults::{DEFAULT_FINSKY_AGENT, DEFAULT_FINSKY_VERSION};

        BuildConfiguration {
            finsky_agent: DEFAULT_FINSKY_AGENT.to_string(),
            finsky_version: DEFAULT_FINSKY_VERSION.to_string(),
            api: DEFAULT_API.to_string(),
            version_code: DEFAULT_VERSION_CODE.to_string(),
            sdk: DEFAULT_SDK.to_string(),
            device: DEFAULT_DEVICE.to_string(),
            hardware: DEFAULT_HARDWARE.to_string(),
            product: DEFAULT_PRODUCT.to_string(),
            platform_version_release: DEFAULT_PLATFORM_VERSION_RELEASE.to_string(),
            model: DEFAULT_MODEL.to_string(),
            build_id: DEFAULT_BUILD_ID.to_string(),
            is_wide_screen: DEFAULT_IS_WIDE_SCREEN.to_string(),
            supported_abis: DEFAULT_SUPPORTED_ABIS.to_string(),
        }
    }
}

impl Default for LoginRequest {
    fn default() -> Self {
        let mut params = HashMap::new();
        params.insert(String::from("Email"), String::from(""));
        params.insert(String::from("EncryptedPasswd"), String::from(""));
        params.insert(String::from("add_account"), String::from("1"));
        params.insert(
            String::from("accountType"),
            String::from(consts::defaults::DEFAULT_ACCOUNT_TYPE),
        );
        params.insert(
            String::from("google_play_services_version"),
            String::from(consts::defaults::DEFAULT_GOOGLE_PLAY_SERVICES_VERSION),
        );
        params.insert(String::from("has_permission"), String::from("1"));
        params.insert(String::from("source"), String::from("android"));
        params.insert(
            String::from("device_country"),
            String::from(consts::defaults::DEFAULT_DEVICE_COUNTRY),
        );
        params.insert(
            String::from("operatorCountry"),
            String::from(consts::defaults::DEFAULT_COUNTRY_CODE),
        );
        params.insert(
            String::from("lang"),
            String::from(consts::defaults::DEFAULT_LANGUAGE),
        );
        params.insert(
            String::from("client_sig"),
            String::from(consts::defaults::DEFAULT_CLIENT_SIG),
        );
        params.insert(
            String::from("callerSig"),
            String::from(consts::defaults::DEFAULT_CALLER_SIG),
        );
        params.insert(
            String::from("droidguard_results"),
            String::from(consts::defaults::DEFAULT_DROIDGUARD_RESULTS),
        );
        params.insert(
            String::from("service"),
            String::from(consts::defaults::DEFAULT_SERVICE),
        );
        params.insert(
            String::from("callerPkg"),
            String::from(consts::defaults::DEFAULT_ANDROID_VENDING),
        );
        LoginRequest {
            params,
            build_config: None,
        }
    }
}

fn build_login_request(username: &str, encrypted_password: &str) -> LoginRequest {
    let encrypted_password = String::from(encrypted_password);
    let build_config = BuildConfiguration {
        ..Default::default()
    };
    let mut login_request = LoginRequest::default();
    login_request.build_config = Some(build_config);
    login_request
        .params
        .insert(String::from("Email"), String::from(username));
    login_request.params.insert(
        String::from("EncryptedPasswd"),
        String::from(encrypted_password),
    );
    login_request
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login() {
        let enc = encrypt_login("foo", "bar").unwrap();
        assert!(b64_general_purpose::STANDARD.encode(&enc).starts_with("AFcb4K"));
        assert_eq!(b64_general_purpose::STANDARD.encode(&enc).len(), 180);
        assert!(!b64_general_purpose::URL_SAFE_NO_PAD.encode(&enc).contains("/"));
    }

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

        use super::Gpapi;
        use googleplay_protobuf::BulkDetailsRequest;

        #[tokio::test]
        #[ignore]
        async fn create_gpapi() {
            match (env::var("GOOGLE_LOGIN"), env::var("GOOGLE_PASSWORD")) {
                (Ok(username), Ok(password)) => {
                    let mut api = Gpapi::new("en_US", "UTC", "hero2lte");
                    api.login(username, password).await.ok();
                    assert!(api.auth_subtoken.is_some());
                    assert!(api.device_config_token.is_some());
                    assert!(api.device_checkin_consistency_token.is_some());

                    assert!(api.details("com.viber.voip").await.is_ok());
                    let pkg_names = ["com.viber.voip", "air.WatchESPN"];
                    assert!(api.bulk_details(&pkg_names).await.is_ok());
                }
                _ => panic!("require login/password for test"),
            }
        }

        #[test]
        fn test_protobuf() {
            let mut bdr = BulkDetailsRequest::default();
            bdr.docid = vec!["test".to_string()].into();
            bdr.include_child_docs = Some(true);
        }
    }
}
