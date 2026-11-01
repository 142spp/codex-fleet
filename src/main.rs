mod actions;
mod app;
mod config;
mod fleet;
mod launcher;
mod model;
mod protocol;
mod snapshot;
mod state;
mod ui;

use actions::TaskAction;
use anyhow::{Context, Result, bail};
use app::{App, Dialog, DialogKind};
use clap::Parser;
use config::{Config, Host};
use crossterm::{
    event::{self, Event as Input, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use fleet::{Event, Fleet};
use model::{GitInfo, SessionDetail, Snapshot, Thread, ThreadStatus, now};
use ratatui::{
    Terminal,
    backend::{CrosstermBackend, TestBackend},
};
use std::{
    io::{IsTerminal, Stdout, stdout},
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Parser)]
#[command(
    version,
    about = "A multi-host command center for existing Codex sessions"
)]
struct Args {
    /// Local hidden-task state; defaults to the XDG state directory.
    #[arg(long)]
    state_file: Option<PathBuf>,
    /// TOML configuration; without this flag, connect to the local shared server only.
    #[arg(short, long)]
    config: Option<PathBuf>,
    /// Fetch all hosts once and exit (exit 2 if any host is unavailable).
    #[arg(long)]
    once: bool,
    /// Print the one-shot result as JSON.
    #[arg(long, requires = "once")]
    json: bool,
    /// Show synthetic sample sessions without connecting to any server.
    #[arg(long)]
    demo: bool,
    /// Render one frame to text or a styled SVG (when the path ends in .svg).
    #[arg(long)]
    snapshot: Option<PathBuf>,
    #[arg(long, default_value_t = 110)]
    width: u16,
    #[arg(long, default_value_t = 32)]
    height: u16,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let config = Config::load(args.config.as_deref())?;
    let mut app = App::new(&config);
    if !args.demo {
        let path = args.state_file.unwrap_or_else(state::default_path);
        let state = state::load(&path)?;
        app.hidden = state.hidden;
        app.pinned = state.pinned;
        app.state_path = Some(path);
    }
    if args.demo {
        app.hosts = demo();
        app.reconcile();
        if let Some(key) = app.selected.clone() {
            demo_preview(&mut app, key);
        }
    } else if args.once || args.snapshot.is_some() {
        let results = futures_util::future::join_all(
            config
                .hosts
                .iter()
                .cloned()
                .map(|h| fleet::snapshot_once(h, config.timeout_seconds)),
        )
        .await;
        for s in results {
            app.apply(Event::Snapshot(s));
        }
    }
    if let Some(path) = args.snapshot {
        if !(30..=500).contains(&args.width) || !(8..=200).contains(&args.height) {
            bail!("Snapshot size must be 30..500 by 8..200");
        }
        if !args.demo
            && ui::has_sidebar(args.width, args.height)
            && let Some(key) = app.selected.clone()
            && let Some((h, _)) = app.session(&key)
            && let Ok(mut rpc) = protocol::Rpc::connect(&h.host, config.timeout_seconds).await
        {
            let result = rpc.details(&key.1).await.map_err(|e| format!("{e:#}"));
            app.apply(Event::Details { key, result });
        }
        let mut terminal = Terminal::new(TestBackend::new(args.width, args.height))?;
        terminal.draw(|f| ui::draw(f, &app))?;
        let buf = terminal.backend().buffer();
        if path.extension().is_some_and(|ext| ext == "svg") {
            std::fs::write(path, snapshot::svg(buf))?;
            return Ok(());
        }
        let mut rendered = String::new();
        for y in 0..args.height {
            let mut x = 0;
            while x < args.width {
                rendered.push_str(buf[(x, y)].symbol());
                x += unicode_width::UnicodeWidthStr::width(buf[(x, y)].symbol()).max(1) as u16;
            }
            rendered.push('\n');
        }
        std::fs::write(path, rendered)?;
    } else if args.once {
        if args.json {
            println!("{}", serde_json::to_string_pretty(&app.hosts)?);
        } else {
            for h in &app.hosts {
                println!(
                    "{}: {} · {} sessions",
                    h.host.name,
                    if h.online { "online" } else { "offline" },
                    h.sessions.len()
                );
                if let Some(e) = &h.error {
                    println!("  {e}");
                }
                for t in &h.sessions {
                    println!("  {:<10} {}  {}", t.state().label(), t.id, t.title());
                }
            }
        }
        if app.hosts.iter().any(|h| !h.online) {
            std::process::exit(2);
        }
    } else {
        if !std::io::stdin().is_terminal() || !stdout().is_terminal() {
            bail!("Use an interactive terminal, or pass --once / --snapshot PATH");
        }
        run(app, &config, args.demo).await?;
    }
    Ok(())
}

