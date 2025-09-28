use anyhow::Result;
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
