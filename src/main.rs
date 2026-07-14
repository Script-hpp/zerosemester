mod auth;
mod notion_api;
mod paperless_api;

use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, List, ListItem, ListState, Paragraph, Wrap},
    Terminal,
};
use std::{io, path::PathBuf, time::Duration};

#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum ExportTarget {
    LocalFolder,
    PaperlessNgx,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AppState {
    Menu,
    FetchingPages,
    PageSelection,
    FolderSelection,
    Exporting,
    Summary,
    PaperlessConfig,
    FetchingPaperless,
    PaperlessPageSelection,
    UploadingPaperless,
    PaperlessSummary,
}

struct SelectablePage {
    page: notion_api::NotionPage,
    selected: bool,
}

struct SelectablePaperlessPage {
    page: notion_api::NotionPage,
    selected: bool,
    already_in_paperless: bool,
}

enum ExportUpdate {
    Log(String),
    Progress(usize),
    Finished(usize, usize),
}

struct App {
    state: AppState,
    menu_index: usize,
    pages: Vec<SelectablePage>,
    page_list_state: ListState,
    export_dir: String,
    export_progress: usize,
    export_total: usize,
    export_logs: Vec<String>,
    status_message: String,
    spinner_frame: usize,
    rx_fetch: Option<tokio::sync::mpsc::Receiver<Result<Vec<notion_api::NotionPage>, String>>>,
    rx_export: Option<tokio::sync::mpsc::Receiver<ExportUpdate>>,
    summary_success: usize,
    summary_failed: usize,
    use_category_folders: bool,
    paperless_config: paperless_api::PaperlessConfig,
    paperless_field_index: usize,
    paperless_pages: Vec<SelectablePaperlessPage>,
    paperless_list_state: ListState,
    paperless_tags: Vec<paperless_api::PaperlessTag>,
    paperless_documents: Vec<paperless_api::PaperlessDocument>,
    rx_paperless_fetch: Option<
        tokio::sync::mpsc::Receiver<
            Result<(Vec<notion_api::NotionPage>, paperless_api::PaperlessMetadata), String>,
        >,
    >,
    paperless_progress: usize,
    paperless_total: usize,
    paperless_logs: Vec<String>,
}

impl App {
    fn new() -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        let mut paperless_state = ListState::default();
        paperless_state.select(Some(0));
        Self {
            state: AppState::Menu,
            menu_index: 0,
            pages: Vec::new(),
            page_list_state: list_state,
            export_dir: "./exports".to_string(),
            export_progress: 0,
            export_total: 0,
            export_logs: Vec::new(),
            status_message: "Ready. Select an option to start.".to_string(),
            spinner_frame: 0,
            rx_fetch: None,
            rx_export: None,
            summary_success: 0,
            summary_failed: 0,
            use_category_folders: false,
            paperless_config: paperless_api::PaperlessConfig::load(),
            paperless_field_index: 0,
            paperless_pages: Vec::new(),
            paperless_list_state: paperless_state,
            paperless_tags: Vec::new(),
            paperless_documents: Vec::new(),
            rx_paperless_fetch: None,
            paperless_progress: 0,
            paperless_total: 0,
            paperless_logs: Vec::new(),
        }
    }
}