struct Screen {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    active: bool,
}
impl Screen {
    fn new() -> Result<Self> {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = disable_raw_mode();
            let _ = execute!(stdout(), LeaveAlternateScreen);
            previous(info);
        }));
        let mut screen = Self {
            terminal: Terminal::new(CrosstermBackend::new(stdout()))?,
            active: false,
        };
        screen.enter()?;
        Ok(screen)
    }
    fn enter(&mut self) -> Result<()> {
        enable_raw_mode()?;
        self.active = true;
        execute!(self.terminal.backend_mut(), EnterAlternateScreen)?;
        // Fullscreen redraw must not query cursor position: after Codex exits,
        // a cursor-report request can race with terminal input and time out.
        self.terminal.resize(self.terminal.size()?.into())?;
        Ok(())
    }
    fn leave(&mut self) -> Result<()> {
        if self.active {
            disable_raw_mode()?;
            execute!(self.terminal.backend_mut(), LeaveAlternateScreen)?;
            self.terminal.show_cursor()?;
            self.active = false;
        }
        Ok(())
    }
}
impl Drop for Screen {
    fn drop(&mut self) {
        let _ = self.leave();
    }
}

async fn run(mut app: App, config: &Config, demo: bool) -> Result<()> {
    let mut screen = Screen::new()?;
    let mut fleet = if demo {
        None
    } else {
        Some(Fleet::start(config))
    };
    // Poll on the UI task itself: no background terminal reader survives a handoff.
    let mut tick = tokio::time::interval(Duration::from_millis(25));
    let mut dirty = true;
    let mut rendered_second = 0;
    let mut preview_focus = app.selected.clone();
    let mut preview_since = Instant::now();
    loop {
        if let Some(key) = app.pending_open.take() {
            open_session(&mut app, &mut screen, fleet.as_ref(), &key).await?;
            dirty = true;
        }
        if preview_focus != app.selected {
            preview_focus = app.selected.clone();
            preview_since = Instant::now();
            app.preview_scroll = 0;
        }
        let size = screen.terminal.size()?;
        if ui::has_sidebar(size.width, size.height)
            && !app.searching
            && preview_since.elapsed() >= Duration::from_millis(200)
            && let Some(key) = app.request_preview()
        {
            if demo {
                demo_preview(&mut app, key);
            } else if let Some(f) = &fleet {
                f.details(&key);
            }
            dirty = true;
        }
        if dirty || rendered_second != now() {
            screen.terminal.draw(|f| ui::draw(f, &app))?;
            dirty = false;
            rendered_second = now();
        }
        let event = if event::poll(Duration::ZERO)? {
            Some(event::read()?)
        } else {
            tokio::select! {
                event=async{match fleet.as_mut(){Some(f)=>f.events.recv().await,None=>std::future::pending().await}}=>{if let Some(e)=event{app.apply(e);dirty=true;}None},
                _=tick.tick()=>None,
                _=tokio::signal::ctrl_c()=>break,
            }
        };
        dirty |= event.is_some();
        let Some(Input::Key(key)) = event else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            break;
        }
        if let Some(mut dialog) = app.dialog.take() {
            let deleting = matches!(dialog.kind, DialogKind::Delete { .. });
            match key.code {
                KeyCode::Esc => {}
                KeyCode::Char('n') if deleting => {}
                KeyCode::Enter | KeyCode::Char('y') if deleting => {
                    let DialogKind::Delete { key } = dialog.kind else {
                        unreachable!()
                    };
                    submit_task(
                        &mut app,
                        fleet.as_ref(),
                        demo,
                        &key.0,
                        TaskAction::Delete { id: key.1 },
                    );
                }
                KeyCode::Enter => {
                    let (host, action) = match dialog.kind {
                        DialogKind::New { host } => (host, TaskAction::New { cwd: dialog.value }),
                        DialogKind::Worktree { host, cwd } => (
                            host,
                            TaskAction::Worktree {
                                cwd,
                                branch: dialog.value,
                            },
                        ),
                        DialogKind::Rename { key } => (
                            key.0,
                            TaskAction::Rename {
                                id: key.1,
                                name: dialog.value,
                            },
                        ),
                        DialogKind::Delete { .. } => unreachable!(),
                    };
                    submit_task(&mut app, fleet.as_ref(), demo, &host, action);
                }
                _ => {
                    if !deleting {
                        match key.code {
                            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                dialog.value.clear()
                            }
                            KeyCode::Char(c) => dialog.value.push(c),
                            KeyCode::Backspace => {
                                dialog.value.pop();
                            }
                            _ => {}
                        }
                    }
                    app.dialog = Some(dialog);
                }
            }
            continue;
        }
        if let Some(target) = app.confirm_archive.clone() {
            match key.code {
                KeyCode::Char('y') | KeyCode::Enter => {
                    app.confirm_archive = None;
                    app.begin_archive(target.clone());
                    if let Some(f) = &fleet {
                        f.archive(&target, true);
                    }
                }
                KeyCode::Esc | KeyCode::Char('n') => app.confirm_archive = None,
                _ => {}
            }
            continue;
        }
        if app.help {
            if matches!(key.code, KeyCode::Esc | KeyCode::Char('?')) {
                app.help = false;
            }
            match key.code {
                KeyCode::Down | KeyCode::PageDown => {
                    app.help_scroll = app.help_scroll.saturating_add(3)
                }
                KeyCode::Up | KeyCode::PageUp => {
                    app.help_scroll = app.help_scroll.saturating_sub(3)
                }
                _ => {}
            }
            continue;
        }
        if app.detail.is_some() {
            match key.code {
                KeyCode::Esc | KeyCode::Char('d') => app.detail = None,
                KeyCode::Down | KeyCode::Char('j') => {
                    app.detail_scroll = app.detail_scroll.saturating_add(1)
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    app.detail_scroll = app.detail_scroll.saturating_sub(1)
                }
                KeyCode::PageDown => app.detail_scroll = app.detail_scroll.saturating_add(10),
                KeyCode::PageUp => app.detail_scroll = app.detail_scroll.saturating_sub(10),
                _ => {}
            }
            continue;
        }
        if app.searching {
            match key.code {
                KeyCode::Esc | KeyCode::Enter => app.searching = false,
                KeyCode::Backspace => {
                    app.query.pop();
                }
                KeyCode::Char(c) => app.query.push(c),
                _ => {}
            }
            app.reconcile();
            continue;
        }
        match key.code {
            KeyCode::Char('q') => break,
            KeyCode::Char('?') => {
                app.help = true;
                app.help_scroll = 0;
            }
            KeyCode::Down | KeyCode::Char('j') => app.move_selection(1),
            KeyCode::Up | KeyCode::Char('k') => app.move_selection(-1),
            KeyCode::Home => app.move_selection(-isize::MAX),
            KeyCode::End => app.move_selection(isize::MAX / 2),
            KeyCode::PageDown => app.move_selection(10),
            KeyCode::PageUp => app.move_selection(-10),
            KeyCode::Tab => app.cycle_filter(1),
            KeyCode::BackTab => app.cycle_filter(-1),
            KeyCode::Char('c') => app.cycle_host(),
            KeyCode::Char('h') => {
                if let Err(e) = app.toggle_hidden() {
                    app.message = format!("Hide failed: {e}");
                }
            }
            KeyCode::Char('H') => {
                app.show_hidden = !app.show_hidden;
                app.reconcile();
            }
            KeyCode::Char('g') => app.cycle_group(),
            KeyCode::Char('/') | KeyCode::F(3) => app.searching = true,
            KeyCode::Esc => {
                app.query.clear();
                app.message.clear();
                app.reconcile();
            }
            KeyCode::F(5) => {
                app.message.clear();
                app.preview = None;
                if let Some(f) = &fleet {
                    f.refresh();
                }
            }
            KeyCode::Char('n') => app.open_new(false),
            KeyCode::Char('w') => app.open_new(true),
            KeyCode::Char('r') => {
                if let Some(key) = app.selected.clone()
                    && let Some((_, t)) = app.session(&key)
                {
                    app.dialog = Some(Dialog {
                        kind: DialogKind::Rename { key: key.clone() },
                        value: t.title(),
                    });
                }
            }
            KeyCode::Backspace => {
                if let Some(key) = app.selected.clone() {
                    app.dialog = Some(Dialog {
                        kind: DialogKind::Delete { key },
                        value: String::new(),
                    });
                }
            }
            KeyCode::Char('p') => {
                if let Err(e) = app.toggle_pin() {
                    app.message = format!("Pin failed: {e}");
                }
            }
            KeyCode::Char('f') | KeyCode::Char('x') => {
                if let Some(target) = app.selected.clone() {
                    let action = match key.code {
                        KeyCode::Char('f') => TaskAction::Fork { id: target.1 },
                        KeyCode::Char('x') => TaskAction::Stop { id: target.1 },
                        _ => unreachable!(),
                    };
                    submit_task(&mut app, fleet.as_ref(), demo, &target.0, action);
                }
            }
            KeyCode::Char('[') => app.preview_scroll = app.preview_scroll.saturating_sub(5),
            KeyCode::Char(']') => app.preview_scroll = app.preview_scroll.saturating_add(5),
            KeyCode::Char('a') => {
                if demo {
                    app.message = "Demo only. No real session was archived.".into();
                } else if let Some(target) = app.request_archive()
                    && let Some(f) = &fleet
                {
                    f.archive(&target, false);
                }
            }
            KeyCode::Char('d') => {
                if let Some(k) = app.show_detail() {
                    if let Some(f) = &fleet {
                        f.details(&k);
                    } else if let Some((_, text)) = &mut app.detail {
                        text.push_str("\nDemo: synthetic preview only.");
                    }
                }
            }
            KeyCode::Enter | KeyCode::Char('o') => {
                if demo {
                    app.message = "Demo only. Run without --demo to enter real sessions.".into();
                    continue;
                }
                if let Some(key) = app.selected.clone() {
                    open_session(&mut app, &mut screen, fleet.as_ref(), &key).await?;
                    dirty = true;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn demo() -> Vec<Snapshot> {
    [
        (
            "local",
            "Laptop",
            vec![
                ("Update documentation", "idle"),
                ("Review API client", "active"),
            ],
        ),
        (
            "desktop",
            "Desktop",
            vec![
                ("Run integration tests", "active"),
                ("Review service supervisor", "notLoaded"),
            ],
        ),
        (
            "jetson",
            "Jetson",
            vec![
                ("Validate sensor stream", "active"),
                ("Debug device connection", "notLoaded"),
            ],
        ),
    ]
    .into_iter()
    .map(|(id, name, tasks)| Snapshot {
        host: Host {
            id: id.into(),
            name: name.into(),
            ssh: None,
            socket: None,
            codex: "codex".into(),
        },
        online: true,
        error: None,
        last_seen: Some(now()),
        socket: Some("/demo/control.sock".into()),
        sessions: tasks
            .into_iter()
            .enumerate()
            .map(|(i, (name, kind))| Thread {
                id: format!("{id}-{i}"),
                name: Some(name.into()),
                preview: "Synthetic sample; no actual session is opened.".into(),
                cwd: if i == 0 {
                    "~/projects/example-app".into()
                } else {
                    "~/projects/codex-fleet".into()
                },
                updated_at: now() - i as i64 * 70,
                status: ThreadStatus {
                    kind: kind.into(),
                    active_flags: if id == "desktop" && i == 0 {
                        vec!["waitingOnApproval".into()]
                    } else {
                        vec![]
                    },
                },
                model: Some("gpt-6.1-sol".into()),
                reasoning_effort: Some("high".into()),
                git_info: Some(GitInfo {
                    branch: Some("main".into()),
                }),
                is_pinned: false,
            })
            .collect(),
    })
    .collect()
}

fn submit_task(app: &mut App, fleet: Option<&Fleet>, demo: bool, host: &str, action: TaskAction) {
    if demo {
        app.message = "Demo only. No task was modified.".into();
        return;
    }
    if app.begin_task(host, &action)
        && let Some(fleet) = fleet
    {
        fleet.task(host, action);
    }
}
async fn open_session(
    app: &mut App,
    screen: &mut Screen,
    fleet: Option<&Fleet>,
    key: &app::Key,
) -> Result<()> {
    let (host, socket, cwd) = match app.session(key) {
        Some((h, t)) if h.online => (
            h.host.clone(),
            h.socket.clone().context("Missing server socket")?,
            t.cwd.clone(),
        ),
        _ => {
            app.message = "Device is offline. Press F5 to retry.".into();
            return Ok(());
        }
    };
    screen.leave()?;
    let result = launcher::resume(&host, &socket, &key.1, &cwd).await;
    screen.enter()?;
    app.message = result.err().map(|e| format!("{e:#}")).unwrap_or_default();
    if let Some(f) = fleet {
        f.refresh();
    }
    Ok(())
}

fn demo_preview(app: &mut App, key: app::Key) {
    if let Some((_, thread)) = app.session(&key) {
        let detail=SessionDetail{thread:thread.clone(),body:thread.preview.clone(),last_message:Some("Demo preview. Select a session to inspect its latest response and project metadata.".into()),last_prompt:Some(thread.preview.clone()),history_error:None};
        app.apply(Event::Details {
            key,
            result: Ok(detail),
        });
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn demo_has_three_distinct_hosts_and_all_statuses() {
        let mut app = super::App::new(&super::Config::default());
        app.hosts = super::demo();
        app.reconcile();
        assert_eq!(app.hosts.len(), 3);
        assert_eq!(app.counts(), [6, 1, 2, 1, 2, 0, 0]);
    }
}
