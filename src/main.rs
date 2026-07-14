mod auth;
mod notion_api;

use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Terminal,
};
use std::{io, time::Duration};

enum ExportTarget {
    LocalFolder,
    PaperlessNgx,
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
 
    let mut selected_index = 0;
 
    loop {
        terminal.draw(|f| {
            let size = f.area();
 
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(10),
                    Constraint::Min(5),
                    Constraint::Length(3),
                ])
                .split(size);
 
            let ascii_logo = r#"
 ______               _____                          _            
|___  /              / ____|                        | |           
   / / ___ _ __ ___ | (___   ___ _ __ ___   ___  ___| |_ ___ _ __ 
  / / / _ \ '__/ _ \ \___ \ / _ \ '_ ` _ \ / _ \/ __| __/ _ \ '__|
 / /_|  __/ | | (_) |____) |  __/ | | | | |  __/\__ \ ||  __/ |   
/_____\___|_|  \___/|_____/ \___|_| |_| |_|\___||___/\__\___|_|   
            "#;
 
            let logo_text = Paragraph::new(ascii_logo)
                .alignment(Alignment::Center)
                .block(Block::default().borders(Borders::ALL));
            f.render_widget(logo_text, chunks[0]);
 
            let menu_options = ["Export to local folder", "Upload to Paperless-ngx"];
 
            let items: Vec<ListItem> = menu_options
                .iter()
                .enumerate()
                .map(|(i, &m)| {
                    if i == selected_index {
                        ListItem::new(format!("  >> {} <<", m))
                            .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
                    } else {
                        ListItem::new(format!("     {}", m))
                    }
                })
                .collect();
 
            let menu_list = List::new(items)
                .block(Block::default().title(" Select export target ").borders(Borders::ALL));
            f.render_widget(menu_list, chunks[1]);
 
            let status_text = Paragraph::new("Status: Up/Down to select | Enter to start | 'q' to quit")
                .alignment(Alignment::Center)
                .block(Block::default().borders(Borders::ALL));
            f.render_widget(status_text, chunks[2]);
        })?;
 
        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Up => {
                        if selected_index > 0 {
                            selected_index -= 1;
                        }
                    }
                    KeyCode::Down => {
                        if selected_index < 1 {
                            selected_index += 1;
                        }
                    }
                    KeyCode::Enter => {
                        if !std::path::Path::new("notion_token.txt").exists() {
                            auth::authenticate_with_notion().await;
                        }
 
                        let pages = notion_api::fetch_pages().await;
                        match pages {
                            Ok(page_list) => {
                                // TODO: show page_list in the TUI instead of printing to stdout
                                println!("Pages found: {:?}", page_list);
                            }
                            Err(e) => {
                                println!("Error fetching pages: {:?}", e);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
 
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}