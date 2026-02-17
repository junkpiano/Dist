use anyhow::{Context, Result};
use nostr_sdk::prelude::*;
use std::time::Duration;
use tokio::time::sleep;

pub async fn post_nostr(nsec_or_hex: &str, relays: &[String], text: &str) -> Result<String> {
    let keys = Keys::parse(nsec_or_hex)?;
    let client = Client::new(keys);

    for r in relays {
        let url = match RelayUrl::parse(r) {
            Ok(u) => u,
            Err(_) => continue,
        };
        let _ = client.add_relay(url).await;
    }

    client.connect().await;

    let builder = EventBuilder::text_note(text);
    let output = client.send_event_builder(builder).await?;

    sleep(Duration::from_millis(300)).await;
    client.disconnect().await;

    Ok(output.id().to_bech32()?)
}

pub async fn update_nostr_profile_image(
    nsec_or_hex: &str,
    relays: &[String],
    image_url: &str,
) -> Result<String> {
    let keys = Keys::parse(nsec_or_hex)?;
    let public_key = keys.public_key();
    let client = Client::new(keys);

    for r in relays {
        let url = match RelayUrl::parse(r) {
            Ok(u) => u,
            Err(_) => continue,
        };
        let _ = client.add_relay(url).await;
    }

    client.connect().await;

    let metadata = fetch_latest_metadata(&client, public_key).await;
    let picture = Url::parse(image_url).context("nostr: invalid image url")?;
    let updated = metadata.picture(picture);

    let output = client.set_metadata(&updated).await?;

    sleep(Duration::from_millis(300)).await;
    client.disconnect().await;

    Ok(output.id().to_bech32()?)
}

async fn fetch_latest_metadata(client: &Client, public_key: PublicKey) -> Metadata {
    let filter = Filter::new()
        .author(public_key)
        .kind(Kind::Metadata)
        .limit(1);

    let events = match client.fetch_events(filter, Duration::from_secs(5)).await {
        Ok(events) => events,
        Err(_) => return Metadata::new(),
    };

    let Some(event) = events.first() else {
        return Metadata::new();
    };

    Metadata::from_json(&event.content).unwrap_or_else(|_| Metadata::new())
}
