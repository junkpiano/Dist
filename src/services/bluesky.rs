use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use html_escape::decode_html_entities;
use linkify::{LinkFinder, LinkKind};
use reqwest::{
    Url,
    header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE},
};
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};

const PREVIEW_MAX_BYTES: usize = 64 * 1024;
const THUMB_MAX_BYTES: usize = 1_500_000;
const BSKY_EMBED_TEXT_LIMIT: usize = 300;

pub async fn post_bluesky(pds: &str, handle: &str, password: &str, text: &str) -> Result<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .context("bsky: build http client")?;

    let sess_resp = client
        .post(format!(
            "{}/xrpc/com.atproto.server.createSession",
            pds.trim_end_matches('/')
        ))
        .json(&serde_json::json!({ "identifier": handle, "password": password }))
        .send()
        .await
        .context("bsky: createSession request failed")?;

    if !sess_resp.status().is_success() {
        return Err(anyhow!("bsky: createSession status={}", sess_resp.status()));
    }
    let session: BskySession = sess_resp.json().await.context("bsky: parse session")?;

    let links = detect_links(text);
    let preview = match links.first() {
        Some(first) => fetch_link_preview(&client, &first.url).await,
        None => None,
    };
    let thumb = if let (Some(first), Some(preview)) = (links.first(), preview.as_ref()) {
        if let Some(image_url) = preview.image.as_ref() {
            fetch_thumbnail_blob(&client, &first.url, image_url, pds, &session.access_jwt).await
        } else {
            None
        }
    } else {
        None
    };
    let facets = build_bsky_facets(&links);
    let embed_preview = preview.clone();
    let record = BskyPostRecord {
        typ: "app.bsky.feed.post",
        text,
        created_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
        langs: None,
        facets,
        embed: build_bsky_external_embed(links.first(), embed_preview, thumb),
    };
    let payload = BskyCreateRecordReq {
        repo: &session.did,
        collection: "app.bsky.feed.post",
        record,
    };
    let rec_resp = client
        .post(format!(
            "{}/xrpc/com.atproto.repo.createRecord",
            pds.trim_end_matches('/')
        ))
        .header(AUTHORIZATION, format!("Bearer {}", session.access_jwt))
        .header(CONTENT_TYPE, "application/json")
        .json(&payload)
        .send()
        .await
        .context("bsky: createRecord request failed")?;

    if !rec_resp.status().is_success() {
        return Err(anyhow!("bsky: createRecord status={}", rec_resp.status()));
    }
    let out: BskyCreateRecordResp = rec_resp.json().await.context("bsky: parse createRecord")?;
    Ok(out.uri)
}

#[derive(Debug, Clone)]
struct DetectedLink {
    url: String,
    start: usize,
    end: usize,
}

#[derive(Debug, Clone)]
struct LinkPreview {
    title: Option<String>,
    description: Option<String>,
    image: Option<String>,
}

fn detect_links(text: &str) -> Vec<DetectedLink> {
    let mut finder = LinkFinder::new();
    finder.kinds(&[LinkKind::Url]);

    finder
        .links(text)
        .filter_map(|link| {
            let uri = link.as_str();
            if !(uri.starts_with("http://") || uri.starts_with("https://")) {
                return None;
            }

            Some(DetectedLink {
                url: uri.to_string(),
                start: link.start(),
                end: link.end(),
            })
        })
        .collect()
}

fn build_bsky_facets(links: &[DetectedLink]) -> Option<Vec<BskyFacet>> {
    if links.is_empty() {
        return None;
    }

    let facets = links
        .iter()
        .map(|link| BskyFacet {
            index: BskyFacetIndex {
                byte_start: link.start,
                byte_end: link.end,
            },
            features: vec![BskyFacetFeatureLink {
                typ: "app.bsky.richtext.facet#link",
                uri: link.url.clone(),
            }],
        })
        .collect();

    Some(facets)
}

