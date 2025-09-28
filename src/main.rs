mod services;

use crate::services::{bluesky, mastodon, nostr};
use anyhow::Result;
use clap::Parser;
use directories::ProjectDirs;
use dotenvy::dotenv;
use futures::join;
use std::collections::HashMap;
use tokio::io::{AsyncReadExt, stdin};

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

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let env = Env::load();

    let text = if args.stdin {
        let mut buf = String::new();
        let mut reader = stdin();
        reader.read_to_string(&mut buf).await?;
        buf.trim().to_string()
    } else {
        args.text
            .clone()
            .unwrap_or_else(|| {
                eprintln!("Usage: dist \"your text\" (or --stdin)");
                std::process::exit(1);
            })
            .trim()
            .to_string()
    };

    if text.is_empty() {
        eprintln!("Text is empty.");
        std::process::exit(1);
    }

    let bsky_fut = async {
        if !args.no_bsky {
            match (env.bsky_handle.as_deref(), env.bsky_password.as_deref()) {
                (Some(handle), Some(password)) => {
                    match bluesky::post_bluesky(&env.bsky_pds, handle, password, &text).await {
                        Ok(uri) => println!("[Bluesky] OK: {uri}"),
                        Err(e) => eprintln!("[Bluesky] ERROR: {e:?}"),
                    }
                }
                _ => println!("[Bluesky] skipped (missing env)"),
            }
        } else {
            println!("[Bluesky] skipped (--no-bsky)");
        }
    };

    let masto_fut = async {
        if !args.no_masto {
            match (env.masto_base.as_deref(), env.masto_token.as_deref()) {
                (Some(base), Some(token)) => {
                    match mastodon::post_mastodon(base, token, &text).await {
                        Ok(url) => println!("[Mastodon] OK: {url}"),
                        Err(e) => eprintln!("[Mastodon] ERROR: {e:?}"),
                    }
                }
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
                    let relays = &env.nostr_relays;
                    match nostr::post_nostr(nsec, relays, &text).await {
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

    join!(bsky_fut, masto_fut, nostr_fut);

    Ok(())
}
