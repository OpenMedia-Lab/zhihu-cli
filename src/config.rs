use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

pub const BASE: &str = "https://www.zhihu.com";
pub const V4: &str = "https://www.zhihu.com/api/v4";
pub const ZHUANLAN: &str = "https://zhuanlan.zhihu.com/api";
pub const IMAGE_API: &str = "https://api.zhihu.com/images";
pub const CONTENT_PUBLISH: &str = "https://www.zhihu.com/api/v4/content/publish";
pub const CONTENT_DRAFTS: &str = "https://www.zhihu.com/api/v4/content/drafts";
pub const OSS_UPLOAD: &str = "https://zhihu-pics-upload.zhimg.com";
pub const LOGIN_URL: &str = "https://www.zhihu.com/signin";
pub const QRCODE_API: &str = "https://www.zhihu.com/api/v3/account/api/login/qrcode";
pub const CAPTCHA_URL: &str = "https://www.zhihu.com/api/v3/oauth/captcha/v2?type=captcha_sign_in";
pub const UDID_URL: &str = "https://www.zhihu.com/udid";

pub const REQUIRED_COOKIES: [&str; 3] = ["z_c0", "_xsrf", "d_c0"];

// Single consistent Chrome fingerprint across UA / sec-ch-ua / sec-fetch-*.
const CHROME_VERSION: &str = "145";
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/145.0.0.0 Safari/537.36";

pub fn browser_headers() -> Vec<(&'static str, String)> {
    vec![
        ("User-Agent", USER_AGENT.into()),
        ("Accept", "application/json, text/plain, */*".into()),
        ("Accept-Language", "en-US,en;q=0.9".into()),
        ("Referer", format!("{BASE}/")),
        (
            "sec-ch-ua",
            format!(
                "\"Not:A-Brand\";v=\"99\", \"Google Chrome\";v=\"{CHROME_VERSION}\", \"Chromium\";v=\"{CHROME_VERSION}\""
            ),
        ),
        ("sec-ch-ua-mobile", "?0".into()),
        ("sec-ch-ua-platform", "\"Windows\"".into()),
    ]
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("zhihu-cli")
}

pub fn cookie_file() -> PathBuf {
    config_dir().join("cookies.json")
}

pub fn load_cookies() -> Option<BTreeMap<String, String>> {
    let data = fs::read_to_string(cookie_file()).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&data).ok()?;
    let obj = parsed.get("cookies")?.as_object()?;
    Some(
        obj.iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
            .collect(),
    )
}

pub fn save_cookies(cookies: &BTreeMap<String, String>) -> anyhow::Result<()> {
    let dir = config_dir();
    fs::create_dir_all(&dir)?;
    let body = serde_json::json!({ "cookies": cookies });
    fs::write(cookie_file(), serde_json::to_string_pretty(&body)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(cookie_file(), fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

pub fn clear_cookies() -> bool {
    cookie_file().exists() && fs::remove_file(cookie_file()).is_ok()
}

pub fn has_required(cookies: &BTreeMap<String, String>) -> bool {
    REQUIRED_COOKIES.iter().all(|k| cookies.contains_key(*k))
}

pub fn cookie_header(cookies: &BTreeMap<String, String>) -> String {
    cookies
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("; ")
}
