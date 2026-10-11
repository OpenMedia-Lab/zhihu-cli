use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use base64::Engine;
use hmac::{Hmac, Mac};
use reqwest::Method;
use serde_json::{json, Value};

use crate::config;
use crate::http::Session;

// ===== identity =====

pub fn me(s: &mut Session) -> Result<Value> {
    s.get(&format!("{}/me", config::V4))
}

// ===== reads =====

fn member_list(
    s: &mut Session,
    url_token: &str,
    kind: &str,
    include: &str,
    offset: u32,
    limit: u32,
) -> Result<Value> {
    s.get(&format!(
        "{}/members/{}/{}?include={}&offset={}&limit={}&sort_by=created",
        config::V4,
        url_token,
        kind,
        urlencoding(include),
        offset,
        limit
    ))
}

pub fn user_articles(s: &mut Session, url_token: &str, offset: u32, limit: u32) -> Result<Value> {
    member_list(
        s,
        url_token,
        "articles",
        "data[*].content,voteup_count,comment_count,created,updated",
        offset,
        limit,
    )
}

pub fn user_answers(s: &mut Session, url_token: &str, offset: u32, limit: u32) -> Result<Value> {
    member_list(
        s,
        url_token,
        "answers",
        "data[*].content,voteup_count,comment_count,created,question",
        offset,
        limit,
    )
}

pub fn user_pins(s: &mut Session, url_token: &str, offset: u32, limit: u32) -> Result<Value> {
    s.get(&format!(
        "{}/members/{}/pins?offset={}&limit={}",
        config::V4,
        url_token,
        offset,
        limit
    ))
}

pub fn article(s: &mut Session, id: &str) -> Result<Value> {
    // zhuanlan serves the full record; v4 is the fallback shape.
    match s.get(&format!("{}/articles/{}", config::ZHUANLAN, id)) {
        Ok(v) => Ok(v),
        Err(_) => s.get(&format!("{}/articles/{}", config::V4, id)),
    }
}

pub fn answer(s: &mut Session, id: &str) -> Result<Value> {
    s.get(&format!(
        "{}/answers/{}?include={}",
        config::V4,
        id,
        urlencoding("content,voteup_count,comment_count,created,updated,author,question")
    ))
}

pub fn question(s: &mut Session, id: &str) -> Result<Value> {
    s.get(&format!(
        "{}/questions/{}?include={}",
        config::V4, id,
        urlencoding("data[*].author,answer_count,follower_count,visit_count,comment_count,created,updated_time,detail,topics")
    ))
}

pub fn pin(s: &mut Session, id: &str) -> Result<Value> {
    match s.get(&format!("{}/pins/{}", config::V4, id)) {
        Ok(v) => Ok(v),
        Err(_) => s.get(&format!("https://api.zhihu.com/pins/{}", id)),
    }
}

/// `read <id-or-url>` — dispatch on URL shape, bare ids try article → answer → pin.
pub fn read(s: &mut Session, id_or_url: &str) -> Result<Value> {
    let t = id_or_url.trim();
    if let Some(id) = t.strip_prefix("https://zhuanlan.zhihu.com/p/") {
        return article(s, id.trim_matches('/'));
    }
    if t.contains("zhihu.com/question/") {
        let rest = t.split("question/").nth(1).unwrap_or("");
        let qid = rest.split('/').next().unwrap_or("");
        if let Some(aid) = t.split("/answer/").nth(1) {
            return answer(s, aid.trim_matches('/').split('?').next().unwrap_or(""));
        }
        return question(s, qid.split('?').next().unwrap_or(""));
    }
    if t.contains("zhihu.com/pin/") || t.contains("/pins/") {
        let id = t
            .split("pin")
            .nth(1)
            .unwrap_or("")
            .trim_matches(|c| c == '/' || c == 's');
        return pin(s, id.split('?').next().unwrap_or(id));
    }
    let bare = t.trim_matches('/').split('?').next().unwrap_or(t);
    article(s, bare)
        .or_else(|_| answer(s, bare))
        .or_else(|_| pin(s, bare))
        .context("no article, answer, or pin found for that id")
}

pub fn search(s: &mut Session, kw: &str, kind: &str, limit: u32) -> Result<Value> {
    s.get(&format!(
        "{}/search_v3?gk_version=gz-gaokao&t={}&q={}&correction=1&offset=0&limit={}&lc_idx=0&search_source=Normal",
        config::V4,
        kind,
        urlencoding(kw),
        limit
    ))
}

