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

    let mut key = object_key.clone();

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
    if key.is_empty() {
        key = metadata_image_key(&info)
            .context("image upload completed without a durable object key")?;
    }
    // GIFs keep their extension, including deduplicated uploads without an object_key.
    if (data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a")) && !key.ends_with(".gif") {
        key.push_str(".gif");
    }
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
    cover: Option<&str>,
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

    let resolved_topics = resolve_topics(s, topics)?;
    let cover_info = cover
        .map(|path| upload_image(s, path, "article"))
        .transpose()?;

    // Step 1: create draft
    let draft = s.post(&format!("{}/articles/drafts", config::ZHUANLAN), &json!({}))?;
    let id = match draft.get("id") {
        Some(Value::String(s)) => s.clone(),
        Some(v) => v.to_string(),
        None => bail!("draft created but no id returned"),
    };

    let mut patch = json!({ "title": title, "content": html });
    if let Some(info) = &cover_info {
        set_cover(&mut patch, info)?;
    }
    // Zhuanlan PATCH can answer 200 with an empty body; tolerate that.
    let resp = s
        .request(
            Method::PATCH,
            &format!("{}/articles/{}/draft", config::ZHUANLAN, id),
            Some(&patch),
        )
        .with_context(|| {
            format!(
                "draft {id} created; update failed; edit https://zhuanlan.zhihu.com/p/{id}/edit"
            )
        })?;
    if !resp.status().is_success() {
        bail!(
            "update draft {id} failed {}: {}",
            resp.status(),
            resp.text().unwrap_or_default()
        );
    }

    bind_topics(s, &id, &resolved_topics)
        .with_context(|| format!("draft {id} created; topic binding failed"))?;
    if !publish {
        return Ok(json!({
            "id": id,
            "state": "draft",
            "edit_url": format!("https://zhuanlan.zhihu.com/p/{}/edit", id),
            "cover": cover_info.as_ref().and_then(|info| info["src"].as_str()).unwrap_or(""),
            "topics": resolved_topics.iter().map(|topic| topic["name"].clone()).collect::<Vec<_>>(),
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
        "cover": cover_info.as_ref().and_then(|info| info["src"].as_str()).unwrap_or(""),
        "topics": resolved_topics.iter().map(|topic| topic["name"].clone()).collect::<Vec<_>>(),
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deduplicated_upload_derives_missing_object_key_from_metadata() {
        assert_eq!(
            metadata_image_key(
                &json!({"src":"https://pic-private.zhihu.com/v2-abc~resize:1440.jpg?auth_key=private"})
            ),
            Some("v2-abc".into())
        );
        assert_eq!(
            metadata_image_key(&json!({"original_hash":"abc"})),
            Some("v2-abc".into())
        );
        assert_eq!(metadata_image_key(&json!({"status":"init"})), None);
    }
    #[test]
    fn topic_binding_uses_numeric_ids() {
        assert_eq!(
            topic_ids(&[json!({"id":"19674381"}), json!({"id":19550517})]).unwrap(),
            vec![19674381, 19550517]
        );
        assert!(topic_ids(&[json!({"id":"invalid"})]).is_err());
    }
    #[test]
    fn cover_uses_signed_editor_url_and_preserves_omitted_fields() {
        let mut patch = article_patch(Some("title"), None);
        set_cover(&mut patch, &json!({"src":"public","signed_src":"signed"})).unwrap();
        assert_eq!(
            patch,
            json!({"title":"title","titleImage":"signed","isTitleImageFullScreen":false})
        );
    }
    #[test]
    fn topic_picker_prefers_exact_name_and_handles_empty_lists() {
        assert_eq!(
            select_topic(
                &json!([{"id":"1","name":"Rust game"},{"id":"2","name":"Rust"}]),
                "Rust"
            )
            .unwrap()["id"],
            "2"
        );
        assert!(select_topic(&json!([]), "missing").is_err());
    }
    #[test]
    fn topic_picker_rejects_nonmatching_suggestions() {
        assert!(select_topic(&json!([{"id":"1","name":"Rust game"}]), "Rust typo").is_err());
    }
    #[test]
    fn images_preserve_source_and_dimensions() {
        let html =
            img_html(&[json!({"src":"https://pic4.zhimg.com/test.png","width":64,"height":64})]);
        assert!(html.starts_with("<img"));
        assert!(html.contains("https://pic4.zhimg.com/test.png"));
        assert!(html.contains("data-rawwidth=\"64\""));
    }
    #[test]
    fn edit_omits_unspecified_fields_and_accepts_explicit_empty() {
        assert_eq!(article_patch(Some("new"), None), json!({"title":"new"}));
        assert_eq!(article_patch(None, Some("")), json!({"content":""}));
        assert_eq!(article_patch(None, None), json!({}));
    }
}

fn article_patch(title: Option<&str>, content: Option<&str>) -> Value {
    let mut patch = json!({});
    if let Some(title) = title {
        patch["title"] = json!(title);
    }
    if let Some(content) = content {
        patch["content"] = json!(content);
    }
    patch
}

pub fn edit_article(
    s: &mut Session,
    id: &str,
    title: Option<&str>,
    content: Option<&str>,
    images: &[String],
    cover: Option<&str>,
    topics: &[String],
    publish: bool,
) -> Result<Value> {
    let mut patch = article_patch(title, content);
    let resolved_topics = resolve_topics(s, topics)?;
    if let Some(path) = cover {
        set_cover(&mut patch, &upload_image(s, path, "article")?)?;
    }
    if !images.is_empty() {
        let mut html = match content {
            Some(value) => value.to_string(),
            None => read_draft(s, id)?["content"]
                .as_str()
                .context("draft has no content; refusing to replace it")?
                .to_string(),
        };
        for path in images {
            html.push_str(&img_html(&[upload_image(s, path, "article")?]));
        }
        patch["content"] = json!(html);
    }
    if patch.as_object().unwrap().is_empty() && topics.is_empty() && !publish {
        bail!("provide --title, --content, --content-file, --image, --cover, --topic or --publish");
    }
    if !patch.as_object().unwrap().is_empty() {
        let response = s.request(
            Method::PATCH,
            &format!("{}/articles/{}/draft", config::ZHUANLAN, id),
            Some(&patch),
        )?;
        if !response.status().is_success() {
            bail!(
                "update article {}: {}",
                response.status(),
                response.text().unwrap_or_default()
            );
        }
    }
    bind_topics(s, id, &resolved_topics)?;
    if publish {
        s.json(
            Method::PUT,
            &format!("{}/articles/{}/publish", config::ZHUANLAN, id),
            Some(&json!({"column":null,"commentPermission":"anyone"})),
        )?;
    }
    let saved =
        read_draft(s, id).with_context(|| format!("article {id} updated; readback failed"))?;
    Ok(json!({
        "id":id,
        "state":saved.get("state").cloned().unwrap_or(json!("updated")),
        "publish_requested":publish,
        "edit_url":format!("https://zhuanlan.zhihu.com/p/{id}/edit"),
        "url":format!("https://zhuanlan.zhihu.com/p/{id}")
    }))
}

fn set_cover(patch: &mut Value, info: &Value) -> Result<()> {
    let src = info["signed_src"]
        .as_str()
        .filter(|src| !src.is_empty())
        .or_else(|| info["src"].as_str())
        .context("cover upload returned no URL")?;
    patch["titleImage"] = json!(src);
    patch["isTitleImageFullScreen"] = json!(false);
    Ok(())
}

pub fn resolve_topics(s: &mut Session, inputs: &[String]) -> Result<Vec<Value>> {
    let mut resolved = Vec::new();
    for input in inputs {
        let topic = if !input.is_empty() && input.bytes().all(|c| c.is_ascii_digit()) {
            json!({"id":input,"name":input})
        } else {
            let response = s.get(&format!(
                "{}/autocomplete/topics?token={}&max_matches=5&use_similar=0&topic_filter=1",
                config::ZHUANLAN,
                urlencoding(input)
            ))?;
            select_topic(&response, input)?
        };
        let id = topic["id"]
            .as_str()
            .map(str::to_string)
            .or_else(|| topic["id"].as_u64().map(|id| id.to_string()))
            .context("topic has no id")?;
        if !resolved.iter().any(|value: &Value| value["id"] == id) {
            resolved.push(json!({"id":id,"name":topic["name"]}));
        }
    }
    Ok(resolved)
}

fn select_topic(response: &Value, input: &str) -> Result<Value> {
    let values = response
        .as_array()
        .or_else(|| response["data"].as_array())
        .context("topic picker returned no list")?;
    let topic = values
        .iter()
        .find(|topic| {
            topic["name"]
                .as_str()
                .is_some_and(|name| name.eq_ignore_ascii_case(input))
        })
        .with_context(|| {
            format!("no exact topic found for {input}; provide its full name or numeric ID")
        })?;
    Ok(topic.clone())
}

fn bind_topics(s: &mut Session, id: &str, topics: &[Value]) -> Result<()> {
    if topics.is_empty() {
        return Ok(());
    }
    let ids = topic_ids(topics)?;
    s.json(
        Method::PUT,
        &format!("{}/articles/{id}/topics", config::ZHUANLAN),
        Some(&json!({"ids":ids})),
    )?;
    Ok(())
}

fn topic_ids(topics: &[Value]) -> Result<Vec<u64>> {
    topics
        .iter()
        .map(|topic| {
            topic["id"]
                .as_u64()
                .or_else(|| topic["id"].as_str().and_then(|id| id.parse().ok()))
                .context("topic id must be numeric")
        })
        .collect()
}

fn metadata_image_key(info: &Value) -> Option<String> {
    info.get("src")
        .and_then(Value::as_str)
        .and_then(|src| {
            src.split('/').find_map(|part| {
                let key = part.split(['~', '?']).next()?;
                key.starts_with("v2-").then(|| key.to_string())
            })
        })
        .or_else(|| {
            info.get("original_hash")
                .and_then(Value::as_str)
                .filter(|hash| !hash.is_empty())
                .map(|hash| {
                    if hash.starts_with("v2-") {
                        hash.to_string()
                    } else {
                        format!("v2-{hash}")
                    }
                })
        })
}
