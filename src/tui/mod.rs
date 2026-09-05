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
    let chat_client = crate::chat::ChatClient::local("127.0.0.1:8080".parse()?);
    let mut chat_events: Option<tokio::sync::mpsc::Receiver<crate::chat::ChatEvent>> = None;
    let mut hub = crate::hub::HubHandle::start(app.paths.models_dir.clone())?;

    while app.running {
        while let Ok(event) = runtime.events.try_recv() {
            app.apply_runtime_event(event);
        }
        while let Some(command) = app.take_hub_command() {
            hub.command(command).await?;
        }
        while let Ok(event) = hub.events.try_recv() {
            app.apply_hub_event(event);
        }
        if let Some(receiver) = &mut chat_events {
            while let Ok(event) = receiver.try_recv() {
                app.apply_chat_event(event);
            }
        }
        if last_telemetry.elapsed() >= Duration::from_secs(1) {
            app.refresh_telemetry();
            last_telemetry = Instant::now();
        }
        terminal.draw(|frame| ui::draw(frame, &app))?;
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match app.handle_key(key) {
                        app::AppAction::Runtime(command) => runtime.command(command).await?,
                        app::AppAction::Hub(command) => hub.command(command).await?,
                        app::AppAction::SendChat(messages) => {
                            chat_events = Some(chat_client.stream_completion(messages, 0.7));
                        }
                        app::AppAction::None => {}
                    }
                }
            }
        }
    }
    runtime.shutdown().await;
    Ok(())
}
