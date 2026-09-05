use super::{App, Screen};
use crate::cluster::NodeRole;
use crate::hardware::bytes_to_gib;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Padding, Paragraph, Tabs, Wrap};

const BLUE: Color = Color::Rgb(59, 130, 246);
const MUTED: Color = Color::Rgb(145, 153, 170);
const BORDER: Color = Color::Rgb(60, 69, 86);

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();
    let lang = app.language;
    if area.width < 60 || area.height < 18 {
        frame.render_widget(
            Paragraph::new(lang.text(
                "BUTION\nEnlarge the terminal.\nMinimum: 60 x 18\nCtrl+Q: quit",
                "BUTION\nУвеличьте окно терминала.\nМинимум: 60 x 18\nCtrl+Q: выход",
            ))
            .wrap(Wrap { trim: false }),
            area,
        );
        return;
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(3),
        ])
        .split(area);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " BUTION",
                Style::default().fg(BLUE).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  v{}", env!("CARGO_PKG_VERSION")),
                Style::default().fg(MUTED),
            ),
        ])),
        rows[0],
    );
    frame.render_widget(
        Tabs::new(Screen::ALL.iter().map(|screen| screen.label(lang)))
            .select(app.screen_index)
            .divider(" ")
            .style(Style::default().fg(MUTED))
            .highlight_style(Style::default().fg(BLUE).add_modifier(Modifier::BOLD)),
        rows[1],
    );
    match app.screen() {
        Screen::Cluster => draw_cluster(frame, rows[2], app),
        Screen::Nodes => draw_nodes(frame, rows[2], app),
        Screen::Models => draw_model(frame, rows[2], app),
        Screen::Benchmark => draw_network(frame, rows[2], app),
        Screen::Chat => draw_chat(frame, rows[2], app),
        Screen::Settings => draw_settings(frame, rows[2], app),
    }
    draw_metrics(frame, rows[3], app);
    frame.render_widget(
        Paragraph::new(help_lines(app))
            .style(Style::default().fg(MUTED))
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(Style::default().fg(BORDER)),
            ),
        rows[4],
    );
    if app.pending_pairing.is_some() {
        draw_pairing(frame, area, app);
    }
}

fn panel(title: &str) -> Block<'_> {
    Block::default()
        .title(format!(" {title} "))
        .title_style(Style::default().fg(BLUE))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER))
        .padding(Padding::horizontal(1))
}

fn field(label: &str, value: impl Into<String>) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<16}"), Style::default().fg(MUTED)),
        Span::raw(value.into()),
    ])
}

