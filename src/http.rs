use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use reqwest::blocking::{Client as Reqwest, Response};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::Method;

use crate::config;

const TIMEOUT: Duration = Duration::from_secs(20);

/// A minimal session: fixed browser fingerprint headers + a flat cookie map
/// absorbed from every Set-Cookie. Cookie scoping is deliberately simple —
/// the jar only ever talks to zhihu hosts.
pub struct Session {
    pub http: Reqwest,
    pub cookies: BTreeMap<String, String>,
    pub extra_headers: Vec<(String, String)>,
}

impl Session {
    pub fn new() -> Self {
        let http = Reqwest::builder()
            .timeout(TIMEOUT)
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .expect("http client");
        Self {
            http,
            cookies: BTreeMap::new(),
            extra_headers: Vec::new(),
        }
    }

    /// Load the saved cookie file; None when never logged in.
    pub fn from_saved() -> Result<Self> {
        let cookies =
            config::load_cookies().context("not logged in — run `zhihu login` or pass --cookie")?;
        let mut s = Self::new();
        s.cookies = cookies;
        Ok(s)
    }

    fn absorb(&mut self, resp: &Response) {
        for value in resp.headers().get_all(reqwest::header::SET_COOKIE) {
            let Ok(text) = value.to_str() else { continue };
            let Some((pair, _)) = text.split_once(';') else {
                continue;
            };
            let Some((name, val)) = pair.trim().split_once('=') else {
                continue;
            };
            if !name.is_empty() {
                self.cookies
                    .insert(name.trim().to_string(), val.trim().to_string());
            }
        }
    }

    fn build_headers(&self) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (k, v) in config::browser_headers() {
            map.insert(
                HeaderName::from_bytes(k.as_bytes()).unwrap(),
                HeaderValue::from_str(&v).unwrap(),
            );
        }
        for (k, v) in &self.extra_headers {
            if let (Ok(name), Ok(value)) = (
                HeaderName::from_bytes(k.as_bytes()),
                HeaderValue::from_str(v),
            ) {
                map.insert(name, value);
            }
        }
        if let Some(xsrf) = self.cookies.get("_xsrf") {
            map.insert("x-xsrftoken", HeaderValue::from_str(xsrf).unwrap());
        }
        if !self.cookies.is_empty() {
            map.insert(
                reqwest::header::COOKIE,
                HeaderValue::from_str(&config::cookie_header(&self.cookies)).unwrap(),
            );
        }
        map
    }

    pub fn request(
        &mut self,
        method: Method,
        url: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<Response> {
        let mut req = self.http.request(method, url).headers(self.build_headers());
        if let Some(b) = body {
            req = req.json(b);
        }
        let resp = req.send().context("request failed")?;
        self.absorb(&resp);
        Ok(resp)
    }

    /// Raw-body request (form posts etc.) with session headers + cookies.
    pub fn request_raw(
        &mut self,
        method: Method,
        url: &str,
        body: Vec<u8>,
        extra: &[(&str, &str)],
    ) -> Result<Response> {
        let mut req = self
            .http
            .request(method, url)
            .headers(self.build_headers())
            .body(body);
        for (k, v) in extra {
            req = req.header(*k, *v);
        }
        let resp = req.send().context("request failed")?;
        self.absorb(&resp);
        Ok(resp)
    }

    /// Like `request`, with extra one-off headers that don't persist.
    pub fn request_with(
        &mut self,
        method: Method,
        url: &str,
        body: Option<&serde_json::Value>,
        extra: &[(&str, &str)],
    ) -> Result<Response> {
        let mut req = self.http.request(method, url).headers(self.build_headers());
        for (k, v) in extra {
            req = req.header(*k, *v);
        }
        if let Some(b) = body {
            req = req.json(b);
        }
        let resp = req.send().context("request failed")?;
        self.absorb(&resp);
        Ok(resp)
    }

    /// JSON request with Zhihu's status conventions folded into anyhow errors.
    pub fn json(
        &mut self,
        method: Method,
        url: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<serde_json::Value> {
        let resp = self.request(method, url, body)?;
        let status = resp.status();
        if status == 401 {
            bail!("not logged in or session expired — run `zhihu login`");
        }
        if status == 403 {
            let text = resp.text().unwrap_or_default();
            bail!(
                "access denied (403): {}",
                text.chars().take(200).collect::<String>()
            );
        }
        if !status.is_success() {
            let text = resp.text().unwrap_or_default();
            bail!(
                "API {}: {}",
                status,
                text.chars().take(200).collect::<String>()
            );
        }
        let text = resp.text().context("read API response")?;
        if text.trim().is_empty() {
            return Ok(serde_json::Value::Null);
        }
        serde_json::from_str(&text).context("API returned invalid JSON")
    }

    pub fn get(&mut self, url: &str) -> Result<serde_json::Value> {
        self.json(Method::GET, url, None)
    }

    pub fn post(&mut self, url: &str, body: &serde_json::Value) -> Result<serde_json::Value> {
        self.json(Method::POST, url, Some(body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn serve(status: &str, body: &str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            let _ = stream.read(&mut request);
            stream.write_all(response.as_bytes()).unwrap();
        });
        url
    }
    #[test]
    fn unicode_error_bodies_do_not_panic() {
        let error = Session::new()
            .get(&serve("403 Forbidden", &"拒绝".repeat(100)))
            .unwrap_err();
        assert!(error.to_string().contains("access denied"));
    }
    #[test]
    fn rejects_invalid_json_in_success_response() {
        assert!(Session::new()
            .get(&serve("200 OK", "not JSON"))
            .unwrap_err()
            .to_string()
            .contains("invalid JSON"));
    }
    #[test]
    fn accepts_empty_success_response() {
        assert_eq!(
            Session::new().get(&serve("204 No Content", "")).unwrap(),
            serde_json::Value::Null
        );
    }
}
