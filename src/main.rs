use std::io;

use anacleto::config::loader;
use anacleto::engine::orchestrator::{Engine, EngineCommand, EngineEvent};
use anacleto::tui::app::{App, run_tui};

use clap::Parser;
use crossterm::event::{
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio::sync::mpsc;

/// Anacleto — Agent orchestration engine.
#[derive(Parser, Debug)]
#[command(
    name = "anacleto",
    version,
    about = "Agent orchestration engine in Rust"
)]
struct Cli {
    /// Path to a project config file (overrides auto-detection).
    #[arg(short, long)]
    config: Option<String>,

    /// Database path (overrides config).
    #[arg(short, long)]
    database: Option<String>,

    /// Enable verbose logging.
    #[arg(short, long)]
    verbose: bool,

    /// Enable debug mode (show LLM request/response payloads).
    #[arg(long)]
    debug: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(if cli.verbose {
            "anacleto=debug"
        } else {
            "anacleto=info"
        })
        .init();

    // Load configuration
    let mut config = loader::load_config(cli.config.as_deref().map(std::path::Path::new))?;
    config.session.debug = cli.debug;

    // Initialize the shell tool inventory with any configuration overrides.
    let overrides: Vec<anacleto::shell::ToolInfo> = config
        .shell
        .tools
        .iter()
        .map(|t| {
            anacleto::shell::ToolInfo::new(t.name.clone(), t.classic.clone(), t.description.clone())
        })
        .collect();
    anacleto::shell::init(&overrides);

    // Create communication channels
    let (event_tx, event_rx) = mpsc::channel::<EngineEvent>(256);
    let (cmd_tx, cmd_rx) = mpsc::channel::<EngineCommand>(64);

    // Initialize engine
    let mut engine = Engine::new(config, event_tx.clone(), cmd_rx);
    engine.initialize().await?;

    // Setup terminal
    let mut stdout = io::stdout();
    crossterm::terminal::enable_raw_mode()?;
    // Probe whether the terminal supports the Kitty keyboard enhancement protocol
    // (writes to /dev/tty, does NOT need a specific handle)
    let kb_supported = crossterm::terminal::supports_keyboard_enhancement()?;
    let backend = CrosstermBackend::new(&mut stdout);
    let mut terminal = Terminal::new(backend)?;
    crossterm::execute!(io::stdout(), crossterm::terminal::EnterAlternateScreen)?;
    // Push the protocol flags AFTER the alternate screen is entered, so the
    // escape sequence is not consumed during terminal initialization.
    if kb_supported {
        crossterm::execute!(
            io::stdout(),
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
            )
        )?;
    }

    // Run engine and TUI concurrently
    let engine_handle = tokio::spawn(async move {
        if let Err(e) = engine.run().await {
            eprintln!("Engine error: {}", e);
        }
    });

    let mut app = App::new(cmd_tx, event_rx, kb_supported);
    let tui_result = run_tui(&mut terminal, &mut app).await;

    // Cleanup
    crossterm::execute!(io::stdout(), PopKeyboardEnhancementFlags)?;
    crossterm::execute!(io::stdout(), crossterm::terminal::LeaveAlternateScreen)?;
    crossterm::terminal::disable_raw_mode()?;

    // Send shutdown to engine
    let _ = app.cmd_tx.try_send(EngineCommand::Shutdown);
    engine_handle.await.ok();

    tui_result.map_err(|e| anyhow::anyhow!("TUI error: {}", e))
}