fn draw_cluster(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let l = app.language;
    let status = if app.cluster_running {
        l.text("Model process started", "Процесс модели запущен")
    } else if app.settings.role == NodeRole::Worker {
        l.text(
            "Waiting for the main computer",
            "Ожидание основного компьютера",
        )
    } else if app.model.is_some() {
        l.text("Ready to start", "Готов к запуску")
    } else {
        l.text(
            "Select a model to get started",
            "Выберите модель для запуска",
        )
    };
    let mut lines = vec![
        Line::styled(
            status,
            Style::default().fg(BLUE).add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
        field(l.text("Role", "Роль"), l.role(app.settings.role)),
        field(
            l.text("Model", "Модель"),
            app.model
                .as_ref()
                .map(|m| m.name.as_str())
                .unwrap_or(l.text("Not selected", "Не выбрана")),
        ),
        field(
            l.text("AI memory", "Память для ИИ"),
            format!("{:.1} GiB", app.hardware.ai_memory_gib()),
        ),
        field(
            l.text("Backend", "Вычисления"),
            app.hardware.backend.to_string(),
        ),
        field(
            l.text("Other nodes", "Другие узлы"),
            app.nodes
                .iter()
                .filter(|n| n.id != app.settings.node_id)
                .count()
                .to_string(),
        ),
    ];
    if !app.distribution.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            l.text("Distribution", "Распределение"),
            Style::default().fg(MUTED),
        ));
        for (name, fraction) in &app.distribution {
            let name = if name == "Local" {
                l.text("This computer", "Этот компьютер")
            } else {
                name
            };
            lines.push(Line::raw(format!("{name}  {:.0}%", fraction * 100.0)));
        }
    } else if app.model.is_none() && app.settings.role != NodeRole::Worker {
        lines.push(Line::raw(""));
        lines.push(Line::raw(l.text(
            "Restart with the path to your GGUF file:",
            "Перезапустите с путём к файлу GGUF:",
        )));
        lines.push(Line::styled(
            "bution --model \"model.gguf\"",
            Style::default().fg(BLUE),
        ));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel(app.screen().label(l)))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_nodes(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let l = app.language;
    let mut lines = Vec::new();
    for node in &app.nodes {
        let local = node.id == app.settings.node_id;
        lines.push(Line::styled(
            format!(
                "{}{}",
                node.name,
                if local {
                    l.text(" (this computer)", " (этот компьютер)")
                } else {
                    ""
                }
            ),
            Style::default().fg(BLUE),
        ));
        lines.push(Line::raw(format!(
            "{} · {} · {:.1} GiB",
            l.role(node.role),
            l.node_status(node.status),
            bytes_to_gib(node.available_memory_bytes),
        )));
        lines.push(Line::styled(
            node.addresses
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", "),
            Style::default().fg(MUTED),
        ));
        lines.push(Line::raw(""));
    }
    if app.nodes.len() <= 1 {
        lines.push(Line::raw(l.text(
            "Looking for computers on the local network…",
            "Поиск компьютеров в локальной сети…",
        )));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel(app.screen().label(l)))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_model(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let l = app.language;
    let lines = if let Some(model) = &app.model {
        vec![
            Line::styled(&model.name, Style::default().fg(BLUE)),
            Line::raw(""),
            field(
                l.text("File size", "Размер файла"),
                format!("{:.1} GiB", bytes_to_gib(model.file_size_bytes)),
            ),
            field(
                l.text("Estimated RAM", "Оценка памяти"),
                format!("{:.1} GiB", bytes_to_gib(model.estimated_memory_bytes)),
            ),
            Line::raw(""),
            Line::styled(l.text("File", "Файл"), Style::default().fg(MUTED)),
            Line::raw(model.path.display().to_string()),
        ]
    } else {
        vec![
            Line::raw(l.text("No model selected.", "Модель не выбрана.")),
            Line::raw(""),
            Line::raw(l.text(
                "Restart with the path to your GGUF file:",
                "Перезапустите с путём к файлу GGUF:",
            )),
            Line::styled("bution --model \"model.gguf\"", Style::default().fg(BLUE)),
        ]
    };
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel(app.screen().label(l)))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_network(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let l = app.language;
    let lines = if let Some(route) = &app.best_route {
        let stability = match route.benchmark.stability {
            crate::network::Stability::Excellent => l.text("Excellent", "Отличная"),
            crate::network::Stability::Good => l.text("Good", "Хорошая"),
            crate::network::Stability::Unstable => l.text("Unstable", "Нестабильная"),
        };
        vec![
            field(
                l.text("Interface", "Интерфейс"),
                route.interface.name.clone(),
            ),
            field(
                l.text("Peer address", "Адрес узла"),
                route.remote_address.to_string(),
            ),
            Line::raw(""),
            field(
                l.text("Latency", "Задержка"),
                format!("{:.1} ms", route.benchmark.latency.average_ms),
            ),
            field(
                l.text("Bandwidth", "Скорость сети"),
                format!("{:.0} Mbps", route.benchmark.bandwidth.megabits_per_second),
            ),
            field(l.text("Stability", "Стабильность"), stability),
        ]
    } else {
        vec![
            Line::raw(l.text("No network measurements yet.", "Сеть ещё не проверена.")),
            Line::raw(""),
            Line::raw(l.text(
                "The network test starts after pairing.",
                "Проверка сети начнётся после подключения узла.",
            )),
        ]
    };
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel(app.screen().label(l)))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_chat(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let l = app.language;
    let block = panel(app.screen().label(l));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let rows = Layout::vertical([Constraint::Min(1), Constraint::Length(2)]).split(inner);
    let mut lines = Vec::new();
    if app.chat_messages.is_empty() {
        lines.push(Line::raw(if app.cluster_running {
            l.text("Type a message below.", "Введите сообщение внизу.")
        } else {
            l.text(
                "Start a model on the Cluster page first.",
                "Сначала запустите модель на странице «Кластер».",
            )
        }));
    }
    for message in &app.chat_messages {
        let label = match message.role {
            crate::chat::ChatRole::System => l.text("System", "Система"),
            crate::chat::ChatRole::User => l.text("You", "Вы"),
            crate::chat::ChatRole::Assistant => l.text("Assistant", "Ассистент"),
        };
        lines.push(Line::styled(label, Style::default().fg(BLUE)));
        lines.extend(
            message
                .content
                .lines()
                .map(|line| Line::raw(line.to_owned())),
        );
        lines.push(Line::raw(""));
    }
    let history = Paragraph::new(lines).wrap(Wrap { trim: false });
    let offset = history
        .line_count(rows[0].width)
        .saturating_sub(rows[0].height as usize);
    frame.render_widget(
        history.scroll((offset.min(u16::MAX as usize) as u16, 0)),
        rows[0],
    );
    let input = if app.chat_streaming {
        l.text("Generating…", "Генерация…").to_owned()
    } else {
        format!("> {}_", app.chat_input)
    };
    let input = Paragraph::new(input)
        .wrap(Wrap { trim: false })
        .style(Style::default().fg(BLUE));
    let offset = input
        .line_count(rows[1].width)
        .saturating_sub(rows[1].height as usize);
    frame.render_widget(
        input.scroll((offset.min(u16::MAX as usize) as u16, 0)),
        rows[1],
    );
}