#[tokio::test]
async fn test_notion_api_connection() {
    println!("Testing API...");
    match notion_api::fetch_pages().await {
        Ok(pages) => {
            println!("Successfully fetched {} pages:", pages.len());
            for page in pages {
                println!(" - {}", page);
            }
        }
        Err(e) => eprintln!("API Error: {}", e),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();
    let spinners = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

    loop {
        // Handle background updates non-blockingly
        if app.state == AppState::FetchingPages {
            app.spinner_frame = (app.spinner_frame + 1) % spinners.len();
            if let Some(rx) = &mut app.rx_fetch {
                if let Ok(res) = rx.try_recv() {
                    match res {
                        Ok(pages) => {
                            app.pages = pages
                                .into_iter()
                                .map(|p| SelectablePage {
                                    page: p,
                                    selected: true,
                                })
                                .collect();
                            app.page_list_state.select(Some(0));
                            app.status_message = format!("Fetched {} pages from Notion workspace.", app.pages.len());
                            app.state = AppState::PageSelection;
                        }
                        Err(e) => {
                            app.status_message = format!("Error: {}", e);
                            app.state = AppState::Menu;
                        }
                    }
                    app.rx_fetch = None;
                }
            }
        } else if app.state == AppState::Exporting {
            app.spinner_frame = (app.spinner_frame + 1) % spinners.len();
            if let Some(rx) = &mut app.rx_export {
                while let Ok(update) = rx.try_recv() {
                    match update {
                        ExportUpdate::Log(msg) => {
                            app.export_logs.push(msg);
                            if app.export_logs.len() > 100 {
                                app.export_logs.remove(0);
                            }
                        }
                        ExportUpdate::Progress(p) => {
                            app.export_progress = p;
                        }
                        ExportUpdate::Finished(s, f) => {
                            app.summary_success = s;
                            app.summary_failed = f;
                            app.status_message = format!("Export completed! Success: {}, Failed: {}", s, f);
                            app.state = AppState::Summary;
                            app.rx_export = None;
                            break;
                        }
                    }
                }
            }
        } else if app.state == AppState::FetchingPaperless {
            app.spinner_frame = (app.spinner_frame + 1) % spinners.len();
            if let Some(rx) = &mut app.rx_paperless_fetch {
                if let Ok(res) = rx.try_recv() {
                    match res {
                        Ok((pages, metadata)) => {
                            app.paperless_tags = metadata.tags;
                            app.paperless_documents = metadata.documents;
                            let mut selectable = Vec::new();
                            for p in pages {
                                let already = app.paperless_documents.iter().any(|doc| {
                                    doc.title.trim().eq_ignore_ascii_case(p.title.trim())
                                        || doc.original_file_name.as_ref().map_or(false, |fn_str| {
                                            let expected = format!("{}.pdf", notion_api::sanitize_filename(&p.title));
                                            fn_str.eq_ignore_ascii_case(&expected)
                                        })
                                });
                                selectable.push(SelectablePaperlessPage {
                                    page: p,
                                    selected: !already,
                                    already_in_paperless: already,
                                });
                            }
                            app.paperless_pages = selectable;
                            app.paperless_list_state.select(Some(0));
                            app.status_message = format!(
                                "Found {} Paperless tags & {} documents.",
                                app.paperless_tags.len(),
                                app.paperless_documents.len()
                            );
                            app.state = AppState::PaperlessPageSelection;
                        }
                        Err(e) => {
                            app.status_message = format!("Connection Error: {}", e);
                            app.state = AppState::PaperlessConfig;
                        }
                    }
                    app.rx_paperless_fetch = None;
                }
            }
        } else if app.state == AppState::UploadingPaperless {
            app.spinner_frame = (app.spinner_frame + 1) % spinners.len();
            if let Some(rx) = &mut app.rx_export {
                while let Ok(update) = rx.try_recv() {
                    match update {
                        ExportUpdate::Log(msg) => {
                            app.paperless_logs.push(msg);
                            if app.paperless_logs.len() > 100 {
                                app.paperless_logs.remove(0);
                            }
                        }
                        ExportUpdate::Progress(p) => {
                            app.paperless_progress = p;
                        }
                        ExportUpdate::Finished(s, f) => {
                            app.summary_success = s;
                            app.summary_failed = f;
                            app.status_message = format!("Paperless upload finished! Success: {}, Failed: {}", s, f);
                            app.state = AppState::PaperlessSummary;
                            app.rx_export = None;
                            break;
                        }
                    }
                }
            }
        }

        terminal.draw(|f| {
            let size = f.area();

            match app.state {
                AppState::Menu => draw_menu(f, &app, size),
                AppState::FetchingPages => draw_fetching(f, &app, size, spinners[app.spinner_frame]),
                AppState::PageSelection => draw_page_selection(f, &mut app, size),
                AppState::FolderSelection => draw_folder_selection(f, &app, size),
                AppState::Exporting => draw_exporting(f, &app, size, spinners[app.spinner_frame]),
                AppState::Summary => draw_summary(f, &app, size),
                AppState::PaperlessConfig => draw_paperless_config(f, &app, size),
                AppState::FetchingPaperless => draw_fetching_paperless(f, &app, size, spinners[app.spinner_frame]),
                AppState::PaperlessPageSelection => draw_paperless_page_selection(f, &mut app, size),
                AppState::UploadingPaperless => draw_uploading_paperless(f, &app, size, spinners[app.spinner_frame]),
                AppState::PaperlessSummary => draw_paperless_summary(f, &app, size),
            }
        })?;

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                match app.state {
                    AppState::Menu => match key.code {
                        KeyCode::Char('q') => break,
                        KeyCode::Up => {
                            if app.menu_index > 0 {
                                app.menu_index -= 1;
                            }
                        }
                        KeyCode::Down => {
                            if app.menu_index < 1 {
                                app.menu_index += 1;
                            }
                        }
                        KeyCode::Enter => {
                            if app.menu_index == 0 {
                                if !std::path::Path::new("notion_token.txt").exists() {
                                    // Suspend raw mode temporarily if opening browser/terminal login prompt
                                    disable_raw_mode().ok();
                                    execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();
                                    println!("Connecting to Notion for OAuth login...");
                                    auth::authenticate_with_notion().await;
                                    enable_raw_mode().ok();
                                    execute!(terminal.backend_mut(), EnterAlternateScreen).ok();
                                    terminal.clear().ok();
                                }

                                app.state = AppState::FetchingPages;
                                app.status_message = "Querying Notion API for pages and categories...".to_string();

                                let (tx, rx) = tokio::sync::mpsc::channel(1);
                                app.rx_fetch = Some(rx);
                                tokio::spawn(async move {
                                    match notion_api::fetch_pages().await {
                                        Ok(p) => { let _ = tx.send(Ok(p)).await; },
                                        Err(e) => { let _ = tx.send(Err(e.to_string())).await; },
                                    }
                                });
                            } else {
                                app.state = AppState::PaperlessConfig;
                                app.status_message = "Configure Paperless-ngx Connection & Tag settings.".to_string();
                            }
                        }
                        _ => {}
                    },
                    AppState::FetchingPages => {
                        if let KeyCode::Esc = key.code {
                            app.state = AppState::Menu;
                            app.status_message = "Cancelled fetching.".to_string();
                            app.rx_fetch = None;
                        }
                    }
                    AppState::PageSelection => match key.code {
                        KeyCode::Esc => {
                            app.state = AppState::Menu;
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            if let Some(selected) = app.page_list_state.selected() {
                                if selected > 0 {
                                    app.page_list_state.select(Some(selected - 1));
                                }
                            }
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            if let Some(selected) = app.page_list_state.selected() {
                                if selected + 1 < app.pages.len() {
                                    app.page_list_state.select(Some(selected + 1));
                                }
                            }
                        }
                        KeyCode::Char(' ') => {
                            if let Some(selected) = app.page_list_state.selected() {
                                if let Some(item) = app.pages.get_mut(selected) {
                                    item.selected = !item.selected;
                                }
                            }
                        }
                        KeyCode::Tab | KeyCode::Char('c') => {
                            app.use_category_folders = !app.use_category_folders;
                        }
                        KeyCode::Char('a') => {
                            for p in &mut app.pages {
                                p.selected = true;
                            }
                        }
                        KeyCode::Char('n') => {
                            for p in &mut app.pages {
                                p.selected = false;
                            }
                        }
                        KeyCode::Enter => {
                            let count = app.pages.iter().filter(|p| p.selected).count();
                            if count > 0 {
                                app.state = AppState::FolderSelection;
                            } else {
                                app.status_message = "Please select at least one page to export!".to_string();
                            }
                        }
                        _ => {}
                    },
                    AppState::FolderSelection => match key.code {
                        KeyCode::Esc => {
                            app.state = AppState::PageSelection;
                        }
                        KeyCode::Tab | KeyCode::Char('c') => {
                            app.use_category_folders = !app.use_category_folders;
                        }
                        KeyCode::Backspace => {
                            app.export_dir.pop();
                        }
                        KeyCode::Char(c) => {
                            app.export_dir.push(c);
                        }
                        KeyCode::Enter => {
                            if app.export_dir.trim().is_empty() {
                                app.export_dir = "./exports".to_string();
                            }
                            let pages_to_export: Vec<_> = app
                                .pages
                                .iter()
                                .filter(|p| p.selected)
                                .map(|p| p.page.clone())
                                .collect();

                            app.export_total = pages_to_export.len();
                            app.export_progress = 0;
                            app.export_logs.clear();
                            app.state = AppState::Exporting;

                            let (tx, rx) = tokio::sync::mpsc::channel(64);
                            app.rx_export = Some(rx);
                            let export_path = PathBuf::from(&app.export_dir);
                            let use_cat = app.use_category_folders;

                            tokio::spawn(async move {
                                let token = match std::fs::read_to_string("notion_token.txt") {
                                    Ok(t) => t.trim().to_string(),
                                    Err(_) => {
                                        let _ = tx.send(ExportUpdate::Log("✖ Failed to read notion_token.txt".to_string())).await;
                                        let _ = tx.send(ExportUpdate::Finished(0, pages_to_export.len())).await;
                                        return;
                                    }
                                };

                                let mut headers = reqwest::header::HeaderMap::new();
                                if let Ok(val) = reqwest::header::HeaderValue::from_str(&format!("Bearer {}", token)) {
                                    headers.insert(reqwest::header::AUTHORIZATION, val);
                                }
                                headers.insert("Notion-Version", reqwest::header::HeaderValue::from_static("2022-06-28"));
                                headers.insert(reqwest::header::CONTENT_TYPE, reqwest::header::HeaderValue::from_static("application/json"));

                                let client = reqwest::Client::new();
                                let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(8));
                                let progress_counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
                                let success_counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
                                let failed_counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

                                let mut tasks = Vec::new();

                                for page in pages_to_export {
                                    let permit = match semaphore.clone().acquire_owned().await {
                                        Ok(p) => p,
                                        Err(_) => break,
                                    };
                                    let tx_clone = tx.clone();
                                    let path_clone = export_path.clone();
                                    let progress_clone = progress_counter.clone();
                                    let success_clone = success_counter.clone();
                                    let failed_clone = failed_counter.clone();
                                    let client_clone = client.clone();
                                    let headers_clone = headers.clone();

                                    let task = tokio::spawn(async move {
                                        let log_prefix = if use_cat {
                                            format!("⏳ Fetching & Exporting [{}] {}...", page.category, page.title)
                                        } else {
                                            format!("⏳ Fetching & Exporting {}...", page.title)
                                        };
                                        let _ = tx_clone.send(ExportUpdate::Log(log_prefix)).await;

                                        match notion_api::export_page_to_pdf(&client_clone, &headers_clone, &page, &path_clone, use_cat).await {
                                            Ok(saved_path) => {
                                                success_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                                let _ = tx_clone
                                                    .send(ExportUpdate::Log(format!("✔ Saved: {}", saved_path.display())))
                                                    .await;
                                            }
                                            Err(e) => {
                                                failed_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                                let _ = tx_clone
                                                    .send(ExportUpdate::Log(format!("✖ Failed [{}]: {}", page.title, e)))
                                                    .await;
                                            }
                                        }

                                        let current = progress_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                                        let _ = tx_clone.send(ExportUpdate::Progress(current)).await;
                                        drop(permit);
                                    });
                                    tasks.push(task);
                                }

                                for t in tasks {
                                    let _ = t.await;
                                }

                                let s = success_counter.load(std::sync::atomic::Ordering::Relaxed);
                                let f = failed_counter.load(std::sync::atomic::Ordering::Relaxed);
                                let _ = tx.send(ExportUpdate::Finished(s, f)).await;
                            });
                        }
                        _ => {}
                    },
                    AppState::Exporting => {
                        // While exporting, disable keyboard except q or Esc to abort if needed
                    }
                    AppState::Summary => match key.code {
                        KeyCode::Enter | KeyCode::Esc => {
                            app.state = AppState::Menu;
                            app.status_message = "Ready. Select an option to start.".to_string();
                        }
                        KeyCode::Char('q') => break,
                        _ => {}
                    },
                    AppState::PaperlessConfig => match key.code {
                        KeyCode::Esc => {
                            app.state = AppState::Menu;
                        }
                        KeyCode::Up => {
                            if app.paperless_field_index > 0 {
                                app.paperless_field_index -= 1;
                            }
                        }
                        KeyCode::Down | KeyCode::Tab => {
                            if app.paperless_field_index < 4 {
                                app.paperless_field_index += 1;
                            } else {
                                app.paperless_field_index = 0;
                            }
                        }
                        KeyCode::Char(' ') if app.paperless_field_index == 2 || app.paperless_field_index == 3 => {
                            if app.paperless_field_index == 2 {
                                app.paperless_config.auto_create_tags = !app.paperless_config.auto_create_tags;
                            } else {
                                app.paperless_config.add_base_tag = !app.paperless_config.add_base_tag;
                            }
                        }
                        KeyCode::Backspace => {
                            if app.paperless_field_index == 0 {
                                app.paperless_config.url.pop();
                            } else if app.paperless_field_index == 1 {
                                app.paperless_config.token.pop();
                            } else if app.paperless_field_index == 4 {
                                app.paperless_config.base_tag_name.pop();
                            }
                        }
                        KeyCode::Char(c) if app.paperless_field_index != 2 && app.paperless_field_index != 3 => {
                            if app.paperless_field_index == 0 {
                                app.paperless_config.url.push(c);
                            } else if app.paperless_field_index == 1 {
                                app.paperless_config.token.push(c);
                            } else if app.paperless_field_index == 4 {
                                app.paperless_config.base_tag_name.push(c);
                            }
                        }
                        KeyCode::Enter => {
                            let _ = app.paperless_config.save();
                            if app.paperless_config.token.trim().is_empty() {
                                app.status_message = "Please enter your Paperless-ngx API Token!".to_string();
                            } else {
                                if !std::path::Path::new("notion_token.txt").exists() {
                                    disable_raw_mode().ok();
                                    execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();
                                    println!("Connecting to Notion for OAuth login...");
                                    auth::authenticate_with_notion().await;
                                    enable_raw_mode().ok();
                                    execute!(terminal.backend_mut(), EnterAlternateScreen).ok();
                                    terminal.clear().ok();
                                }
                                app.state = AppState::FetchingPaperless;
                                app.status_message = "Connecting to Paperless & fetching Notion pages...".to_string();
                                let (tx, rx) = tokio::sync::mpsc::channel(1);
                                app.rx_paperless_fetch = Some(rx);
                                let config = app.paperless_config.clone();
                                tokio::spawn(async move {
                                    let client = reqwest::Client::builder()
                                        .timeout(std::time::Duration::from_secs(30))
                                        .build()
                                        .unwrap_or_else(|_| reqwest::Client::new());

                                    let notion_res = notion_api::fetch_pages().await;
                                    let paperless_res = paperless_api::fetch_metadata(&client, &config).await;

                                    match (notion_res, paperless_res) {
                                        (Ok(pages), Ok(meta)) => {
                                            let _ = tx.send(Ok((pages, meta))).await;
                                        }
                                        (Err(e), _) => {
                                            let _ = tx.send(Err(format!("Notion error: {}", e))).await;
                                        }
                                        (_, Err(e)) => {
                                            let _ = tx.send(Err(format!("Paperless error: {}", e))).await;
                                        }
                                    }
                                });
                            }
                        }
                        _ => {}
                    },
                    AppState::FetchingPaperless => {
                        if let KeyCode::Esc = key.code {
                            app.state = AppState::PaperlessConfig;
                            app.status_message = "Cancelled Paperless check.".to_string();
                            app.rx_paperless_fetch = None;
                        }
                    }
                    AppState::PaperlessPageSelection => match key.code {
                        KeyCode::Esc => {
                            app.state = AppState::PaperlessConfig;
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            if let Some(selected) = app.paperless_list_state.selected() {
                                if selected > 0 {
                                    app.paperless_list_state.select(Some(selected - 1));
                                }
                            }
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            if let Some(selected) = app.paperless_list_state.selected() {
                                if selected + 1 < app.paperless_pages.len() {
                                    app.paperless_list_state.select(Some(selected + 1));
                                }
                            }
                        }
                        KeyCode::Char(' ') => {
                            if let Some(selected) = app.paperless_list_state.selected() {
                                if let Some(item) = app.paperless_pages.get_mut(selected) {
                                    item.selected = !item.selected;
                                }
                            }
                        }
                        KeyCode::Char('a') => {
                            for p in &mut app.paperless_pages {
                                if !p.already_in_paperless {
                                    p.selected = true;
                                }
                            }
                        }
                        KeyCode::Char('n') => {
                            for p in &mut app.paperless_pages {
                                p.selected = false;
                            }
                        }
                        KeyCode::Enter => {
                            let pages_to_upload: Vec<notion_api::NotionPage> = app
                                .paperless_pages
                                .iter()
                                .filter(|p| p.selected)
                                .map(|p| p.page.clone())
                                .collect();

                            if pages_to_upload.is_empty() {
                                app.status_message = "Please select at least one page to upload!".to_string();
                            } else {
                                app.paperless_total = pages_to_upload.len();
                                app.paperless_progress = 0;
                                app.paperless_logs.clear();
                                app.state = AppState::UploadingPaperless;

                                let (tx, rx) = tokio::sync::mpsc::channel(64);
                                app.rx_export = Some(rx);
                                let config = app.paperless_config.clone();
                                let tags_cache = app.paperless_tags.clone();
                                let export_dir = PathBuf::from(&app.export_dir);
                                let use_cat = app.use_category_folders;

                                tokio::spawn(async move {
                                    let token = match std::fs::read_to_string("notion_token.txt") {
                                        Ok(t) => t.trim().to_string(),
                                        Err(_) => {
                                            let _ = tx.send(ExportUpdate::Log("✖ Failed to read notion_token.txt".to_string())).await;
                                            let _ = tx.send(ExportUpdate::Finished(0, pages_to_upload.len())).await;
                                            return;
                                        }
                                    };

                                    let mut headers = reqwest::header::HeaderMap::new();
                                    if let Ok(val) = reqwest::header::HeaderValue::from_str(&format!("Bearer {}", token)) {
                                        headers.insert(reqwest::header::AUTHORIZATION, val);
                                    }
                                    headers.insert("Notion-Version", reqwest::header::HeaderValue::from_static("2022-06-28"));
                                    headers.insert(reqwest::header::CONTENT_TYPE, reqwest::header::HeaderValue::from_static("application/json"));

                                    let client = reqwest::Client::builder()
                                        .timeout(std::time::Duration::from_secs(30))
                                        .build()
                                        .unwrap_or_else(|_| reqwest::Client::new());

                                    let paperless_headers = match paperless_api::build_headers(&config.token) {
                                        Ok(h) => h,
                                        Err(e) => {
                                            let _ = tx.send(ExportUpdate::Log(format!("✖ Paperless auth error: {}", e))).await;
                                            let _ = tx.send(ExportUpdate::Finished(0, pages_to_upload.len())).await;
                                            return;
                                        }
                                    };

                                    let tags_cache_arc = std::sync::Arc::new(tokio::sync::Mutex::new(tags_cache));
                                    
                                    // Pre-resolve base tags if enabled (supports comma-separated list like "Abitur, Notion")
                                    let mut base_tag_ids = Vec::new();
                                    if config.add_base_tag && !config.base_tag_name.trim().is_empty() {
                                        let mut guard = tags_cache_arc.lock().await;
                                        for tag_part in config.base_tag_name.split(',') {
                                            let clean_t = tag_part.trim();
                                            if !clean_t.is_empty() {
                                                let _ = tx.send(ExportUpdate::Log(format!("🏷️ Resolving base tag '{}'...", clean_t))).await;
                                                match paperless_api::ensure_tag(&client, &config, &paperless_headers, clean_t, &mut guard).await {
                                                    Ok(id) => {
                                                        if !base_tag_ids.contains(&id) {
                                                            base_tag_ids.push(id);
                                                        }
                                                    }
                                                    Err(e) => {
                                                        let _ = tx.send(ExportUpdate::Log(format!("⚠️ Base tag warning [{}]: {}", clean_t, e))).await;
                                                    }
                                                }
                                            }
                                        }
                                    }

                                    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(6));
                                    let progress_counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
                                    let success_counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
                                    let failed_counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

                                    let mut tasks = Vec::new();

                                    for page in pages_to_upload {
                                        let permit = match semaphore.clone().acquire_owned().await {
                                            Ok(p) => p,
                                            Err(_) => break,
                                        };
                                        let tx_clone = tx.clone();
                                        let client_clone = client.clone();
                                        let headers_clone = headers.clone();
                                        let p_headers_clone = paperless_headers.clone();
                                        let config_clone = config.clone();
                                        let path_clone = export_dir.clone();
                                        let progress_clone = progress_counter.clone();
                                        let success_clone = success_counter.clone();
                                        let failed_clone = failed_counter.clone();
                                        let tags_arc = tags_cache_arc.clone();
                                        let base_ids_clone = base_tag_ids.clone();

                                        let task = tokio::spawn(async move {
                                            let _ = tx_clone.send(ExportUpdate::Log(format!("⏳ Preparing '{}'...", page.title))).await;

                                            let target_dir = if use_cat {
                                                path_clone.join(notion_api::sanitize_filename(&page.category))
                                            } else {
                                                path_clone.clone()
                                            };
                                            let pdf_path = target_dir.join(format!("{}.pdf", notion_api::sanitize_filename(&page.title)));

                                            let pdf_ready = if pdf_path.exists() {
                                                true
                                            } else {
                                                let _ = tx_clone.send(ExportUpdate::Log(format!("🛠️ Rendering PDF for '{}'...", page.title))).await;
                                                match notion_api::export_page_to_pdf(&client_clone, &headers_clone, &page, &path_clone, use_cat).await {
                                                    Ok(_) => true,
                                                    Err(e) => {
                                                        let _ = tx_clone.send(ExportUpdate::Log(format!("✖ PDF render failed [{}]: {}", page.title, e))).await;
                                                        false
                                                    }
                                                }
                                            };

                                            if pdf_ready {
                                                let mut tag_ids = base_ids_clone;
                                                if config_clone.auto_create_tags && !page.category.trim().is_empty() {
                                                    let mut cache_guard = tags_arc.lock().await;
                                                    match paperless_api::ensure_tag(&client_clone, &config_clone, &p_headers_clone, &page.category, &mut cache_guard).await {
                                                        Ok(cid) => {
                                                            if !tag_ids.contains(&cid) {
                                                                tag_ids.push(cid);
                                                            }
                                                        }
                                                        Err(e) => {
                                                            let _ = tx_clone.send(ExportUpdate::Log(format!("⚠️ Tag warning [{}]: {}", page.category, e))).await;
                                                        }
                                                    }
                                                }

                                                let _ = tx_clone.send(ExportUpdate::Log(format!("📤 Uploading '{}' to Paperless...", page.title))).await;
                                                match paperless_api::upload_document(&client_clone, &config_clone, &p_headers_clone, &pdf_path, &page.title, &tag_ids).await {
                                                    Ok(_) => {
                                                        success_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                                        let _ = tx_clone.send(ExportUpdate::Log(format!("✔ Uploaded: {} (Tags: {:?})", page.title, tag_ids))).await;
                                                    }
                                                    Err(e) => {
                                                        failed_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                                        let _ = tx_clone.send(ExportUpdate::Log(format!("✖ Upload failed [{}]: {}", page.title, e))).await;
                                                    }
                                                }
                                            } else {
                                                failed_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                            }

                                            let current = progress_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                                            let _ = tx_clone.send(ExportUpdate::Progress(current)).await;
                                            drop(permit);
                                        });
                                        tasks.push(task);
                                    }

                                    for t in tasks {
                                        let _ = t.await;
                                    }

                                    let s = success_counter.load(std::sync::atomic::Ordering::Relaxed);
                                    let f = failed_counter.load(std::sync::atomic::Ordering::Relaxed);
                                    let _ = tx.send(ExportUpdate::Finished(s, f)).await;
                                });
                            }
                        }
                        _ => {}
                    },
                    AppState::UploadingPaperless => {
                        // While uploading, ignore input except quit/esc
                    }
                    AppState::PaperlessSummary => match key.code {
                        KeyCode::Enter | KeyCode::Esc => {
                            app.state = AppState::Menu;
                            app.status_message = "Ready. Select an option to start.".to_string();
                        }
                        KeyCode::Char('q') => break,
                        _ => {}
                    },
                }
            }
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn draw_menu(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(10),
            Constraint::Min(5),
            Constraint::Length(3),
        ])
        .split(area);

    let ascii_logo = r#"
 ______               _____                          _            
