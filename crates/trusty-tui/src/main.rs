//! trusty-tui — terminal UI for trusty-izzie.
//!
//! Layout: left panel (entities sidebar) | right panel (chat).

use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame, Terminal,
};
use std::io;

use trusty_core::{init_logging, load_config};

/// trusty-tui command-line arguments.
#[derive(Parser)]
#[command(name = "trusty-tui", about = "trusty-izzie terminal UI")]
struct Cli {
    /// Path to a custom configuration file.
    #[arg(long)]
    config: Option<String>,
}

/// Application state for the TUI.
struct App {
    /// Messages displayed in the chat panel (role, content).
    messages: Vec<(String, String)>,
    /// Current input buffer.
    input: String,
    /// Entity names displayed in the sidebar.
    entities: Vec<String>,
    /// Whether the application is running.
    running: bool,
    /// Which panel is focused.
    focus: Focus,
}

#[derive(PartialEq, Eq)]
enum Focus {
    Chat,
    Entities,
}

impl App {
    fn new() -> Self {
        Self {
            messages: vec![(
                "assistant".to_string(),
                "Hi! I'm trusty-izzie. How can I help you today?".to_string(),
            )],
            input: String::new(),
            entities: vec!["Loading entities...".to_string()],
            running: true,
            focus: Focus::Chat,
        }
    }

    /// Handle a key event and return whether the UI should redraw.
    fn handle_key(&mut self, event: event::KeyEvent) -> bool {
        match event.code {
            // Quit
            KeyCode::Char('q') if event.modifiers.contains(KeyModifiers::CONTROL) => {
                self.running = false;
            }
            KeyCode::Esc => {
                self.running = false;
            }

            // Tab to switch focus
            KeyCode::Tab => {
                self.focus = match self.focus {
                    Focus::Chat => Focus::Entities,
                    Focus::Entities => Focus::Chat,
                };
            }

            // Chat input
            KeyCode::Char(c) if self.focus == Focus::Chat => {
                self.input.push(c);
            }
            KeyCode::Backspace if self.focus == Focus::Chat => {
                self.input.pop();
            }
            KeyCode::Enter if self.focus == Focus::Chat => {
                if !self.input.is_empty() {
                    let msg = std::mem::take(&mut self.input);
                    self.messages.push(("user".to_string(), msg));
                    // TODO: send to ChatEngine and push assistant reply
                    self.messages.push((
                        "assistant".to_string(),
                        "(thinking... chat engine not yet connected)".to_string(),
                    ));
                }
            }

            _ => return false,
        }
        true
    }
}

fn render(frame: &mut Frame, app: &App) {
    // Split screen: 25% sidebar | 75% chat
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(25), Constraint::Percentage(75)])
        .split(frame.area());

    render_entities(frame, app, chunks[0]);
    render_chat(frame, app, chunks[1]);
}

fn render_entities(frame: &mut Frame, app: &App, area: Rect) {
    let border_style = if app.focus == Focus::Entities {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };

    let items: Vec<ListItem> = app
        .entities
        .iter()
        .map(|e| ListItem::new(e.as_str()))
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .title(" Entities [Tab] ")
                .borders(Borders::ALL)
                .border_style(border_style),
        )
        .highlight_style(Style::default().add_modifier(Modifier::BOLD));

    frame.render_widget(list, area);
}

fn render_chat(frame: &mut Frame, app: &App, area: Rect) {
    // Split chat area: messages (top) + input (bottom)
    let chat_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(3)])
        .split(area);

    let border_style = if app.focus == Focus::Chat {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };

    // Render messages
    let message_lines: Vec<Line> = app
        .messages
        .iter()
        .flat_map(|(role, content)| {
            let role_style = match role.as_str() {
                "user" => Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
                "assistant" => Style::default()
                    .fg(Color::Blue)
                    .add_modifier(Modifier::BOLD),
                _ => Style::default(),
            };
            let header = Line::from(Span::styled(format!("{}: ", role), role_style));
            let body = Line::from(content.as_str());
            let spacer = Line::from("");
            vec![header, body, spacer]
        })
        .collect();

    let messages = Paragraph::new(message_lines)
        .block(
            Block::default()
                .title(" Chat ")
                .borders(Borders::ALL)
                .border_style(border_style),
        )
        .wrap(Wrap { trim: false });

    frame.render_widget(messages, chat_chunks[0]);

    // Render input box
    let input = Paragraph::new(app.input.as_str())
        .block(
            Block::default()
                .title(" Message (Enter to send, Ctrl-Q to quit) ")
                .borders(Borders::ALL)
                .border_style(border_style),
        )
        .wrap(Wrap { trim: true });

    frame.render_widget(input, chat_chunks[1]);
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let log_level = std::env::var("TRUSTY_LOG_LEVEL").unwrap_or_else(|_| "warn".to_string());
    init_logging(&log_level);

    let _config = load_config(cli.config.as_deref()).await?;

    // Set up terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();

    // Main event loop
    while app.running {
        terminal.draw(|frame| render(frame, &app))?;

        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                app.handle_key(key);
            }
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    Ok(())
}