fn draw_settings(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let l = app.language;
    let mut lines = vec![
        field(
            l.text("Language", "Язык"),
            l.text("English (system)", "Русский (системный)"),
        ),
        field(l.text("Version", "Версия"), env!("CARGO_PKG_VERSION")),
        field(
            l.text("Control / RPC", "Управление / RPC"),
            format!("{} / {}", app.settings.control_port, app.settings.rpc_port),
        ),
    ];
    if let Some(version) = &app.update_available {
        lines.push(field(l.text("Update", "Обновление"), version.clone()));
        lines.push(Line::raw("bution --update"));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        l.text("Recent events", "Последние события"),
        Style::default().fg(MUTED),
    ));
    lines.extend(
        app.logs
            .iter()
            .rev()
            .take(3)
            .rev()
            .map(|line| Line::raw(line.clone())),
    );
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel(app.screen().label(l)))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_metrics(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let sample = &app.telemetry;
    let speed = sample
        .generation_tokens_per_second
        .map(|speed| format!("{speed:.1} {}", app.language.text("tok/s", "ток/с")))
        .unwrap_or_else(|| "—".into());
    frame.render_widget(
        Paragraph::new(format!(
            " RAM {:.1}/{:.1} GiB   CPU {:.0}%   {}",
            bytes_to_gib(sample.memory_used_bytes),
            bytes_to_gib(sample.memory_total_bytes),
            sample.cpu_percent,
            speed,
        ))
        .style(Style::default().fg(MUTED)),
        area,
    );
}

fn help_lines(app: &App) -> Vec<Line<'static>> {
    let l = app.language;
    if app.pending_pairing.is_some() {
        return vec![Line::raw(l.text(
            " ←/→ choice   Enter confirm   Esc reject",
            " ←/→ выбор   Enter подтвердить   Esc отклонить",
        ))];
    }
    let mut actions = Vec::new();
    match app.screen() {
        Screen::Cluster => {
            if app.cluster_running {
                actions.push(l.text("Enter stop", "Enter остановить"));
            } else {
                actions.push(l.text("Space role", "Space роль"));
                if app.model.is_some() && app.settings.role != NodeRole::Worker {
                    actions.push(l.text("Enter start", "Enter запустить"));
                }
            }
        }
        Screen::Chat => {
            if !app.chat_streaming {
                if app.cluster_running {
                    actions.push(l.text("Enter send", "Enter отправить"));
                }
                actions.push(l.text("Ctrl+N clear chat", "Ctrl+N очистить чат"));
            } else {
                actions.push(l.text("Generating…", "Генерация…"));
            }
        }
        _ => {}
    }
    let navigation = if app.screen() == Screen::Chat {
        l.text(
            " Tab/Shift+Tab pages   Ctrl+Q quit",
            " Tab/Shift+Tab страницы   Ctrl+Q выход",
        )
    } else {
        l.text(
            " Tab/Shift+Tab pages   Q quit",
            " Tab/Shift+Tab страницы   Q выход",
        )
    };
    vec![
        Line::raw(format!(" {}", actions.join("   "))),
        Line::raw(navigation),
    ]
}

