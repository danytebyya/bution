use super::{App, Screen};
use crate::hardware::bytes_to_gib;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Gauge, List, ListItem, Paragraph, Wrap};

const LOGO: &str = r#"██████╗ ██╗   ██╗████████╗██╗ ██████╗ ███╗   ██╗
██╔══██╗██║   ██║╚══██╔══╝██║██╔═══██╗████╗  ██║
██████╔╝██║   ██║   ██║   ██║██║   ██║██╔██╗ ██║
██╔══██╗██║   ██║   ██║   ██║██║   ██║██║╚██╗██║
██████╔╝╚██████╔╝   ██║   ██║╚██████╔╝██║ ╚████║
╚═════╝  ╚═════╝    ╚═╝   ╚═╝ ╚═════╝ ╚═╝  ╚═══╝"#;

const CYAN: Color = Color::Rgb(76, 201, 240);
const BLUE: Color = Color::Rgb(67, 97, 238);
const MUTED: Color = Color::Rgb(125, 133, 151);
const PANEL: Color = Color::Rgb(30, 35, 48);

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(if area.height >= 32 { 9 } else { 3 }),
            Constraint::Min(12),
            Constraint::Length(3),
        ])
        .split(area);
    draw_header(frame, rows[0], area.height >= 32);
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(20),
            Constraint::Min(40),
            Constraint::Length(28),
        ])
        .split(rows[1]);
    draw_navigation(frame, columns[0], app);
    draw_content(frame, columns[1], app);
    draw_telemetry(frame, columns[2], app);
    draw_help(frame, rows[2]);
}

fn draw_header(frame: &mut Frame<'_>, area: Rect, show_logo: bool) {
    let text = if show_logo {
        let mut text =
            Text::from(LOGO).style(Style::default().fg(CYAN).add_modifier(Modifier::BOLD));
        text.lines.push(Line::styled(
            "Distributed Local AI Cluster",
            Style::default().fg(MUTED),
        ));
        text
    } else {
        Text::from(Line::from(vec![
            Span::styled(
                "BUTION",
                Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
            ),
            Span::styled("  Distributed Local AI Cluster", Style::default().fg(MUTED)),
        ]))
    };
    frame.render_widget(Paragraph::new(text).alignment(Alignment::Center), area);
}

fn panel(title: &str) -> Block<'_> {
    Block::default()
        .title(format!(" {title} "))
        .title_style(Style::default().fg(CYAN).add_modifier(Modifier::BOLD))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(PANEL))
}

fn draw_navigation(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let items = Screen::ALL.iter().enumerate().map(|(index, screen)| {
        let selected = index == app.screen_index;
        let prefix = if selected { "› " } else { "  " };
        ListItem::new(Line::from(Span::styled(
            format!("{prefix}{}", screen.label()),
            if selected {
                Style::default().fg(CYAN).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(MUTED)
            },
        )))
    });
    frame.render_widget(List::new(items).block(panel("BUTION")), area);
}

fn draw_content(frame: &mut Frame<'_>, area: Rect, app: &App) {
    match app.screen() {
        Screen::Cluster => draw_cluster(frame, area, app),
        Screen::Nodes => draw_nodes(frame, area, app),
        Screen::Models => draw_models(frame, area, app),
        Screen::Benchmark => draw_benchmark(frame, area, app),
        Screen::Chat => draw_chat(frame, area, app),
        Screen::Settings => draw_settings(frame, area, app),
    }
}