fn build_bsky_external_embed(
    link: Option<&DetectedLink>,
    preview: Option<LinkPreview>,
    thumb: Option<BskyThumb>,
) -> Option<BskyExternalEmbed> {
    let link = link?;

    let mut title = link.url.clone();
    let mut description = link.url.clone();

    if let Some(preview) = preview {
        if let Some(t) = preview.title {
            title = t;
        }
        if let Some(d) = preview.description {
            description = d;
        }
    }

    Some(BskyExternalEmbed {
        typ: "app.bsky.embed.external",
        external: BskyExternal {
            uri: link.url.clone(),
            title: clamp_text(title, BSKY_EMBED_TEXT_LIMIT),
            description: clamp_text(description, BSKY_EMBED_TEXT_LIMIT),
            thumb,
        },
    })
}

async fn fetch_link_preview(client: &reqwest::Client, url: &str) -> Option<LinkPreview> {
    let response = client
        .get(url)
        .header(ACCEPT, "text/html,application/xhtml+xml;q=0.9,*/*;q=0.1")
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .ok()?;

    if !response.status().is_success() {
        return None;
    }

    if let Some(content_type) = response.headers().get(CONTENT_TYPE) {
        if let Ok(ct) = content_type.to_str() {
            if !ct.to_ascii_lowercase().contains("text/html") {
                return None;
            }
        }
    }

    let bytes = response.bytes().await.ok()?;
    let slice = &bytes[..bytes.len().min(PREVIEW_MAX_BYTES)];
    let body = String::from_utf8_lossy(slice);

    let document = Html::parse_document(&body);
    let meta_selector = Selector::parse("meta").ok()?;
    let title_selector = Selector::parse("title").ok()?;

    let mut preview = LinkPreview {
        title: None,
        description: None,
        image: None,
    };

    for meta in document.select(&meta_selector) {
        let value = meta.value();
        let raw = match value.attr("content") {
            Some(c) => c.trim(),
            None => continue,
        };
        if raw.is_empty() {
            continue;
        }

        let decoded = decode_html_entities(raw).to_string();
        let text_value = normalize_text(&decoded);

        if let Some(property) = value.attr("property") {
            match property {
                "og:title" if preview.title.is_none() => preview.title = text_value.clone(),
                "og:description" if preview.description.is_none() => {
                    preview.description = text_value.clone()
                }
                "og:image" | "og:image:url" | "og:image:secure_url" if preview.image.is_none() => {
                    preview.image = Some(decoded.clone())
                }
                _ => {}
            }
        }

        if let Some(name) = value.attr("name") {
            match name {
                "twitter:title" | "title" if preview.title.is_none() => {
                    preview.title = text_value.clone()
                }
                "twitter:description" | "description" if preview.description.is_none() => {
                    preview.description = text_value.clone()
                }
                "twitter:image" | "twitter:image:src" if preview.image.is_none() => {
                    preview.image = Some(decoded.clone())
                }
                _ => {}
            }
        }

        if preview.title.is_some() && preview.description.is_some() && preview.image.is_some() {
            break;
        }
    }

    if preview.title.is_none() {
        if let Some(title_el) = document.select(&title_selector).next() {
            let raw_title = title_el.text().collect::<String>();
            let decoded = decode_html_entities(raw_title.trim()).to_string();
            if let Some(normalized) = normalize_text(&decoded) {
                preview.title = Some(normalized);
            }
        }
    }

    if preview.title.is_none() && preview.description.is_none() && preview.image.is_none() {
        return None;
    }

    Some(preview)
}

async fn fetch_thumbnail_blob(
    client: &reqwest::Client,
    page_url: &str,
    image_url: &str,
    pds: &str,
    access_token: &str,
) -> Option<BskyThumb> {
    let resolved = resolve_url(page_url, image_url)?;
    let response = client
        .get(resolved.clone())
        .header(ACCEPT, "image/*")
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .ok()?;

    if !response.status().is_success() {
        return None;
    }

    let mime_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "application/octet-stream".to_string());

    if !mime_type.to_ascii_lowercase().starts_with("image/") {
        return None;
    }

    let bytes = response.bytes().await.ok()?;
    if bytes.len() > THUMB_MAX_BYTES {
        return None;
    }

    upload_blob(client, pds, access_token, bytes.to_vec(), &mime_type).await
}

