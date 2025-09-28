# Dist

Single-binary CLI that cross-posts short text updates to Bluesky, Mastodon, and Nostr in parallel.

## Features
- Posts a message to all configured services with one command.
- Reads credentials and server details from environment variables (supports a local `.env`).
- Accepts text via CLI argument or `--stdin`, making it easy to script.
- Optional `--no-*` flags let you skip individual services (handy for testing).
- Bluesky posts automatically annotate URLs, fetch metadata, and upload thumbnails so the first link renders with a rich card preview.

## Requirements
- Rust 1.82+ (the project uses the 2024 edition).
- Accounts and API credentials for each network you plan to post to.

## Setup
1. Install the Rust toolchain if you have not already: <https://rustup.rs>.
2. Clone the repository and change into the project directory.
3. Create a config file at `~/.config/dist/config.env` (or the platform-specific user config dir) with the credentials you want to use. During development (`cargo run`, `cargo test`, etc.) the binary also reads a local `.env` when present for convenience.

### Configuration
The application looks for the following environment variables. Missing values simply cause that service to be skipped at runtime.

| Variable | Required for | Description |
| --- | --- | --- |
| `BSKY_HANDLE` | Bluesky | Your Bluesky handle (e.g. `alice.bsky.social`). |
| `BSKY_PASSWORD` | Bluesky | App password associated with the handle. |
| `BSKY_PDS` | Bluesky (optional) | Bluesky PDS endpoint; defaults to `https://bsky.social`. |
| `MASTODON_BASE_URL` | Mastodon | Base URL of the Mastodon instance (e.g. `https://mastodon.social`). |
| `MASTODON_ACCESS_TOKEN` | Mastodon | Access token with permission to post statuses. |
| `NOSTR_NSEC` | Nostr | Your Nostr private key in `nsec` (or hex) format. |
| `NOSTR_RELAYS` | Nostr (optional) | Comma-separated list of relay URLs; invalid entries are ignored. |

Example snippet (`config.env` or `.env` during development):

```
BSKY_HANDLE=alice.example
BSKY_PASSWORD=xxxx-xxxx-xxxx
MASTODON_BASE_URL=https://mastodon.social
MASTODON_ACCESS_TOKEN=your-token
NOSTR_NSEC=nsec1...
NOSTR_RELAYS=wss://relay.damus.io,wss://nos.lol
```

## Building
Compile the release binary with:

```
cargo build --release
```

The resulting executable is saved at `target/release/dist` by default (rename or re-alias as you like).

## Usage
Print the help message:

```
cargo run -- --help
```

Post a message provided on the command line:

```
cargo run -- "Hello from crosspost-rs!"
```

Read text from standard input (useful for piping or scripting):

```
echo "Automated update" | cargo run -- --stdin
```

Skip individual services when you need to test credentials:

```
cargo run -- "Testing" --no-bsky --no-nostr
```

When a service call succeeds you will see its canonical URL or event ID in the output; errors are logged to stderr without stopping the other posts.

## Development
- The project uses `tokio` for async execution and `reqwest` / `nostr-sdk` for API calls.
- Each service request runs concurrently via `futures::join!`.
- Contributions are welcome; feel free to open issues or pull requests.
- For contributor expectations and workflows, see [AGENTS.md](AGENTS.md).