fn draw_pairing(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let Some(pairing) = &app.pending_pairing else {
        return;
    };
    let l = app.language;
    let width = 56.min(area.width.saturating_sub(2));
    let height = 12.min(area.height.saturating_sub(2));
    let popup = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, popup);
    let accept = l.text("Accept", "Принять");
    let reject = l.text("Reject", "Отклонить");
    let buttons = if pairing.accept_selected {
        format!("[ {accept} ]     {reject}")
    } else {
        format!("{accept}     [ {reject} ]")
    };
    let lines = vec![
        Line::raw(l.text("Connect this computer?", "Подключить этот компьютер?")),
        Line::raw(""),
        Line::raw(pairing.name.clone()),
        Line::styled(pairing.address.to_string(), Style::default().fg(MUTED)),
        Line::raw(""),
        Line::raw(format!("{}: {}", l.text("Code", "Код"), pairing.code)),
        Line::raw(""),
        Line::styled(buttons, Style::default().fg(BLUE)),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel(l.text("Pairing", "Подключение")))
            .wrap(Wrap { trim: false }),
        popup,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locale::Language;
    use ratatui::{Terminal, backend::TestBackend};

    fn render(app: &App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn help(app: &App) -> String {
        help_lines(app)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn hints_match_page_and_available_actions() {
        let mut app = super::super::app::tests::test_app();
        for index in 0..Screen::ALL.len() {
            app.screen_index = index;
            assert_eq!(
                help(&app).contains("Space role"),
                app.screen() == Screen::Cluster
            );
            assert!(!help(&app).contains("Enter start"));
            assert!(!help(&app).contains("Enter send"));
        }
        app.screen_index = 0;
        app.cluster_running = true;
        assert!(help(&app).contains("Enter stop"));
        assert!(!help(&app).contains("Space"));
        app.screen_index = 4;
        assert!(help(&app).contains("Enter send"));
        app.chat_streaming = true;
        assert!(!help(&app).contains("Enter send"));
        assert!(!help(&app).contains("Ctrl+N"));
    }

    #[test]
    fn pages_and_hints_fit_in_small_terminals_in_both_languages() {
        let mut app = super::super::app::tests::test_app();
        for language in [Language::English, Language::Russian] {
            app.language = language;
            for (width, height) in [(80, 24), (60, 18), (120, 40)] {
                for index in 0..Screen::ALL.len() {
                    app.screen_index = index;
                    let rendered = render(&app, width, height);
                    assert!(rendered.contains(app.screen().label(language)));
                    assert!(rendered.contains(language.text("quit", "выход")));
                    for hint in help_lines(&app) {
                        assert!(hint.width() <= width as usize, "{hint}");
                    }
                    if language == Language::English {
                        assert!(
                            !rendered
                                .chars()
                                .any(|ch| ('\u{0400}'..='\u{04ff}').contains(&ch))
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn long_chat_keeps_latest_response_and_input_visible() {
        let mut app = super::super::app::tests::test_app();
        app.screen_index = 4;
        app.cluster_running = true;
        app.chat_messages.push(crate::chat::ChatMessage {
            role: crate::chat::ChatRole::Assistant,
            content: format!("{}\nLATEST_RESPONSE", "long response ".repeat(150)),
        });
        app.chat_input = "next question".into();
        let rendered = render(&app, 80, 24);
        assert!(rendered.contains("LATEST_RESPONSE"), "{rendered}");
        assert!(rendered.contains("next question"));
    }

    #[test]
    fn pairing_replaces_page_hints() {
        let mut app = super::super::app::tests::test_app();
        let (response, _) = tokio::sync::oneshot::channel();
        app.apply_runtime_event(crate::runtime::RuntimeEvent::PairingRequested {
            name: "Windows-PC".into(),
            address: "192.168.1.2:31750".parse().unwrap(),
            code: "123456".into(),
            response,
        });
        let rendered = render(&app, 80, 24);
        assert!(rendered.contains("Accept"));
        assert!(help(&app).contains("Esc reject"));
        assert!(!help(&app).contains("Space role"));
    }

    #[test]
    fn tiny_terminal_shows_resize_instruction() {
        let app = super::super::app::tests::test_app();
        assert!(render(&app, 40, 12).contains("60 x 18"));
    }

    #[test]
    fn cluster_preview() {
        let mut app = super::super::app::tests::test_app();
        for language in [Language::English, Language::Russian] {
            app.language = language;
            let rendered = render(&app, 80, 24);
            assert!(rendered.contains("bution --model"));
            assert!(rendered.contains(language.text("Space role", "Space роль")));
            println!("\n{rendered}");
        }
    }
}
