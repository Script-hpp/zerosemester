mod auth;
mod notion_api;

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
}

struct SelectablePage {
    page: notion_api::NotionPage,
    selected: bool,
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
}

impl App {
    fn new() -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
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
                                app.status_message = "Paperless-ngx integration is coming soon!".to_string();
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
                        KeyCode::Tab => {
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

    let menu_options = ["Export Notion Pages to Local PDF Folder (Flat/Categorized)", "Upload to Paperless-ngx (Coming Soon)"];

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
    let header = Paragraph::new(format!(
        " 📄 Select Pages to Export (Selected: {}/{}) ",
        selected_count,
        app.pages.len()
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

    let footer = Paragraph::new(" ↑/↓ or k/j: Navigate | Space: Toggle | 'a': Select All | 'n': Deselect All | Enter: Confirm | Esc: Back ")
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