/// Resolve topic names → ids. The editor's picker hits the topic suggest API.
pub fn topics(s: &mut Session, kw: &str) -> Result<Value> {
    match s.get(&format!(
        "{}/topics/autocomplete?token={}&max_matches=10",
        config::V4,
        urlencoding(kw)
    )) {
        Ok(v) if v.as_array().map(|a| !a.is_empty()).unwrap_or(false) => Ok(v),
        _ => search(s, kw, "topic", 10),
    }
}

pub fn hot(s: &mut Session, limit: u32) -> Result<Value> {
    s.get(&format!(
        "{}/creators/rank/hot?domain=0&limit={}",
        config::V4,
        limit
    ))
}

fn urlencoding(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

// ===== image upload =====

/// Upload a local image file: register md5 → OSS signed PUT → durable URL.
///
/// The durable URL is derivable (`pic*.zhimg.com/{object_key}`) once the PUT
/// lands — zhihu's `status` field may sit at `init` for a long while, so we
/// only do a short best-effort poll for the richer `src` fields.
pub fn upload_image(s: &mut Session, path: &str, source: &str) -> Result<Value> {
    let data = std::fs::read(path).with_context(|| format!("image not found: {path}"))?;
    let md5_hex = format!("{:x}", md5::compute(&data));

    let reg = s.post(
        config::IMAGE_API,
        &json!({ "image_hash": md5_hex, "source": source }),
    )?;
    let uf = reg
        .get("upload_file")
        .context("no upload_file in response")?;
    let image_id = uf
        .get("image_id")
        .map(|v| {
            v.as_str()
                .map(str::to_string)
                .unwrap_or_else(|| v.to_string())
        })
        .context("no image_id")?;
    let state = uf.get("state").and_then(|v| v.as_i64()).unwrap_or(-1);
    let object_key = uf
        .get("object_key")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if state == 2 {
        oss_put(
            &object_key,
            &data,
            reg.get("upload_token").context("no upload_token")?,
        )?;
    } else if state != 1 {
        bail!("unexpected image state: {state}");
    }

    // GIFs keep their extension in the object key.
    let mut key = object_key.clone();
    if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        key.push_str(".gif");
    }

    // Zhihu flips `status` to "success" once ingest completes; the response's
    // `source` query param is what unlocks the public zhimg URL.
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut info = Value::Null;
    while Instant::now() < deadline {
        if let Ok(v) = s.get(&format!("{}/{}", config::IMAGE_API, image_id)) {
            info = v;
            let st = info.get("status").and_then(|x| x.as_str());
            if st == Some("success")
                || st == Some("completed")
                || info.get("original_hash").is_some()
            {
                break;
            }
        }
        sleep(Duration::from_secs(2));
    }

    let src_param = info
        .get("src")
        .and_then(|v| v.as_str())
        .and_then(|u| u.split("source=").nth(1))
        .and_then(|q| q.split('&').next())
        .unwrap_or("1f5c5e47")
        .to_string();
    let canonical = format!("https://pic4.zhimg.com/{key}?source={src_param}");

    let (w, h) = imagesize::size(path)
        .map(|d| (d.width as i64, d.height as i64))
        .unwrap_or((0, 0));
    Ok(json!({
        "src": canonical,
        "signed_src": info.get("src").cloned().unwrap_or(Value::Null),
        "original_src": info.get("original_src").cloned().unwrap_or(json!(canonical)),
        "watermark": info.get("watermark").cloned().unwrap_or(json!("watermark")),
        "watermark_src": info.get("watermark_src").cloned().unwrap_or(json!("")),
        "width": w,
        "height": h,
        "object_key": key,
    }))
}