|___  /              / ____|                        | |           
   / / ___ _ __ ___ | (___   ___ _ __ ___   ___  ___| |_ ___ _ __ 
  / / / _ \ '__/ _ \ \___ \ / _ \ '_ ` _ \ / _ \/ __| __/ _ \ '__|
 / /_|  __/ | | (_) |____) |  __/ | | | | |  __/\__ \ ||  __/ |   
/_____\___|_|  \___/|_____/ \___|_| |_| |_|\___||___/\__\___|_|   
    "#;

    let logo_text = Paragraph::new(ascii_logo)
        .style(Style::default().fg(Color::Cyan))
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::DarkGray)));
    f.render_widget(logo_text, chunks[0]);

    let menu_options = [
        "Export Notion Pages to Local PDF Folder (Flat/Categorized)",
        "Sync & Upload to Paperless-ngx (with Deduplication & Tags)",
    ];

    let items: Vec<ListItem> = menu_options
        .iter()
        .enumerate()
        .map(|(i, &m)| {
            if i == app.menu_index {
                ListItem::new(format!("  ▶  {}  ◀", m))
                    .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
            } else {
                ListItem::new(format!("     {}", m)).style(Style::default().fg(Color::Gray))
            }
        })
        .collect();

    let menu_list = List::new(items)
        .block(
            Block::default()
                .title(" Select Export Target ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Blue)),
        );
    f.render_widget(menu_list, chunks[1]);

    let status_text = Paragraph::new(format!("Status: {} | Up/Down to select | Enter to start | 'q' to quit", app.status_message))
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::DarkGray)));
    f.render_widget(status_text, chunks[2]);
}

fn draw_fetching(f: &mut ratatui::Frame, app: &App, area: Rect, spinner: &str) {
    let block = Block::default()
        .title(" Fetching from Notion ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(40), Constraint::Length(3), Constraint::Percentage(40)])
        .split(inner);

    let text = Paragraph::new(format!("{}  {}", spinner, app.status_message))
        .alignment(Alignment::Center)
        .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD));
    f.render_widget(text, chunks[1]);
}

fn draw_page_selection(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(3),
        ])
        .split(area);

    let selected_count = app.pages.iter().filter(|p| p.selected).count();
    let mode_text = if app.use_category_folders {
        "Categorized Subfolders"
    } else {
        "Flat Directory (Default)"
    };
    let header = Paragraph::new(format!(
        " 📄 Select Pages to Export (Selected: {}/{}) | Mode: [{}] (Press Tab or 'c' to toggle) ",
        selected_count,
        app.pages.len(),
        mode_text
    ))
    .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
    .alignment(Alignment::Center)
    .block(Block::default().borders(Borders::ALL));
    f.render_widget(header, chunks[0]);

    let items: Vec<ListItem> = app
        .pages
        .iter()
        .map(|p| {
            let checkbox = if p.selected { "[x]" } else { "[ ]" };
            let style = if p.selected {
                Style::default().fg(Color::White)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            let content = Line::from(vec![
                Span::styled(format!(" {} ", checkbox), if p.selected { Style::default().fg(Color::Green) } else { Style::default().fg(Color::DarkGray) }),
                Span::styled(format!("[{}] ", p.page.category), Style::default().fg(Color::Yellow)),
                Span::styled(p.page.title.clone(), style),
            ]);
            ListItem::new(content)
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .title(" Notion Workspace Pages ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Blue)),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    f.render_stateful_widget(list, chunks[1], &mut app.page_list_state);

    let footer = Paragraph::new(" ↑/↓ or k/j: Navigate | Space: Toggle | 'a': Select All | 'n': Deselect All | Tab/'c': Toggle Flat/Category | Enter: Confirm | Esc: Back ")
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::DarkGray)));
    f.render_widget(footer, chunks[2]);
}

fn draw_folder_selection(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(8),
            Constraint::Length(5),
            Constraint::Min(1),
            Constraint::Length(3),
        ])
        .split(area);

    let selected_count = app.pages.iter().filter(|p| p.selected).count();
    let base_dir = if app.export_dir.is_empty() { "./exports" } else { &app.export_dir };
    let path_example = if app.use_category_folders {
        format!("{}/<Category>/<Page Title>.pdf", base_dir)
    } else {
        format!("{}/<Page Title>.pdf", base_dir)
    };
    let mode_str = if app.use_category_folders { "Category Subfolders (ON)" } else { "Flat Directory (OFF - Default)" };

    let info_text = format!(
        "You have selected {} pages to export as PDF files.\n\nExport Structure: [{}]\n  Path structure: {}\n  (Press [Tab] at any time to toggle between Flat Directory and Category Subfolders)\n\nConfigure your export target directory below:",
        selected_count,
        mode_str,
        path_example
    );

    let info_block = Paragraph::new(info_text)
        .block(Block::default().title(" Export Configuration ").borders(Borders::ALL))
        .wrap(Wrap { trim: true });
    f.render_widget(info_block, chunks[0]);

    let input_text = Paragraph::new(format!("{}█", app.export_dir))
        .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
        .block(
            Block::default()
                .title(" Target Folder Path (Type to modify) ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green)),
        );
    f.render_widget(input_text, chunks[1]);

    let footer = Paragraph::new(" Enter: Start PDF Export | Tab: Toggle Category Folders | Backspace/Char: Edit Path | Esc: Back ")
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::DarkGray)));
    f.render_widget(footer, chunks[3]);
}

fn draw_exporting(f: &mut ratatui::Frame, app: &App, area: Rect, spinner: &str) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(5),
            Constraint::Length(3),
        ])
        .split(area);

    let percent = if app.export_total > 0 {
        ((app.export_progress as f64 / app.export_total as f64) * 100.0) as u16
    } else {
        0
    };

    let gauge = Gauge::default()
        .block(Block::default().title(format!(" {} Exporting PDF Files... ", spinner)).borders(Borders::ALL))
        .gauge_style(Style::default().fg(Color::Green).bg(Color::DarkGray))
        .percent(percent)
        .label(format!("{} / {} ({}%)", app.export_progress, app.export_total, percent));
    f.render_widget(gauge, chunks[0]);

    let log_items: Vec<ListItem> = app
        .export_logs
        .iter()
        .rev()
        .take(area.height as usize)
        .map(|log| {
            let style = if log.contains("✔") {
                Style::default().fg(Color::Green)
            } else if log.contains("✖") {
                Style::default().fg(Color::Red)
            } else {
                Style::default().fg(Color::Cyan)
            };
            ListItem::new(log.clone()).style(style)
        })
        .collect();

    let logs_list = List::new(log_items).block(
        Block::default()
            .title(" Live Export Activity (Most Recent at Top) ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Blue)),
    );
    f.render_widget(logs_list, chunks[1]);

    let footer = Paragraph::new(" Please wait while pages are being converted to PDF and saved to disk... ")
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::DarkGray)));
    f.render_widget(footer, chunks[2]);
}

fn draw_summary(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" 🎉 Export Completed! ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(20), Constraint::Min(8), Constraint::Percentage(20), Constraint::Length(3)])
        .split(inner);

    let summary_text = format!(
        "PDF Export Run Complete!\n\n✔ Successfully Saved: {} pages\n✖ Failed: {} pages\n📁 Target Location: {}\n\nCheck your folder to view the organized PDF files.",
        app.summary_success, app.summary_failed, app.export_dir
    );

    let text_widget = Paragraph::new(summary_text)
        .alignment(Alignment::Center)
        .style(Style::default().fg(Color::White))
        .block(Block::default().borders(Borders::NONE));
    f.render_widget(text_widget, chunks[1]);

    let footer = Paragraph::new(" Enter / Esc: Return to Main Menu | 'q': Quit ")
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::DarkGray)));
    f.render_widget(footer, chunks[3]);
}

fn draw_paperless_config(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(8), Constraint::Min(10), Constraint::Length(3)])
        .split(area);

    let header = Paragraph::new(" 🔗 Configure Paperless-ngx Integration \nConnect to your Paperless server to inspect existing tags and check for duplicated documents before uploading.")
        .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(header, chunks[0]);

    let url_style = if app.paperless_field_index == 0 { Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD) } else { Style::default() };
    let token_style = if app.paperless_field_index == 1 { Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD) } else { Style::default() };
    let auto_tag_style = if app.paperless_field_index == 2 { Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD) } else { Style::default() };
    let base_tag_style = if app.paperless_field_index == 3 { Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD) } else { Style::default() };
    let base_name_style = if app.paperless_field_index == 4 { Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD) } else { Style::default() };

    let token_display = if app.paperless_config.token.is_empty() {
        "<Empty - type to set>".to_string()
    } else if app.paperless_field_index == 1 {
        app.paperless_config.token.clone()
    } else {
        "••••••••••••••••••••••••••••••••".to_string()
    };

    let items = vec![
        ListItem::new(Line::from(vec![
            Span::styled(if app.paperless_field_index == 0 { "▶ URL: " } else { "  URL: " }, url_style),
            Span::raw(&app.paperless_config.url),
        ])),
        ListItem::new(Line::from(vec![
            Span::styled(if app.paperless_field_index == 1 { "▶ API Token: " } else { "  API Token: " }, token_style),
            Span::raw(token_display),
        ])),
        ListItem::new(Line::from(vec![
            Span::styled(if app.paperless_field_index == 2 { "▶ Auto-create Category Tags: " } else { "  Auto-create Category Tags: " }, auto_tag_style),
            Span::raw(if app.paperless_config.auto_create_tags { "[x] Yes (Matches existing Paperless tags or creates new)" } else { "[ ] No" }),
        ])),
        ListItem::new(Line::from(vec![
            Span::styled(if app.paperless_field_index == 3 { "▶ Add Base Tag to All: " } else { "  Add Base Tag to All: " }, base_tag_style),
            Span::raw(if app.paperless_config.add_base_tag { "[x] Yes" } else { "[ ] No" }),
        ])),
        ListItem::new(Line::from(vec![
            Span::styled(if app.paperless_field_index == 4 { "▶ Base Tag Name: " } else { "  Base Tag Name: " }, base_name_style),
            Span::raw(&app.paperless_config.base_tag_name),
        ])),
    ];

    let list = List::new(items).block(Block::default().title(" Settings ").borders(Borders::ALL).border_style(Style::default().fg(Color::Blue)));
    f.render_widget(list, chunks[1]);

    let footer = Paragraph::new(" ↑/↓/Tab: Switch Field | Type: Edit | Space: Toggle Checkbox | Enter: Connect & Fetch Metadata | Esc: Back ")
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::DarkGray)));
    f.render_widget(footer, chunks[2]);
}

fn draw_fetching_paperless(f: &mut ratatui::Frame, app: &App, area: Rect, spinner: &str) {
    let block = Block::default()
        .title(" 🔗 Inspecting Paperless-ngx & Notion ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(40), Constraint::Length(3), Constraint::Percentage(40)])
        .split(inner);

    let text = Paragraph::new(format!("{}  {}", spinner, app.status_message))
        .alignment(Alignment::Center)
        .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD));
    f.render_widget(text, chunks[1]);
}

fn draw_paperless_page_selection(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(5), Constraint::Min(5), Constraint::Length(3)])
        .split(area);

    let selected_count = app.paperless_pages.iter().filter(|p| p.selected).count();
    let already_count = app.paperless_pages.iter().filter(|p| p.already_in_paperless).count();

    let header_text = format!(
        " 🏷️ Select Pages to Sync with Paperless-ngx \nSelected to upload: {}/{} | Already in Paperless: {} | Paperless Tags available: {} ",
        selected_count,
        app.paperless_pages.len(),
        already_count,
        app.paperless_tags.len()
    );
    let header = Paragraph::new(header_text)
        .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(header, chunks[0]);

    let items: Vec<ListItem> = app
        .paperless_pages
        .iter()
        .map(|p| {
            let checkbox = if p.selected { "[x]" } else { "[ ]" };
            let status_span = if p.already_in_paperless {
                Span::styled(" ✔ [ALREADY IN PAPERLESS] ", Style::default().fg(Color::DarkGray))
            } else {
                Span::styled(" ✨ [NEW - READY TO UPLOAD] ", Style::default().fg(Color::Green))
            };
            let style = if p.selected {
                Style::default().fg(Color::White)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            let content = Line::from(vec![
                Span::styled(format!(" {} ", checkbox), if p.selected { Style::default().fg(Color::Green) } else { Style::default().fg(Color::DarkGray) }),
                status_span,
                Span::styled(format!("[{}] ", p.page.category), Style::default().fg(Color::Yellow)),
                Span::styled(p.page.title.clone(), style),
            ]);
            ListItem::new(content)
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .title(" Notion Pages vs Paperless Inventory ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Blue)),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    f.render_stateful_widget(list, chunks[1], &mut app.paperless_list_state);

    let footer = Paragraph::new(" ↑/↓ or k/j: Navigate | Space: Toggle | 'a': Select All New | 'n': Deselect All | Enter: Start Upload | Esc: Back ")
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::DarkGray)));
    f.render_widget(footer, chunks[2]);
}

fn draw_uploading_paperless(f: &mut ratatui::Frame, app: &App, area: Rect, spinner: &str) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(5), Constraint::Length(3)])
        .split(area);

    let percent = if app.paperless_total > 0 {
        ((app.paperless_progress as f64 / app.paperless_total as f64) * 100.0) as u16
    } else {
        0
    };

    let gauge = Gauge::default()
        .block(Block::default().title(format!(" {} Uploading to Paperless-ngx... ", spinner)).borders(Borders::ALL))
        .gauge_style(Style::default().fg(Color::Green).bg(Color::DarkGray))
        .percent(percent)
        .label(format!("{} / {} ({}%)", app.paperless_progress, app.paperless_total, percent));
    f.render_widget(gauge, chunks[0]);

    let log_items: Vec<ListItem> = app
        .paperless_logs
        .iter()
        .rev()
        .take(area.height as usize)
        .map(|log| {
            let style = if log.contains("✔") {
                Style::default().fg(Color::Green)
            } else if log.contains("✖") {
                Style::default().fg(Color::Red)
            } else {
                Style::default().fg(Color::Cyan)
            };
            ListItem::new(log.clone()).style(style)
        })
        .collect();

    let logs_list = List::new(log_items).block(
        Block::default()
            .title(" Live Paperless Sync Activity (Most Recent at Top) ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Blue)),
    );
    f.render_widget(logs_list, chunks[1]);

    let footer = Paragraph::new(" Uploading documents & assigning tags... ")
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::DarkGray)));
    f.render_widget(footer, chunks[2]);
}

fn draw_paperless_summary(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" 🎉 Paperless-ngx Sync Completed! ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(20), Constraint::Min(8), Constraint::Percentage(20), Constraint::Length(3)])
        .split(inner);

    let summary_text = format!(
        "Paperless-ngx Upload Complete!\n\n✔ Successfully Uploaded: {} pages\n✖ Failed: {} pages\n🔗 Target Server: {}\n\nCheck your Paperless-ngx web interface to view your synced documents with assigned tags.",
        app.summary_success, app.summary_failed, app.paperless_config.url
    );

    let text_widget = Paragraph::new(summary_text)
        .alignment(Alignment::Center)
        .style(Style::default().fg(Color::White))
        .block(Block::default().borders(Borders::NONE));
    f.render_widget(text_widget, chunks[1]);

    let footer = Paragraph::new(" Enter / Esc: Return to Main Menu | 'q': Quit ")
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::DarkGray)));
    f.render_widget(footer, chunks[3]);
}