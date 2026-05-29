use std::io;
use std::time::Duration;

use anyhow::Result;
use capsem_tui::fixture::FixtureProvider;
use capsem_tui::provider::StateProvider;
use capsem_tui::ui::{render, render_snapshot};
use clap::Parser;
use crossterm::event::{self, Event, KeyCode};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

#[derive(Parser)]
#[command(
    author,
    version,
    about = "Standalone Capsem terminal control UI prototype"
)]
struct Cli {
    /// Print a deterministic text rendering instead of opening the terminal UI.
    #[arg(long)]
    snapshot: bool,

    /// Snapshot width.
    #[arg(long, default_value_t = 100)]
    width: u16,

    /// Snapshot height.
    #[arg(long, default_value_t = 24)]
    height: u16,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let provider = FixtureProvider;
    let state = provider.load()?;

    if cli.snapshot {
        println!("{}", render_snapshot(&state, cli.width, cli.height)?);
        return Ok(());
    }

    run_interactive(&state)
}

fn run_interactive(state: &capsem_tui::model::AppState) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_loop(&mut terminal, state);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &capsem_tui::model::AppState,
) -> Result<()> {
    loop {
        terminal.draw(|frame| render(frame, state))?;
        if event::poll(Duration::from_millis(250))? {
            match event::read()? {
                Event::Key(key) if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) => {
                    break;
                }
                _ => {}
            }
        }
    }
    Ok(())
}
