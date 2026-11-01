use crate::{
    app::{App, DialogKind, FILTERS, Group, Row},
    model::{State, age, plain_text},
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, Borders, Cell, Clear, List, ListItem, ListState, Padding, Paragraph,
        Row as TableRow, Table, TableState, Wrap,
    },
};

fn color(state: State) -> Color {
    match state {
        State::NeedsYou => Color::Yellow,
        State::Working => Color::Cyan,
        State::Ready => Color::Green,
        State::Saved => Color::Gray,
        State::Error => Color::Red,
        State::Unknown => Color::Magenta,
    }
}
pub fn has_sidebar(width: u16, height: u16) -> bool {
    width >= 110 && height >= 14
}
fn preview_lines(app: &App) -> Vec<Line<'static>> {
    let Some(key) = &app.selected else {
        return vec![Line::from("Select a session to view details.")];
    };
    let Some((host, t)) = app.session(key) else {
        return vec![];
    };
    let result = app
        .preview
        .as_ref()
        .filter(|(k, _)| k == key)
        .map(|(_, r)| r);
    let detail = result.and_then(|r| r.as_ref().ok());
    let metadata = detail.map(|d| &d.thread).unwrap_or(t);
    let mut lines = vec![
        Line::default(),
        Line::styled(
            t.title(),
            Style::default()
                .fg(color(t.state()))
                .add_modifier(Modifier::BOLD),
        ),
        Line::from(format!(
            "{} {}",
            t.state().symbol(),
            if host.online {
                t.state().label()
            } else {
                "Offline (cached)"
            }
        )),
        Line::default(),
    ];
    let model = metadata
        .model
        .as_ref()
        .or(t.model.as_ref())
        .map_or("—", String::as_str);
    let reasoning = metadata
        .reasoning_effort
        .as_ref()
        .or(t.reasoning_effort.as_ref())
        .map_or("—", String::as_str);
    let branch = metadata
        .git_info
        .as_ref()
        .and_then(|g| g.branch.as_ref())
        .or_else(|| t.git_info.as_ref().and_then(|g| g.branch.as_ref()))
        .map_or("—", String::as_str);
    for (label, value) in [
        ("Device", host.host.name.as_str()),
        ("Model", model),
        ("Reasoning", reasoning),
        ("Project", t.cwd.as_str()),
        ("Branch", branch),
    ] {
        lines.push(Line::styled(
            label.to_string(),
            Style::default().fg(Color::DarkGray),
        ));
        lines.push(Line::from(plain_text(value)));
        lines.push(Line::default());
    }
    lines.push(Line::styled(
        "Last message",
        Style::default().fg(Color::DarkGray),
    ));
    let message = if let Some(d) = detail {
        d.history_error
            .as_deref()
            .or(d.last_message.as_deref())
            .unwrap_or("No assistant message yet.")
    } else if let Some(Err(e)) = result {
        e.as_str()
    } else if !host.online {
        "Device offline. Reconnect to load the last message."
    } else {
        "Loading recent message…"
    };
    lines.extend(
        plain_text(message)
            .lines()
            .map(|line| Line::from(line.to_string())),
    );
    lines.push(Line::default());
    lines.push(Line::styled("Prompt", Style::default().fg(Color::DarkGray)));
    let prompt = detail
        .and_then(|d| d.last_prompt.as_deref())
        .unwrap_or(&t.preview);
    lines.extend(
        plain_text(prompt)
            .lines()
            .map(|line| Line::from(line.to_string())),
    );
    lines
}
pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    if area.width < 30 || area.height < 8 {
        frame.render_widget(Paragraph::new("Resize terminal to at least 30×8"), area);
        return;
    }
    let parts = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(2),
        Constraint::Length(1),
    ])
    .split(area);
    let online = app.hosts.iter().filter(|h| h.online).count();
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " Codex Fleet",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(
                    "  Group: {}    Connected {}/{}",
                    app.group.label(),
                    online,
                    app.hosts.len()
                ),
                Style::default().fg(Color::DarkGray),
            ),
        ])),
        parts[0],
    );
    let counts = app.counts();
    let mut spans = vec![];
    for (i, filter) in FILTERS.iter().enumerate() {
        let style = if i == app.filter {
            Style::default().bg(Color::DarkGray).fg(Color::White)
        } else {
            Style::default().fg(Color::Gray)
        };
        spans.push(Span::styled(
            format!(" {} {} ", filter.map_or("All", State::label), counts[i]),
            style,
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), parts[1]);
    let host_name = app
        .host_filter
        .as_ref()
        .and_then(|id| app.hosts.iter().find(|h| &h.host.id == id))
        .map_or("All", |h| h.host.name.as_str());
    let search = if app.searching {
        format!("/{}▏", app.query)
    } else if app.query.is_empty() {
        "/ search".into()
    } else {
        format!("/{}", app.query)
    };
    frame.render_widget(
        Paragraph::new(format!(
            " Device: {host_name}    {search}{}",
            if app.show_hidden {
                "    Hidden tasks"
            } else {
                ""
            }
        ))
        .block(Block::default().borders(Borders::BOTTOM)),
        parts[2],
    );
    let wide = has_sidebar(area.width, area.height);
    let (listing, sidebar) = if wide {
        let columns = Layout::horizontal([Constraint::Percentage(64), Constraint::Percentage(36)])
            .split(parts[3]);
        (columns[0], Some(columns[1]))
    } else {
        (parts[3], None)
    };
    let rows = app.rows();
    let items: Vec<Text> = rows
        .iter()
        .map(|row| match row {
            Row::Heading(name, count) => Text::from(vec![
                Line::default(),
                Line::from(Span::styled(
                    format!("{}  {count}", plain_text(name)),
                    Style::default().fg(Color::White),
                )),
            ])
            .style(Style::default().add_modifier(Modifier::BOLD)),
            Row::Folder(name, count) => Text::from(Line::from(Span::styled(
                format!("  {}  {count}", plain_text(name)),
                Style::default().fg(Color::DarkGray),
            ))),
            Row::Session(key) => {
                let (h, t) = app.session(key).expect("row exists");
                let state = if h.online {
                    t.state().label()
                } else {
                    "Offline"
                };
                let style = Style::default().fg(if h.online {
                    color(t.state())
                } else {
                    Color::DarkGray
                });
                let symbol = if app.archiving.contains(key) {
                    "…"
                } else if h.online {
                    t.state().symbol()
                } else {
                    "×"
                };
                let prefix = if app.group == Group::Host {
                    format!("    {symbol} ")
                } else {
                    format!("{symbol} [{}] ", h.host.name)
                };
                let mut spans = vec![Span::styled(prefix, style)];
                if t.is_pinned {
                    spans.push(Span::styled("★ ", Style::default().fg(Color::Yellow)));
                }
                spans.push(Span::styled(t.title(), style));
                if listing.width < 68 && area.width >= 100 {
                    spans.push(Span::styled(
                        format!("   {state} · {}", age(t.updated_at)),
                        Style::default().fg(Color::DarkGray),
                    ));
                }
                Text::from(Line::from(spans))
            }
        })
        .collect();
    if items.is_empty() {
        frame.render_widget(
            Paragraph::new(
                " No matching sessions. Check host status below; F5 retries connections.",
            )
            .style(Style::default().fg(Color::DarkGray))
            .wrap(Wrap { trim: false }),
            listing,
        );
    } else {
        let selection = app.selected.as_ref().and_then(|key| {
            rows.iter()
                .position(|r| matches!(r,Row::Session(k) if k==key))
        });
        if listing.width >= 68 {
            let table_rows = rows.iter().zip(items).map(|(row, item)| {
                let height = if matches!(row, Row::Heading(_, _)) {
                    2
                } else {
                    1
                };
                let (status, updated) = match row {
                    Row::Session(key) => {
                        let (host, t) = app.session(key).expect("session row");
                        (
                            if app.archiving.contains(key) {
                                "Archiving"
                            } else if host.online {
                                t.state().label()
                            } else {
                                "Offline"
                            }
                            .to_string(),
                            format!("{} ago", age(t.updated_at)),
                        )
                    }
                    _ => (String::new(), String::new()),
                };
                TableRow::new(vec![
                    Cell::from(item),
                    Cell::from(status),
                    Cell::from(updated).style(Style::default().fg(Color::DarkGray)),
                ])
                .height(height)
            });
            let table = Table::new(
                table_rows,
                [
                    Constraint::Min(15),
                    Constraint::Length(11),
                    Constraint::Length(10),
                ],
            )
            .column_spacing(2)
            .header(
                TableRow::new(["Tasks", "Status", "Updated"])
                    .style(Style::default().fg(Color::DarkGray)),
            )
            .highlight_symbol("› ")
            .row_highlight_style(Style::default().bg(Color::Blue).fg(Color::White));
            frame.render_stateful_widget(
                table,
                listing,
                &mut TableState::default().with_selected(selection),
            );
        } else {
            let mut state = ListState::default().with_selected(selection);
            frame.render_stateful_widget(
                List::new(items.into_iter().map(ListItem::new))
                    .highlight_symbol("› ")
                    .highlight_style(Style::default().bg(Color::Blue).fg(Color::White)),
                listing,
                &mut state,
            );
        }
    }
    if let Some(sidebar) = sidebar {
        frame.render_widget(
            Paragraph::new(preview_lines(app))
                .block(
                    Block::default()
                        .borders(Borders::LEFT)
                        .padding(Padding::horizontal(1))
                        .title(" Task details · [ ] scroll "),
                )
                .wrap(Wrap { trim: false })
                .scroll((app.preview_scroll, 0)),
            sidebar,
        );
    }
    let health = app
        .hosts
        .iter()
        .map(|h| {
            let s = if h.online {
                "online".into()
            } else if let Some(e) = &h.error {
                format!("offline: {e}")
            } else {
                "connecting".into()
            };
            let last = if !h.online {
                h.last_seen
                    .map(|t| format!(" (last seen {} ago)", age(t)))
                    .unwrap_or_default()
            } else {
                String::new()
            };
            format!("{}: {s}{last}", h.host.name)
        })
        .collect::<Vec<_>>()
        .join("  |  ");
    let message = if app.message.is_empty() {
        health
    } else {
        app.message.clone()
    };
    frame.render_widget(
        Paragraph::new(plain_text(&message))
            .style(Style::default().fg(Color::DarkGray))
            .wrap(Wrap { trim: false }),
        parts[4],
    );
    frame.render_widget(
        Paragraph::new(
            "? help  ↑/↓ move  enter open  n new  a archive  F3 search  c device  F5 refresh  q quit",
        ),
        parts[5],
    );
    if let Some(dialog) = &app.dialog {
        let (title, explanation) = match &dialog.kind {
            DialogKind::New { host } => ("New task", format!("Device: {host}\nProject directory:")),
            DialogKind::Worktree { host, cwd } => (
                "New worktree",
                format!(
                    "Device: {host}\nFrom: {cwd}\nNew branch (committed HEAD; existing changes stay in place):"
                ),
            ),
            DialogKind::Rename { .. } => ("Rename task", "New task name:".into()),
            DialogKind::Delete { key } => (
                "Permanently delete task?",
                format!(
                    "{}\n\nThis removes its transcript and spawned descendants.\nThis cannot be undone.\n\ny / Enter: delete     n / Esc: cancel",
                    app.session(key)
                        .map(|(_, t)| t.title())
                        .unwrap_or_else(|| key.1.clone())
                ),
            ),
        };
        let area = popup(area, 90, 13);
        frame.render_widget(Clear, area);
        let content = if matches!(dialog.kind, DialogKind::Delete { .. }) {
            explanation
        } else {
            format!(
                "{explanation}\n\n{}▏\n\nEnter: submit    Ctrl-U: clear    Esc: cancel",
                plain_text(&dialog.value)
            )
        };
        frame.render_widget(
            Paragraph::new(content)
                .block(Block::bordered().title(format!(" {title} ")))
                .wrap(Wrap { trim: false }),
            area,
        );
    } else if let Some(key) = &app.confirm_archive {
        let name = app
            .session(key)
            .map(|(_, t)| t.title())
            .unwrap_or_else(|| key.1.clone());
        let area = popup(area, 76, 8);
        frame.render_widget(Clear, area);
        frame.render_widget(Paragraph::new(format!("{}\n\nThis session may be running. Archiving can stop its work.\n\ny / Enter: archive    n / Esc: cancel",plain_text(&name))).block(Block::bordered().title(" Archive session? ")).wrap(Wrap{trim:false}),area);
    } else if app.help {
        draw_help(
            frame,
            Rect::new(
                parts[3].x,
                parts[3].y,
                parts[3].width,
                parts[3].height + parts[4].height,
            ),
            app.help_scroll,
        );
        frame.render_widget(Clear, parts[5]);
        frame.render_widget(
            Paragraph::new("esc back    ↑/↓ scroll on narrow screens"),
            parts[5],
        );
    } else if let Some((_, body)) = &app.detail {
        let area = popup(area, 110, 35);
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(plain_text(body))
                .block(Block::bordered().title(" Recent conversation · ↑/↓ scroll · Esc close "))
                .wrap(Wrap { trim: false })
                .scroll((app.detail_scroll, 0)),
            area,
        );
    }
}
fn popup(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width.saturating_sub(2));
    let h = height.min(area.height.saturating_sub(2));
    Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    )
}
fn draw_help(frame: &mut Frame, area: Rect, scroll: u16) {
    frame.render_widget(Clear, area);
    let groups = [
        (
            "Navigate",
            vec![
                ("↑ / k", "Up"),
                ("↓ / j", "Down"),
                ("enter", "Open"),
                ("pgup", "Page up"),
                ("pgdn", "Page down"),
                ("^c / q", "Quit"),
            ],
        ),
        (
            "Tasks",
            vec![
                ("n", "New"),
                ("w", "New worktree"),
                ("o", "Resume"),
                ("f", "Fork"),
                ("r", "Rename"),
                ("x", "Stop"),
                ("a", "Archive"),
                ("h", "Hide / unhide"),
                ("backspace", "Delete"),
                ("p", "Pin / unpin"),
            ],
        ),
        (
            "View",
            vec![
                ("tab/shift+tab", "Filter"),
                ("f3 / /", "Search"),
                ("g", "Group"),
                ("c", "Device filter"),
                ("H", "Hidden tasks"),
                ("F5", "Refresh"),
                ("d", "Full history"),
                ("[ / ]", "Scroll details"),
            ],
        ),
    ];
    let sections: Vec<Text> = groups
        .into_iter()
        .map(|(name, keys)| {
            let mut lines = vec![Line::styled(
                name,
                Style::default().add_modifier(Modifier::BOLD),
            )];
            lines.extend(keys.into_iter().map(|(key, action)| {
                Line::from(vec![
                    Span::styled(format!("{key:<15}"), Style::default().fg(Color::LightBlue)),
                    Span::raw(action),
                ])
            }));
            Text::from(lines)
        })
        .collect();
    frame.render_widget(
        Paragraph::new("Task shortcuts").style(Style::default().add_modifier(Modifier::BOLD)),
        Rect::new(area.x, area.y, area.width, 1),
    );
    let body = Rect::new(
        area.x,
        area.y + 2,
        area.width,
        area.height.saturating_sub(2),
    );
    if area.width >= 90 {
        let columns = Layout::horizontal([
            Constraint::Percentage(26),
            Constraint::Percentage(34),
            Constraint::Percentage(40),
        ])
        .split(body);
        for (text, column) in sections.into_iter().zip(columns.iter()) {
            frame.render_widget(Paragraph::new(text).scroll((scroll, 0)), *column);
        }
    } else {
        let mut lines = vec![];
        for text in sections {
            lines.extend(text.lines);
            lines.push(Line::default());
        }
        frame.render_widget(
            Paragraph::new(lines)
                .scroll((scroll, 0))
                .wrap(Wrap { trim: false }),
            body,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn responsive_columns_and_preview_use_available_width() {
        let config = crate::config::Config::default();
        let mut app = App::new(&config);
        app.hosts[0].online = true;
        let thread = crate::model::Thread {
            id: "a".into(),
            name: Some("alpha".into()),
            cwd: "/project".into(),
            model: Some("fixture-model".into()),
            status: crate::model::ThreadStatus {
                kind: "idle".into(),
                active_flags: vec![],
            },
            ..Default::default()
        };
        app.hosts[0].sessions = vec![
            thread.clone(),
            crate::model::Thread {
                id: "b".into(),
                name: Some("beta-long-title".into()),
                ..thread.clone()
            },
        ];
        app.reconcile();
        app.apply(crate::fleet::Event::Details {
            key: app.selected.clone().unwrap(),
            result: Ok(crate::model::SessionDetail {
                thread,
                body: "history".into(),
                last_message: Some("latest reply".into()),
                last_prompt: Some("last question".into()),
                history_error: None,
            }),
        });
        for width in [60, 90, 140] {
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, 40)).unwrap();
            terminal.draw(|f| draw(f, &app)).unwrap();
            let lines: Vec<String> = (0..40)
                .map(|y| {
                    (0..width)
                        .map(|x| terminal.backend().buffer()[(x, y)].symbol())
                        .collect()
                })
                .collect();
            let text = lines.join("\n");
            assert_eq!(text.contains("Task details"), width >= 110);
            if width >= 110 {
                assert!(text.contains("fixture-model"));
                assert!(text.contains("latest reply"));
                assert!(text.contains("last question"));
            }
            if width >= 68 {
                let a = lines
                    .iter()
                    .find(|s| s.split('│').next().unwrap().contains("alpha"))
                    .unwrap();
                let b = lines
                    .iter()
                    .find(|s| s.split('│').next().unwrap().contains("beta-long-title"))
                    .unwrap();
                // Compare terminal columns, not byte offsets (the selection glyph is UTF-8).
                assert_eq!(
                    unicode_width::UnicodeWidthStr::width(&a[..a.find("Ready").unwrap()]),
                    unicode_width::UnicodeWidthStr::width(&b[..b.find("Ready").unwrap()]),
                );
            }
        }
    }
    #[test]
    fn small_and_large_terminal_render_without_panics() {
        let app = App::new(&crate::config::Config::default());
        for (w, h) in [(20, 5), (40, 12), (110, 35)] {
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
            terminal.draw(|f| draw(f, &app)).unwrap();
        }
    }
}