async fn upload_blob(
    client: &reqwest::Client,
    pds: &str,
    access_token: &str,
    data: Vec<u8>,
    mime_type: &str,
) -> Option<BskyThumb> {
    let url = format!(
        "{}/xrpc/com.atproto.repo.uploadBlob",
        pds.trim_end_matches('/')
    );

    let response = client
        .post(url)
        .header(AUTHORIZATION, format!("Bearer {}", access_token))
        .header(CONTENT_TYPE, mime_type)
        .body(data)
        .send()
        .await
        .ok()?;

    if !response.status().is_success() {
        return None;
    }

    let payload: UploadBlobResponse = response.json().await.ok()?;

    Some(BskyThumb {
        typ: "blob",
        mime_type: payload.blob.mime_type,
        size: payload.blob.size,
        reference: BskyThumbRef {
            link: payload.blob.reference.link,
        },
    })
}

fn normalize_text(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }

    let collapsed = trimmed.split_whitespace().collect::<Vec<_>>().join(" ");

    if collapsed.is_empty() {
        None
    } else {
        Some(collapsed)
    }
}

fn resolve_url(base: &str, candidate: &str) -> Option<Url> {
    if let Ok(url) = Url::parse(candidate) {
        return Some(url);
    }

    let base = Url::parse(base).ok()?;
    base.join(candidate).ok()
}

fn clamp_text(text: String, limit: usize) -> String {
    if limit == 0 {
        return String::new();
    }

    let char_count = text.chars().count();
    if char_count <= limit {
        return text;
    }

    let take = limit.saturating_sub(1);
    if take == 0 {
        return "…".to_string();
    }

    let mut truncated: String = text.chars().take(take).collect();
    truncated.push('…');
    truncated
}

#[derive(Serialize, Deserialize)]
struct BskySession {
    #[serde(rename = "accessJwt")]
    access_jwt: String,
    did: String,
}

#[derive(Serialize)]
struct BskyPostRecord<'a> {
    #[serde(rename = "$type")]
    typ: &'a str,
    text: &'a str,
    #[serde(rename = "createdAt")]
    created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    langs: Option<Vec<&'a str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    facets: Option<Vec<BskyFacet>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    embed: Option<BskyExternalEmbed>,
}

#[derive(Serialize)]
struct BskyCreateRecordReq<'a> {
    repo: &'a str,
    collection: &'a str,
    record: BskyPostRecord<'a>,
}

#[derive(Serialize)]
struct BskyFacetIndex {
    #[serde(rename = "byteStart")]
    byte_start: usize,
    #[serde(rename = "byteEnd")]
    byte_end: usize,
}

#[derive(Serialize)]
struct BskyFacetFeatureLink {
    #[serde(rename = "$type")]
    typ: &'static str,
    uri: String,
}

#[derive(Serialize)]
struct BskyFacet {
    index: BskyFacetIndex,
    features: Vec<BskyFacetFeatureLink>,
}

#[derive(Serialize)]
struct BskyExternalEmbed {
    #[serde(rename = "$type")]
    typ: &'static str,
    external: BskyExternal,
}

#[derive(Serialize)]
struct BskyExternal {
    uri: String,
    title: String,
    description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    thumb: Option<BskyThumb>,
}

#[derive(Serialize)]
struct BskyThumb {
    #[serde(rename = "$type")]
    typ: &'static str,
    #[serde(rename = "mimeType")]
    mime_type: String,
    size: usize,
    #[serde(rename = "ref")]
    reference: BskyThumbRef,
}

#[derive(Serialize)]
struct BskyThumbRef {
    #[serde(rename = "$link")]
    link: String,
}

#[derive(Deserialize)]
struct BskyCreateRecordResp {
    uri: String,
}

#[derive(Deserialize)]
struct UploadBlobResponse {
    blob: UploadBlobData,
}

#[derive(Deserialize)]
struct UploadBlobData {
    #[serde(rename = "$type")]
    _typ: String,
    #[serde(rename = "mimeType")]
    mime_type: String,
    size: usize,
    #[serde(rename = "ref")]
    reference: UploadBlobRef,
}

#[derive(Deserialize)]
struct UploadBlobRef {
    #[serde(rename = "$link")]
    link: String,
}