fn draw_cluster(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let model = app
        .model
        .as_ref()
        .map(|model| model.name.as_str())
        .unwrap_or("No model selected");
    let text = vec![
        Line::styled(
            "CLUSTER READY",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
        Line::from(vec![
            Span::styled("Node     ", Style::default().fg(MUTED)),
            Span::raw(&app.settings.node_name),
        ]),
        Line::from(vec![
            Span::styled("Role     ", Style::default().fg(MUTED)),
            Span::raw(format!("{:?}", app.settings.role)),
        ]),
        Line::from(vec![
            Span::styled("Backend  ", Style::default().fg(MUTED)),
            Span::raw(app.hardware.backend.to_string()),
        ]),
        Line::from(vec![
            Span::styled("Memory   ", Style::default().fg(MUTED)),
            Span::raw(format!(
                "{:.1} GiB available for AI",
                app.hardware.ai_memory_gib()
            )),
        ]),
        Line::from(vec![
            Span::styled("Model    ", Style::default().fg(MUTED)),
            Span::raw(model),
        ]),
        Line::raw(""),
        Line::styled(
            "Waiting for trusted nodes on the local network…",
            Style::default().fg(CYAN),
        ),
    ];
    frame.render_widget(
        Paragraph::new(text)
            .block(panel("CLUSTER"))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn draw_nodes(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let mut lines = Vec::new();
    for node in &app.nodes {
        lines.push(Line::from(vec![
            Span::styled("● ", Style::default().fg(Color::Green)),
            Span::styled(&node.name, Style::default().add_modifier(Modifier::BOLD)),
        ]));
        lines.push(Line::raw(format!(
            "  {} • {:.1} GiB available",
            node.compute_backend,
            bytes_to_gib(node.available_memory_bytes)
        )));
        lines.push(Line::styled(
            format!("  {:?} • {:?}", node.role, node.status),
            Style::default().fg(MUTED),
        ));
        lines.push(Line::raw(""));
    }
    frame.render_widget(Paragraph::new(lines).block(panel("NODES")), area);
}

fn draw_models(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let lines = if let Some(model) = &app.model {
        vec![
            Line::styled(
                &model.name,
                Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
            ),
            Line::raw(""),
            Line::raw(format!(
                "File size       {:.1} GiB",
                bytes_to_gib(model.file_size_bytes)
            )),
            Line::raw(format!(
                "Estimated RAM   {:.1} GiB",
                bytes_to_gib(model.estimated_memory_bytes)
            )),
            Line::raw(format!(
                "Cluster memory  {:.1} GiB",
                app.nodes
                    .iter()
                    .map(|node| node.available_memory_bytes)
                    .sum::<u64>() as f64
                    / 1_073_741_824.0
            )),
        ]
    } else {
        vec![
            Line::styled("No GGUF model selected", Style::default().fg(MUTED)),
            Line::raw(""),
            Line::raw("Start with --model /path/to/model.gguf"),
        ]
    };
    frame.render_widget(Paragraph::new(lines).block(panel("MODEL")), area);
}

fn draw_benchmark(frame: &mut Frame<'_>, area: Rect, _app: &App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Length(4),
            Constraint::Length(4),
            Constraint::Min(1),
        ])
        .split(area.inner(ratatui::layout::Margin {
            horizontal: 2,
            vertical: 2,
        }));
    frame.render_widget(
        Gauge::default()
            .block(panel("Latency"))
            .gauge_style(Style::default().fg(CYAN))
            .ratio(0.0)
            .label("Not tested"),
        rows[0],
    );
    frame.render_widget(
        Gauge::default()
            .block(panel("Bandwidth"))
            .gauge_style(Style::default().fg(BLUE))
            .ratio(0.0)
            .label("Not tested"),
        rows[1],
    );
    frame.render_widget(
        Gauge::default()
            .block(panel("Stability"))
            .gauge_style(Style::default().fg(Color::Green))
            .ratio(0.0)
            .label("Not tested"),
        rows[2],
    );
    frame.render_widget(panel("NETWORK BENCHMARK"), area);
}

fn draw_chat(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let model = app
        .model
        .as_ref()
        .map(|model| model.name.as_str())
        .unwrap_or("No model running");
    let text = vec![
        Line::styled(
            model,
            Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
        ),
        Line::styled(
            format!("{} Node(s)", app.nodes.len()),
            Style::default().fg(MUTED),
        ),
        Line::raw(""),
        Line::raw("Start a model to chat with the cluster."),
        Line::raw(""),
        Line::styled("> _", Style::default().fg(CYAN)),
    ];
    frame.render_widget(
        Paragraph::new(text)
            .block(panel("CHAT"))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_settings(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let trust = app.settings.trusted_peers.len();
    let text = vec![
        Line::from(vec![
            Span::styled("Node name       ", Style::default().fg(MUTED)),
            Span::raw(&app.settings.node_name),
        ]),
        Line::from(vec![
            Span::styled("Permanent UUID  ", Style::default().fg(MUTED)),
            Span::raw(app.settings.node_id.to_string()),
        ]),
        Line::from(vec![
            Span::styled("Mode             ", Style::default().fg(MUTED)),
            Span::styled(
                format!("{:?}", app.settings.role),
                Style::default().fg(CYAN),
            ),
        ]),
        Line::from(vec![
            Span::styled("Control port     ", Style::default().fg(MUTED)),
            Span::raw(app.settings.control_port.to_string()),
        ]),
        Line::from(vec![
            Span::styled("RPC port         ", Style::default().fg(MUTED)),
            Span::raw(app.settings.rpc_port.to_string()),
        ]),
        Line::from(vec![
            Span::styled("Trusted nodes    ", Style::default().fg(MUTED)),
            Span::raw(trust.to_string()),
        ]),
        Line::raw(""),
        Line::styled(
            "Space cycles Automatic / Main / Worker",
            Style::default().fg(CYAN),
        ),
    ];
    frame.render_widget(Paragraph::new(text).block(panel("SETTINGS")), area);
}

fn draw_telemetry(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let sample = &app.telemetry;
    let used = bytes_to_gib(sample.memory_used_bytes);
    let total = bytes_to_gib(sample.memory_total_bytes);
    let network = sample.network_receive_mbps + sample.network_send_mbps;
    let mut lines = vec![
        Line::styled(
            &app.settings.node_name,
            Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
        ),
        Line::raw(format!("RAM  {used:.1} / {total:.1} GiB")),
        Line::raw(format!("CPU  {:.0}%", sample.cpu_percent)),
        Line::raw(""),
        Line::styled("Network", Style::default().fg(MUTED)),
        Line::raw(format!("{network:.1} Mbps")),
        Line::raw(""),
        Line::styled("Speed", Style::default().fg(MUTED)),
        Line::raw(
            sample
                .generation_tokens_per_second
                .map(|speed| format!("{speed:.1} tok/s"))
                .unwrap_or_else(|| "— tok/s".into()),
        ),
        Line::raw(""),
        Line::styled(
            "EVENT LOG",
            Style::default().fg(MUTED).add_modifier(Modifier::BOLD),
        ),
    ];
    lines.extend(
        app.logs
            .iter()
            .rev()
            .take(6)
            .rev()
            .map(|line| Line::styled(format!("• {line}"), Style::default().fg(MUTED))),
    );
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel("CLUSTER"))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn draw_help(frame: &mut Frame<'_>, area: Rect) {
    frame.render_widget(
        Paragraph::new("↑ ↓ move   ← → tabs   Enter select   Esc back   Space toggle   Q exit")
            .alignment(Alignment::Center)
            .style(Style::default().fg(MUTED))
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(Style::default().fg(PANEL)),
            ),
        area,
    );
}
