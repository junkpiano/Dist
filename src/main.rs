use std::{collections::HashMap, time::Duration};

use anyhow::{Context, Result, anyhow};
use clap::Parser;
use directories::ProjectDirs;
use dotenvy::dotenv;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use tokio::time::sleep;

// Nostr
use nostr_sdk::prelude::*;

// Futures
use futures::join;

/// Simple, single-binary cross-poster for Bluesky, Mastodon, and Nostr.
/// - Credentials are read from environment variables (.env supported).
/// - Text is taken from CLI arg or STDIN when --stdin is set.
#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// The text to post (ignored when --stdin is provided)
    text: Option<String>,
    /// Read text from STDIN
    #[arg(long)]
    stdin: bool,

    /// Skip a service (useful for testing)
    #[arg(long)]
    no_bsky: bool,
    #[arg(long)]
    no_masto: bool,
    #[arg(long)]
    no_nostr: bool,
}

#[derive(Debug)]
struct Env {
    // Bluesky
    bsky_handle: Option<String>,
    bsky_password: Option<String>,
    bsky_pds: String,

    // Mastodon
    masto_base: Option<String>,
    masto_token: Option<String>,

    // Nostr
    nostr_nsec: Option<String>,
    nostr_relays: Vec<String>,
}

impl Env {
    fn load() -> Self {
        let config = load_config_defaults();

        #[cfg(debug_assertions)]
        {
            let _ = dotenv();
        }

        let bsky_pds =
            lookup_env("BSKY_PDS", &config).unwrap_or_else(|| "https://bsky.social".to_string());

        let nostr_relays = lookup_env("NOSTR_RELAYS", &config)
            .map(|s| {
                s.split(',')
                    .map(|x| x.trim().to_string())
                    .filter(|x| !x.is_empty())
                    .collect()
            })
            .unwrap_or_else(|| Vec::<String>::new());

        Self {
            bsky_handle: lookup_env("BSKY_HANDLE", &config),
            bsky_password: lookup_env("BSKY_PASSWORD", &config),
            bsky_pds,
            masto_base: lookup_env("MASTODON_BASE_URL", &config),
            masto_token: lookup_env("MASTODON_ACCESS_TOKEN", &config),
            nostr_nsec: lookup_env("NOSTR_NSEC", &config),
            nostr_relays,
        }
    }
}

fn load_config_defaults() -> HashMap<String, String> {
    let mut values = HashMap::new();

    if let Some(dirs) = ProjectDirs::from("", "", "dist") {
        let config_path = dirs.config_dir().join("config.env");
        if let Ok(iter) = dotenvy::from_path_iter(&config_path) {
            for item in iter.flatten() {
                values.insert(item.0, item.1);
            }
        }
    }

    values
}

fn lookup_env(key: &str, config: &HashMap<String, String>) -> Option<String> {
    std::env::var(key).ok().or_else(|| config.get(key).cloned())
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
}

#[derive(Serialize)]
struct BskyCreateRecordReq<'a> {
    repo: &'a str,
    collection: &'a str,
    record: BskyPostRecord<'a>,
}

#[derive(Deserialize)]
struct BskyCreateRecordResp {
    uri: String,
}

async fn post_bluesky(pds: &str, handle: &str, password: &str, text: &str) -> Result<String> {
    // 1) createSession
    let client = reqwest::Client::new();
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

    // 2) createRecord
    let record = BskyPostRecord {
        typ: "app.bsky.feed.post",
        text,
        created_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
        langs: None, // e.g. Some(vec!["ja"])
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

#[derive(Deserialize)]
struct MastoResp {
    url: Option<String>,
    uri: Option<String>,
}

async fn post_mastodon(base: &str, token: &str, text: &str) -> Result<String> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/v1/statuses", base.trim_end_matches('/')))
        .header(AUTHORIZATION, format!("Bearer {}", token))
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

async fn post_nostr(nsec_or_hex: &str, relays: &[String], text: &str) -> Result<String> {
    let keys = Keys::parse(nsec_or_hex)?;
    let client = Client::new(keys);

    // Add relays (ignore invalid ones)
    for r in relays {
        let url = match RelayUrl::parse(r) {
            Ok(u) => u,
            Err(_) => continue,
        };
        // Ignore errors per-relay; we only need some to succeed
        let _ = client.add_relay(url).await;
    }

    client.connect().await;

    // Build and send in one shot (client holds the signer=keys)
    let builder = EventBuilder::text_note(text);
    let output = client.send_event_builder(builder).await?;

    // Give relays a brief grace time to ack
    sleep(Duration::from_millis(300)).await;

    client.disconnect().await;

    Ok(output.id().to_bech32()?)
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let env = Env::load();

    // Resolve message text (from arg or STDIN)
    let text = if args.stdin {
        use tokio::io::{AsyncReadExt, stdin};
        let mut buf = String::new();
        let mut reader = stdin();
        reader.read_to_string(&mut buf).await?;
        buf.trim().to_string()
    } else {
        args.text
            .clone()
            .unwrap_or_else(|| {
                eprintln!("Usage: crosspost-rs \"your text\"  (or --stdin)");
                std::process::exit(1);
            })
            .trim()
            .to_string()
    };

    if text.is_empty() {
        eprintln!("Text is empty.");
        std::process::exit(1);
    }

    // Build three futures that borrow from local variables (no 'static required)
    let bsky_fut = async {
        if !args.no_bsky {
            match (env.bsky_handle.as_deref(), env.bsky_password.as_deref()) {
                (Some(h), Some(pw)) => match post_bluesky(&env.bsky_pds, h, pw, &text).await {
                    Ok(uri) => println!("[Bluesky] OK: {uri}"),
                    Err(e) => eprintln!("[Bluesky] ERROR: {e:?}"),
                },
                _ => println!("[Bluesky] skipped (missing env)"),
            }
        } else {
            println!("[Bluesky] skipped (--no-bsky)");
        }
    };

    let masto_fut = async {
        if !args.no_masto {
            match (env.masto_base.as_deref(), env.masto_token.as_deref()) {
                (Some(base), Some(token)) => match post_mastodon(base, token, &text).await {
                    Ok(url) => println!("[Mastodon] OK: {url}"),
                    Err(e) => eprintln!("[Mastodon] ERROR: {e:?}"),
                },
                _ => println!("[Mastodon] skipped (missing env)"),
            }
        } else {
            println!("[Mastodon] skipped (--no-masto)");
        }
    };

    let nostr_fut = async {
        if !args.no_nostr {
            match env.nostr_nsec.as_deref() {
                Some(nsec) => {
                    let relays: &[String] = &env.nostr_relays; // borrow the Vec as a slice
                    match post_nostr(nsec, relays, &text).await {
                        Ok(id) => println!("[Nostr] OK: {id}"),
                        Err(e) => eprintln!("[Nostr] ERROR: {e:?}"),
                    }
                }
                None => println!("[Nostr] skipped (missing env)"),
            }
        } else {
            println!("[Nostr] skipped (--no-nostr)");
        }
    };

    // Run all in parallel and wait here
    join!(bsky_fut, masto_fut, nostr_fut);

    Ok(())
}
