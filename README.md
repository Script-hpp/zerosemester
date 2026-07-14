# ZeroSemester

A high-performance terminal application (TUI) written in Rust that exports Notion pages to local PDF files and seamlessly syncs them to your Paperless-ngx instance with automatic deduplication and tag assignment.

## Why

I finished school and wanted all my Notion notes exported and backed up before starting my university studies. Exporting hundreds of pages manually was tedious and impractical, so I built ZeroSemester to automate the entire pipeline in Rust: a fast terminal user interface that logs into Notion via OAuth, recursively fetches pages and block hierarchies, renders clean PDFs locally, and pushes them directly into a self-hosted Paperless-ngx server.

## Features

- **Interactive Terminal UI (TUI)**: Built with `ratatui` and `crossterm`, featuring intuitive keyboard navigation, live activity logs, and real-time progress bars.
- **Notion OAuth 2.0 & Rate-Limit Protection**: Automatic OAuth token acquisition and persistence (`notion_token.txt`). Includes robust exponential backoff and dynamic `Retry-After` header handling for HTTP 429 rate limits when fetching hundreds of pages.
- **Multithreaded Local PDF Export**: Uses a concurrent `tokio` worker pool (`tokio::sync::Semaphore`) and `spawn_blocking` to render hundreds of pages in parallel without blocking or freezing the UI. Supports both **Flat Directory Mode** (default, optimized for OCR pipelines) and **Categorized Folder Mode** (`Tab` toggle).
- **Paperless-ngx REST API Integration**:
  - **Interactive Configuration**: Set your Paperless server URL, API token, and tag preferences directly inside the TUI or via `paperless_config.json`.
  - **Smart Deduplication**: Automatically queries your Paperless-ngx server for existing documents and flags them (`[ALREADY IN PAPERLESS]`) by matching titles and filenames, preventing duplicate uploads.
  - **Dynamic Tag Assignment & Reuse**: Fetches existing tags from Paperless (`GET /api/tags/`) and reuses matching IDs (case-insensitive). Optionally auto-creates new tags from Notion categories or assigns custom comma-separated base tags (e.g., `Abitur, Notion`) to all exported documents.
  - **Concurrent Upload Workers**: Renders missing PDFs on the fly and uploads document streams directly via `multipart/form-data`.

## Status

Fully functional across the entire export and synchronization workflow:
- TUI & Event Loop: Completed
- Notion OAuth & API Client: Completed
- Multithreaded PDF Export & Rate-Limit Backoff: Completed
- Paperless-ngx Sync, Deduplication & Tagging: Completed

## Technology Stack

- **Core & Async**: Rust, `tokio` (full async runtime, multi-worker channels, semaphores)
- **TUI & Styling**: `ratatui`, `crossterm`
- **Networking & Auth**: `reqwest` (with JSON & Multipart features), `oauth2`, `axum` (for local OAuth callback server)
- **Serialization & Data**: `serde`, `serde_json`, `url`

## Installation & Usage

1. **Requirements**: Ensure you have Rust installed via `rustup`.
2. **Notion Setup**: Create an internal or public Notion integration to obtain your Client ID and Client Secret, or log in via the built-in OAuth flow.
3. **Run the Application**:

```bash
git clone https://github.com/YOUR_USERNAME/zerosemester.git
cd zerosemester
cargo run
```

### Configuration Files

The application automatically manages and stores your configurations locally:
- `notion_token.txt`: Stores the authenticated Notion Bearer token.
- `paperless_config.json`: Stores your Paperless-ngx connection settings and tagging behavior:
  ```json
  {
    "url": "http://localhost:8000",
    "token": "your_paperless_api_token",
    "auto_create_tags": true,
    "add_base_tag": true,
    "base_tag_name": "Abitur"
  }
  ```

## Contributing

Pull requests, feature requests, and bug reports are welcome.