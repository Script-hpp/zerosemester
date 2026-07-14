# ZeroSemester

A terminal app that exports Notion pages to PDF. Written in Rust.

## Why

I finished school and wanted my Notion notes out of Notion before starting my studies. Doing it by hand was tedious, and I'd been looking for an excuse to write something real in Rust. So: a small TUI that logs into Notion, pulls pages, and saves them as PDFs. Optionally it should push them into my Paperless-ngx instance, so everything ends up on my own server instead of someone else's cloud.

## Status

Early. Don't expect it to work yet.

- TUI: works
- Notion OAuth login: in progress
- PDF export: in progress
- Paperless-ngx upload: not started

## Stack

Rust, [ratatui](https://ratatui.rs/) + crossterm for the TUI, tokio for async, reqwest/oauth2/axum for the network and login side, serde for the JSON.

## Running it

You need Rust (via rustup) and a Notion integration with a client ID and secret.

```bash
git clone https://github.com/YOUR_USERNAME/zerosemester.git
cd zerosemester
cargo run
```

## Contributing

The code is young and messy. Issues, ideas, and PRs are welcome.