use anyhow::{Context, Result, anyhow};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::Deserialize;

#[derive(Deserialize)]
struct MastoResp {
    url: Option<String>,
    uri: Option<String>,
}

pub async fn post_mastodon(base: &str, token: &str, text: &str) -> Result<String> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/v1/statuses", base.trim_end_matches('/')))
        .header(AUTHORIZATION, format!("Bearer {}", token))
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .form(&[("status", text), ("visibility", "public")])
        .send()
        .await
        .context("mastodon: request failed")?;

    if !resp.status().is_success() {
        return Err(anyhow!("mastodon: status={}", resp.status()));
    }
    let out: MastoResp = resp.json().await.context("mastodon: parse")?;
    Ok(out.url.or(out.uri).unwrap_or_default())
}
