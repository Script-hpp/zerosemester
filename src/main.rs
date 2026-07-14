mod auth;

use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style}, // NEU: Für Farben und fette Schrift
    widgets::{Block, Borders, List, ListItem, Paragraph}, // NEU: List und ListItem
    Terminal,
};
use std::{io, time::Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // NEU: Unser State. 0 = Folder Export, 1 = Paperless Export
    let mut selected_index = 0;

    loop {
        terminal.draw(|f| {
            let size = f.size();
            
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

            // NEU: Das interaktive Menü bauen
            let menu_options = ["Lokaler Ordner Export (Backup)", "Paperless-ngx Upload (OAuth)"];
            
            // Wir wandeln unsere Text-Optionen in klickbare ListItems um
            let items: Vec<ListItem> = menu_options
                .iter()
                .enumerate()
                .map(|(i, &m)| {
                    if i == selected_index {
                        // Das ausgewählte Item wird gelb, fett und bekommt Pfeile
                        ListItem::new(format!("  >> {} <<", m))
                            .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
                    } else {
                        // Normale Items bleiben schlicht
                        ListItem::new(format!("     {}", m))
                    }
                })
                .collect();

            let list_block = Block::default()
                .title(" Wähle den Export-Modus ")
                .borders(Borders::ALL);
                
            let menu_list = List::new(items).block(list_block);
            f.render_widget(menu_list, chunks[1]);

            let status_block = Block::default().borders(Borders::ALL);
            let status_text = Paragraph::new("Status: Wähle mit Hoch/Runter | 'Enter' zum Starten | 'q' zum Beenden")
                .alignment(Alignment::Center)
                .block(status_block);
            f.render_widget(status_text, chunks[2]);
        })?;

        // Tasteneingaben verarbeiten
        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Up => {
                        // Wenn wir nicht schon ganz oben sind (0), gehen wir eins hoch
                        if selected_index > 0 {
                            selected_index -= 1;
                        }
                    }
                    KeyCode::Down => {
                        // Wenn wir nicht schon ganz unten sind (1), gehen wir eins runter
                        if selected_index < 1 {
                            selected_index += 1;
                        }
                    }
                    KeyCode::Enter => {
                        // Hier lösen wir später die Aktion aus!
                        // Im Moment machen wir noch nichts.
                    }
                    _ => {}
                }
            }
        }
    }

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;

    Ok(())
}