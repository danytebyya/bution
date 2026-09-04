mod app;
mod ui;

pub use app::{App, Screen};

use anyhow::Result;
use crossterm::cursor::{Hide, Show};
use crossterm::event::{self, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::io::stdout;
use std::time::{Duration, Instant};

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(stdout(), Show, LeaveAlternateScreen);
    }
}

pub async fn run(mut app: App) -> Result<()> {
    let mut runtime = crate::runtime::RuntimeHandle::start(
        app.settings.clone(),
        app.paths.clone(),
        app.hardware.clone(),
        app.interfaces.clone(),
    )
    .await?;
    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen, Hide)?;
    let _guard = TerminalGuard;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    terminal.clear()?;
    let mut last_telemetry = Instant::now();

    while app.running {
        while let Ok(event) = runtime.events.try_recv() {
            app.apply_runtime_event(event);
        }
        if last_telemetry.elapsed() >= Duration::from_secs(1) {
            app.refresh_telemetry();
            last_telemetry = Instant::now();
        }
        terminal.draw(|frame| ui::draw(frame, &app))?;
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    if let app::AppAction::Runtime(command) = app.handle_key(key) {
                        runtime.command(command).await?;
                    }
                }
            }
        }
    }
    runtime.shutdown().await;
    Ok(())
}