/// Rehost a remote image URL via zhihu's server-side fetch — no OSS roundtrip.
pub fn upload_image_url(s: &mut Session, url: &str, source: &str) -> Result<Value> {
    let body = format!("url={}&source={}", urlencoding(url), urlencoding(source));
    let resp = s.request_raw(
        Method::POST,
        &format!("{}/uploaded_images", config::ZHUANLAN),
        body.into_bytes(),
        &[
            ("Content-Type", "application/x-www-form-urlencoded"),
            ("x-requested-with", "fetch"),
        ],
    )?;
    if !resp.status().is_success() {
        bail!(
            "remote image rehost {}: {}",
            resp.status(),
            resp.text().unwrap_or_default()
        );
    }
    let v: Value = resp.json().unwrap_or_default();
    let src = v
        .get("src")
        .and_then(|x| x.as_str())
        .context("no src in response")?;
    // Reduce the draft-scoped signed URL to the durable public form.
    let src_param = src
        .split("source=")
        .nth(1)
        .and_then(|q| q.split('&').next())
        .map(|s| format!("?source={s}"))
        .unwrap_or_default();
    let hash = src
        .split('/')
        .find_map(|p| {
            p.split(['~', '.'])
                .next()
                .and_then(|h| h.strip_prefix("v2-"))
                .map(|h| format!("v2-{h}"))
        })
        .filter(|h| h.len() > 4 && src.contains(".zhihu.com/"));
    let canonical = match hash {
        Some(h) => format!("https://pic4.zhimg.com/{h}.jpg{src_param}"),
        None => src.to_string(),
    };
    Ok(json!({ "src": canonical, "original_src": src }))
}

fn oss_put(obj_key: &str, data: &[u8], token: &Value) -> Result<()> {
    let content_type = guess_image_mime(data);
    let date = httpdate::fmt_http_date(SystemTime::now());
    let security_token = token["access_token"].as_str().context("oss token")?;
    let access_id = token["access_id"].as_str().context("oss access_id")?;
    let access_key = token["access_key"].as_str().context("oss access_key")?;
    let oss_ua = "aliyun-sdk-js/6.8.0";

    // Canonical oss headers, sorted — must match the signed set exactly.
    let canon = format!(
        "x-oss-date:{date}\nx-oss-security-token:{security_token}\nx-oss-user-agent:{oss_ua}"
    );
    let to_sign = format!("PUT\n\n{content_type}\n{date}\n{canon}\n/zhihu-pics/{obj_key}");
    let mut mac = Hmac::<sha1::Sha1>::new_from_slice(access_key.as_bytes())?;
    mac.update(to_sign.as_bytes());
    let sig = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());

    let resp = reqwest::blocking::Client::new()
        .put(format!("{}/{}", config::OSS_UPLOAD, obj_key))
        .header("Content-Type", content_type)
        .header("x-oss-date", date)
        .header("x-oss-security-token", security_token)
        .header("x-oss-user-agent", oss_ua)
        .header("Authorization", format!("OSS {access_id}:{sig}"))
        .body(data.to_vec())
        .send()
        .context("OSS upload failed")?;
    if !resp.status().is_success() {
        bail!(
            "OSS upload {}: {}",
            resp.status(),
            resp.text().unwrap_or_default()
        );
    }
    Ok(())
}

fn guess_image_mime(data: &[u8]) -> &'static str {
    if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        "image/gif"
    } else if data.starts_with(&[0x89, b'P', b'N', b'G']) {
        "image/png"
    } else if data.starts_with(b"RIFF") && data.len() > 11 && &data[8..12] == b"WEBP" {
        "image/webp"
    } else {
        "image/jpeg"
    }
}

fn img_html(infos: &[Value]) -> String {
    let mut out = String::new();
    for i in infos {
        let src = i["src"].as_str().unwrap_or("");
        out.push_str(&format!(
            "<img src=\"{src}\" data-caption=\"\" data-size=\"normal\" data-rawwidth=\"{}\" data-rawheight=\"{}\"/>",
            i["width"].as_i64().unwrap_or(0),
            i["height"].as_i64().unwrap_or(0),
        ));
    }
    out
}

// ===== write: article (draft-first) =====

