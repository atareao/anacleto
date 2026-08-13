use std::io::{self, Write};
use std::time::Duration;

use anacleto::agent::types::AgentStatus;
use anacleto::config::loader;
use anacleto::engine::orchestrator::{Engine, EngineCommand, EngineEvent};
use anacleto::tui::app::{App, run_tui};

use clap::Parser;
use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio::sync::mpsc;
use tracing_subscriber::prelude::*;

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

    /// Run in headless mode (no TUI, output to stdout).
    #[arg(long)]
    headless: bool,

    /// Initial task/prompt for headless mode (ignored otherwise).
    #[arg(long)]
    task: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Initialize logging: stdout + file with daily rotation
    let log_filter = if cli.verbose {
        "anacleto=debug"
    } else {
        "anacleto=info"
    };

    // File appender with daily rotation
    let log_dir = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("anacleto")
        .join("logs");
    std::fs::create_dir_all(&log_dir).ok();
    let file_appender = tracing_appender::rolling::daily(&log_dir, "anacleto.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    let filter = tracing_subscriber::EnvFilter::new(log_filter);
    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(non_blocking)
        .with_ansi(false)
        .with_filter(filter.clone());
    let stdout_layer = tracing_subscriber::fmt::layer().with_filter(filter);

    tracing_subscriber::registry()
        .with(file_layer)
        .with(stdout_layer)
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
    // The event channel is large because during LLM streaming each token
    // becomes an `AgentStreamChunk` event; a small capacity would fill up
    // quickly and block the engine, slowing down the whole pipeline.
    let (event_tx, event_rx) = mpsc::channel::<EngineEvent>(4096);
    let (cmd_tx, cmd_rx) = mpsc::channel::<EngineCommand>(64);

    // Initialize engine
    let mut engine = Engine::new(config.clone(), event_tx.clone(), cmd_rx);
    engine.initialize().await?;

    // SIGHUP handler for config hot-reload
    let sighup_cmd_tx = cmd_tx.clone();
    tokio::spawn(async move {
        use tokio::signal::unix::{SignalKind, signal};
        let mut stream = match signal(SignalKind::hangup()) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("Could not install SIGHUP handler: {}", e);
                return;
            }
        };
        loop {
            stream.recv().await;
            tracing::info!("Received SIGHUP, reloading config...");
            let _ = sighup_cmd_tx.send(EngineCommand::ReloadConfig).await;
        }
    });

    if cli.headless {
        // Headless mode: no TUI, run engine in background
        // If a task is provided, send it as user input
        if let Some(task) = cli.task {
            cmd_tx.send(EngineCommand::UserInput(task)).await?;
        }

        // Spawn a monitor task that prints events to stdout and
        // sends Shutdown when the agent goes back to Idle.
        let shutdown_tx = cmd_tx.clone();
        let mut monitor_rx = event_rx;
        tokio::spawn(async move {
            while let Some(event) = monitor_rx.recv().await {
                match event {
                    EngineEvent::AgentStreamChunk { content, .. } => {
                        print!("{}", content);
                        let _ = io::stdout().flush();
                    }
                    EngineEvent::AgentOutput { content, .. } => {
                        println!("{}", content);
                    }
                    EngineEvent::AgentStatusChanged { status, .. } => {
                        if status == AgentStatus::Idle {
                            // Give a moment for final events to flush
                            tokio::time::sleep(Duration::from_millis(500)).await;
                            let _ = shutdown_tx.send(EngineCommand::Shutdown).await;
                            break;
                        }
                    }
                    EngineEvent::ShuttingDown => break,
                    EngineEvent::Error { message, .. } => {
                        eprintln!("Error: {}", message);
                    }
                    _ => {}
                }
            }
        });

        // Run engine (blocks until shutdown)
        engine.run().await?;
    } else {
        // Setup terminal
        let mut stdout = io::stdout();
        crossterm::terminal::enable_raw_mode()?;
        // Probe whether the terminal supports the Kitty keyboard enhancement protocol
        // (writes to /dev/tty, does NOT need a specific handle)
        let kb_supported = crossterm::terminal::supports_keyboard_enhancement()?;
        let backend = CrosstermBackend::new(&mut stdout);
        let mut terminal = Terminal::new(backend)?;
        crossterm::execute!(io::stdout(), crossterm::terminal::EnterAlternateScreen)?;
        crossterm::execute!(io::stdout(), EnableMouseCapture)?;
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

        let mut app = App::new(cmd_tx, event_rx, kb_supported, &config);
        let tui_result = run_tui(&mut terminal, &mut app).await;

        // Cleanup
        crossterm::execute!(io::stdout(), DisableMouseCapture)?;
        crossterm::execute!(io::stdout(), PopKeyboardEnhancementFlags)?;
        crossterm::execute!(io::stdout(), crossterm::terminal::LeaveAlternateScreen)?;
        crossterm::terminal::disable_raw_mode()?;

        // Send shutdown to engine
        let _ = app.cmd_tx.try_send(EngineCommand::Shutdown);
        engine_handle.await.ok();

        tui_result.map_err(|e| anyhow::anyhow!("TUI error: {}", e))?;
    }

    Ok(())
}
