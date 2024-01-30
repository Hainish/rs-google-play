// These are obtained from reversing the Play Store and extracting the public key components from the pem
/// Google Play Public Key (base64 encoded)
pub const GOOGLE_PUB_KEY_B64: &'static str = "AAAAgMom/1a/v0lblO2Ubrt60J2gcuXSljGFQXgcyZWveWLEwo6prwgi3iJIZdodyhKZQrNWp5nKJ3srRXcUW+F1BD3baEVGcmEgqaLZUNBjm057pKRI16kB0YppeGx5qIQ5QjKzsR8ETQbKLNWgRY0QRNVz34kMJR3P/LgHax/6rmf5AAAAAwEAAQ==";
/// Exact ciphersuite specification is needed, see:
/// https://stackoverflow.com/questions/22832104/how-can-i-see-hidden-app-data-in-google-drive
/// Updated version
pub const GOOGLE_ACCEPTED_CIPHERS: &'static str = "TLS_AES_256_GCM_SHA384:TLS_CHACHA20_POLY1305_SHA256:TLS_AES_128_GCM_SHA256:ECDHE+AESGCM:ECDHE+CHACHA20:DHE-RSA-AES256-GCM-SHA384:DHE-RSA-AES128-GCM-SHA256:DHE-RSA-CHACHA20-POLY1305:ECDHE-ECDSA-AES256-CCM8:ECDHE-ECDSA-AES256-CCM:ECDHE-ECDSA-AES128-CCM8:ECDHE-ECDSA-AES128-CCM:ECDHE-ECDSA-AES256-SHA384:ECDHE-RSA-AES256-SHA384:ECDHE-ECDSA-AES128-SHA256:ECDHE-RSA-AES128-SHA256:ECDHE-ECDSA-AES256-SHA:ECDHE-RSA-AES256-SHA:ECDHE-ECDSA-AES128-SHA:ECDHE-RSA-AES128-SHA:DHE+CHACHA20:ECDH+AESGCM:DH+AESGCM:ECDH+AES:DH+AES:RSA+AESGCM:RSA+AES:!aNULL:!eNULL:!MD5:!DSS";
pub const SUPPORTED_ELLIPTIC_CURVE_GROUPS: &'static str = "X25519:prime256v1:X448:secp521r1:secp384r1";

pub mod defaults {
    pub const DEFAULT_LANGUAGE: &str = "en_US";
    pub const DEFAULT_CLIENT_SIG: &str = "38918a453d07199354f8b19af05ec6562ced5788";
    pub const DEFAULT_CALLER_SIG: &str = "38918a453d07199354f8b19af05ec6562ced5788";
    pub const DEFAULT_DROIDGUARD_RESULTS: &str = "dummy123";
    pub const DEFAULT_COUNTRY_CODE: &str = "us";
    pub const DEFAULT_AUTH_USER_AGENT: &str = "GoogleAuth/1.4";
    pub mod api_user_agent {
        pub const DEFAULT_API: &str = "3";
        pub const DEFAULT_VERSION_CODE: &str = "81053300";
        pub const DEFAULT_SDK: &str = "27";
        pub const DEFAULT_DEVICE: &str = "hero2lte";
        pub const DEFAULT_HARDWARE: &str = "samsungexynos8890";
        pub const DEFAULT_PRODUCT: &str = "hero2ltexx";
        pub const DEFAULT_PLATFORM_VERSION_RELEASE: &str = "8.1.0";
        pub const DEFAULT_MODEL: &str = "SM-G935F";
        pub const DEFAULT_BUILD_ID: &str = "OPM2.171019.029.B1";
        pub const DEFAULT_IS_WIDE_SCREEN: &str = "0";
        pub const DEFAULT_SUPPORTED_ABIS: &str = "arm64-v8a;armeabi-v7a;armeabi";
    }
    pub const DEFAULT_FINSKY_AGENT: &str = "Android-Finsky";
    pub const DEFAULT_FINSKY_VERSION: &str = "10.5.33-all [0] [PR] 201016072";
    pub const DEFAULT_DFE_TARGETS: &str = "CAEScFfqlIEG6gUYogFWrAISK1WDAg+hAZoCDgIU1gYEOIACFkLMAeQBnASLATlASUuyAyqCAjY5igOMBQzfA/IClwFbApUC4ANbtgKVAS7OAX8YswHFBhgDwAOPAmGEBt4OfKkB5weSB5AFASkiN68akgMaxAMSAQEBA9kBO7UBFE1KVwIDBGs3go6BBgEBAgMECQgJAQIEAQMEAQMBBQEBBAUEFQYCBgUEAwMBDwIBAgOrARwBEwMEAg0mrwESfTEcAQEKG4EBMxghChMBDwYGASI3hAEODEwXCVh/EREZA4sBYwEdFAgIIwkQcGQRDzQ2fTC2AjfVAQIBAYoBGRg2FhYFBwEqNzACJShzFFblAo0CFxpFNBzaAd0DHjIRI4sBJZcBPdwBCQGhAUd2A7kBLBVPngEECHl0UEUMtQETigHMAgUFCc0BBUUlTywdHDgBiAJ+vgKhAU0uAcYCAWQ/5ALUAw1UwQHUBpIBCdQDhgL4AY4CBQICjARbGFBGWzA1CAEMOQH+BRAOCAZywAIDyQZ2MgM3BxsoAgUEBwcHFia3AgcGTBwHBYwBAlcBggFxSGgIrAEEBw4QEqUCASsWadsHCgUCBQMD7QICA3tXCUw7ugJZAwGyAUwpIwM5AwkDBQMJA5sBCw8BNxBVVBwVKhebARkBAwsQEAgEAhESAgQJEBCZATMdzgEBBwG8AQQYKSMUkAEDAwY/CTs4/wEaAUt1AwEDAQUBAgIEAwYEDx1dB2wGeBFgTQ";
    pub const DEFAULT_DEVICE_COUNTRY: &str = "en";
    pub const DEFAULT_ANDROID_VENDING: &str = "com.google.android.gms";
    pub const DEFAULT_ACCOUNT_TYPE: &str = "HOSTED_OR_GOOGLE";
    pub const DEFAULT_GOOGLE_PLAY_SERVICES_VERSION: &str = "12866025";
    pub const DEFAULT_SERVICE: &str = "ac2dm";
    pub const DEFAULT_BASE_URL: &str = "https://android.clients.google.com";
}
