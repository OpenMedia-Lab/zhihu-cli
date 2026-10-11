use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use reqwest::Method;

use crate::config;
use crate::http::Session;

/// `zhihu login` — QR scan (default), `--cookie` paste, or `--browser` extract.
pub fn login(cookie: Option<&str>, browser: Option<&str>) -> Result<()> {
    let cookies = if let Some(raw) = cookie {
        login_with_cookie(raw)?
    } else if let Some(name) = browser {
        login_with_browser(name)?
    } else {
        login_qr()?
    };
    config::save_cookies(&cookies)?;
    eprintln!(
        "logged in — cookies saved to {}",
        config::cookie_file().display()
    );
    Ok(())
}

fn login_with_cookie(raw: &str) -> Result<BTreeMap<String, String>> {
    let mut cookies = BTreeMap::new();
    for part in raw.split(';') {
        let part = part.trim();
        if let Some((k, v)) = part.split_once('=') {
            cookies.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    if !cookies.contains_key("z_c0") {
        bail!("cookie string missing z_c0 — copy the full Cookie header from a logged-in zhihu.com request");
    }
    // z_c0 alone isn't enough; pull _xsrf / d_c0 from a homepage visit.
    let mut s = Session::new();
    s.cookies = cookies.clone();
    let _ = s.request(Method::GET, &format!("{}/", config::BASE), None);
    cookies.extend(s.cookies.clone());
    Ok(cookies)
}

#[cfg(feature = "browser-cookies")]
fn login_with_browser(name: &str) -> Result<BTreeMap<String, String>> {
    let found = rookie::load(Some(vec!["zhihu.com".to_string()]))
        .map_err(|e| anyhow::anyhow!("browser cookie extraction failed: {e}"))?;
    let mut cookies = BTreeMap::new();
    for c in found {
        cookies.insert(c.name, c.value);
    }
    let _ = name; // rookie picks the first browser that has the cookies
    if !cookies.contains_key("z_c0") {
        bail!("no zhihu.com session found in installed browsers — log in there first");
    }
    let mut s = Session::new();
    s.cookies = cookies.clone();
    let _ = s.request(Method::GET, &format!("{}/", config::BASE), None);
    cookies.extend(s.cookies);
    Ok(cookies)
}

#[cfg(not(feature = "browser-cookies"))]
fn login_with_browser(_name: &str) -> Result<BTreeMap<String, String>> {
    bail!("this build has no browser-cookies feature — use QR login or --cookie")
}

/// QR login ported from the browser flow: signin cookies → udid → captcha →
/// qrcode token → render → poll scan_info until z_c0 lands.
fn login_qr() -> Result<BTreeMap<String, String>> {
    let mut s = Session::new();
    s.extra_headers = vec![
        ("Referer".into(), config::LOGIN_URL.into()),
        ("Origin".into(), config::BASE.into()),
        ("x-requested-with".into(), "fetch".into()),
    ];

    let _ = s.request(Method::GET, config::LOGIN_URL, None);
    let _ = s.request(Method::POST, config::UDID_URL, Some(&serde_json::json!({})));
    let _ = s.request(Method::GET, config::CAPTCHA_URL, None);

    let resp = s.post(config::QRCODE_API, &serde_json::json!({}))?;
    let token = resp
        .get("token")
        .or_else(|| resp.get("qrcode_token"))
        .and_then(|v| v.as_str())
        .context("QR API returned no token")?
        .to_string();
    let link = resp
        .get("link")
        .and_then(|v| v.as_str())
        .context("QR API returned no link")?
        .to_string();

    eprintln!("scan with the Zhihu app to log in:\n");
    render_qr(&link);
    eprintln!("\nif the QR is unreadable, open this link on your phone:\n{link}");
    eprintln!("\nwaiting for confirmation …");

    // Poll — the official contract: status 0 = unscanned, 1 = scanned pending,
    // access_token/user_id in the body = confirmed.
    s.extra_headers = vec![
        (
            "Referer".into(),
            format!("{}/signin?next=%2F", config::BASE),
        ),
        ("Accept".into(), "*/*".into()),
        ("sec-fetch-dest".into(), "empty".into()),
        ("sec-fetch-mode".into(), "cors".into()),
        ("sec-fetch-site".into(), "same-origin".into()),
        ("x-requested-with".into(), "fetch".into()),
        ("x-zse-93".into(), "101_3_3.0".into()),
    ];
    let scan_url = format!("{}/{}/scan_info", config::QRCODE_API, token);
    let deadline = Instant::now() + Duration::from_secs(120);
    while Instant::now() < deadline {
        sleep(Duration::from_millis(800));
        let Ok(resp) = s.request(Method::GET, &scan_url, None) else {
            continue;
        };
        if s.cookies.contains_key("z_c0") {
            break;
        }
        if !resp.status().is_success() {
            continue;
        }
        let info: serde_json::Value = resp.json().unwrap_or_default();
        match info.get("status").and_then(|v| v.as_i64()) {
            Some(0) | Some(1) => continue,
            _ => {}
        }
        if info.get("access_token").is_some() || info.get("user_id").is_some() {
            break;
        }
        for key in ["cookie", "cookies"] {
            if let Some(raw) = info.get(key).and_then(|v| v.as_str()) {
                if raw.contains("z_c0") {
                    for part in raw.split(';') {
                        if let Some((k, v)) = part.trim().split_once('=') {
                            s.cookies.insert(k.trim().into(), v.trim().into());
                        }
                    }
                }
            }
        }
        if let Some(z) = info.get("z_c0").and_then(|v| v.as_str()) {
            s.cookies.insert("z_c0".into(), z.to_string());
        }
        if s.cookies.contains_key("z_c0") {
            break;
        }
    }

    // The scan-info success may not Set-Cookie everything; /me completes the jar.
    let _ = s.request(Method::GET, &format!("{}/me", config::V4), None);

    if !config::has_required(&s.cookies) {
        bail!("QR login timed out or was not confirmed (no z_c0)");
    }
    Ok(s.cookies)
}

fn render_qr(data: &str) {
    let Ok(code) = qrcode::QrCode::new(data.as_bytes()) else {
        return;
    };
    let m = code.to_colors();
    let w = code.width();
    let border = 2usize;
    for y in (0..w + border * 2).step_by(2) {
        let mut line = String::new();
        for x in 0..w + border * 2 {
            let top = cell(&m, w, x, y, border);
            let bot = cell(&m, w, x, y + 1, border);
            line.push(match (top, bot) {
                (false, false) => ' ',
                (true, false) => '▀',
                (false, true) => '▄',
                (true, true) => '█',
            });
        }
        eprintln!("{line}");
    }
}

fn cell(m: &[qrcode::Color], w: usize, x: usize, y: usize, border: usize) -> bool {
    if x < border || y < border || x >= w + border || y >= w + border {
        return false;
    }
    m[(y - border) * w + (x - border)] == qrcode::Color::Dark
}

pub fn logout() {
    if config::clear_cookies() {
        eprintln!("logged out — {}", config::cookie_file().display());
    } else {
        eprintln!("no saved session");
    }
}

/// Interactive fallback inside `login --cookie` when the arg is "-": read stdin.
pub fn read_cookie_stdin() -> Result<String> {
    eprint!("paste Cookie header, then Enter: ");
    std::io::stderr().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim().to_string())
}