/// Create an article. `publish: false` (the default, `--draft`) stops after the
/// draft update and returns the edit URL — the preview-first path.
pub fn create_article(
    s: &mut Session,
    title: &str,
    content: &str,
    topics: &[String],
    images: &[String],
    publish: bool,
) -> Result<Value> {
    let mut html = content.to_string();
    if !images.is_empty() {
        let mut infos = Vec::new();
        for p in images {
            infos.push(upload_image(s, p, "article")?);
        }
        html.push_str(&img_html(&infos));
    }

    // Step 1: create draft
    let draft = s.post(&format!("{}/articles/drafts", config::ZHUANLAN), &json!({}))?;
    let id = match draft.get("id") {
        Some(Value::String(s)) => s.clone(),
        Some(v) => v.to_string(),
        None => bail!("draft created but no id returned"),
    };

    // Step 2: fill title/content. NOTE: zhihu binds article topics at publish
    // time via the editor — the draft API does not store them.
    let _ = topics;
    let patch = json!({ "title": title, "content": html });
    // Zhuanlan PATCH can answer 200 with an empty body; tolerate that.
    let resp = s.request(
        Method::PATCH,
        &format!("{}/articles/{}/draft", config::ZHUANLAN, id),
        Some(&patch),
    )?;
    if !resp.status().is_success() {
        bail!(
            "update draft {}: {}",
            resp.status(),
            resp.text().unwrap_or_default()
        );
    }

    if !publish {
        return Ok(json!({
            "id": id,
            "state": "draft",
            "edit_url": format!("https://zhuanlan.zhihu.com/p/{}/edit", id),
        }));
    }

    // Step 3: publish (explicit --publish only)
    s.json(
        Method::PUT,
        &format!("{}/articles/{}/publish", config::ZHUANLAN, id),
        Some(&json!({ "column": null, "commentPermission": "anyone" })),
    )?;
    Ok(json!({
        "id": id,
        "state": "published",
        "url": format!("https://zhuanlan.zhihu.com/p/{}", id),
    }))
}

/// Read a draft back — verification for the preview path.
pub fn read_draft(s: &mut Session, id: &str) -> Result<Value> {
    s.get(&format!("{}/articles/{}/draft", config::ZHUANLAN, id))
}

// ===== write: pin (想法) — publishes immediately, zhihu has no pin preview =====

pub fn create_pin(s: &mut Session, title: &str, content: &str, images: &[String]) -> Result<Value> {
    let mut infos = Vec::new();
    for p in images {
        infos.push(upload_image(s, p, "pin")?);
    }

    let draft = s.post(config::CONTENT_DRAFTS, &json!({ "action": "pin" }))?;
    let draft_id = draft
        .pointer("/data/content_id")
        .context("content draft returned no content_id")?
        .clone();

    let millis = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let mut data = json!({
        "publish": { "traceId": format!("{},{}", millis, uuid::Uuid::new_v4()) },
        "commentsPermission": { "comment_permission": "all" },
        "extra_info": { "view_permission": "all", "publisher": "pc" },
        "draft": { "disabled": 1, "id": draft_id },
        "title": { "title": title },
        "hybrid": {
            "html": if content.is_empty() { img_html(&infos) } else { format!("{content}{}", img_html(&infos)) },
            "textLength": title.chars().count() + content.chars().count(),
        },
    });
    if !infos.is_empty() {
        let medias: Vec<Value> = infos
            .iter()
            .map(|i| {
                json!({ "image": {
                    "width": i["width"], "height": i["height"],
                    "url": i["src"], "originalUrl": i["original_src"],
                    "watermark": i["watermark"], "watermarkUrl": i["watermark_src"],
                }})
            })
            .collect();
        data["media"] = json!({ "medias": medias });
    }

    let resp = s.request_with(
        Method::POST,
        config::CONTENT_PUBLISH,
        Some(&json!({ "action": "pin", "data": data })),
        &[("x-requested-with", "fetch")],
    )?;
    let status = resp.status();
    let body: Value = resp.json().unwrap_or(Value::Null);
    if !status.is_success() {
        bail!("pin publish {}: {}", status, body);
    }
    if body
        .get("code")
        .and_then(|c| c.as_i64())
        .map(|c| c != 0)
        .unwrap_or(false)
    {
        bail!(
            "pin publish failed: {}",
            body.get("message")
                .or(body.get("toast_message"))
                .and_then(|m| m.as_str())
                .unwrap_or("unknown")
        );
    }
    match body.pointer("/data/result").and_then(|r| r.as_str()) {
        Some(r) => Ok(serde_json::from_str(r).unwrap_or(body)),
        None => Ok(body),
    }
}

// ===== delete =====

pub fn delete_article(s: &mut Session, id: &str) -> Result<()> {
    let resp = s.request(
        Method::DELETE,
        &format!("{}/articles/{}", config::ZHUANLAN, id),
        None,
    )?;
    if !resp.status().is_success() {
        bail!("delete article {}", resp.status());
    }
    Ok(())
}

pub fn delete_pin(s: &mut Session, id: &str) -> Result<()> {
    let resp = s.request(Method::DELETE, &format!("{}/pins/{}", config::V4, id), None)?;
    if !resp.status().is_success() {
        bail!("delete pin {}", resp.status());
    }
    Ok(())
}
