use crate::{
    actions::{Outcome, TaskAction},
    config::Config,
    fleet::Event,
    model::{SessionDetail, Snapshot, State, Thread, age},
};
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

pub type Key = (String, String);
#[derive(Clone)]
pub enum DialogKind {
    New { host: String },
    Worktree { host: String, cwd: String },
    Rename { key: Key },
    Delete { key: Key },
}
pub struct Dialog {
    pub kind: DialogKind,
    pub value: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Project,
    Host,
    None,
}
impl Group {
    pub fn label(self) -> &'static str {
        match self {
            Self::Project => "Project",
            Self::Host => "Device",
            Self::None => "None",
        }
    }
}
pub const FILTERS: [Option<State>; 7] = [
    None,
    Some(State::NeedsYou),
    Some(State::Working),
    Some(State::Ready),
    Some(State::Saved),
    Some(State::Error),
    Some(State::Unknown),
];
#[derive(Clone)]
pub enum Row {
    Heading(String, usize),
    Folder(String, usize),
    Session(Key),
}
pub struct App {
    pub dialog: Option<Dialog>,
    pub task_busy: bool,
    pub pending_open: Option<Key>,
    pub hidden: HashSet<Key>,
    pub pinned: HashSet<Key>,
    pub show_hidden: bool,
    pub state_path: Option<PathBuf>,
    pub hosts: Vec<Snapshot>,
    pub selected: Option<Key>,
    pub filter: usize,
    pub host_filter: Option<String>,
    pub group: Group,
    pub query: String,
    pub searching: bool,
    pub help: bool,
    pub help_scroll: u16,
    pub detail: Option<(Key, String)>,
    pub detail_scroll: u16,
    pub message: String,
    pub archiving: HashSet<Key>,
    pub confirm_archive: Option<Key>,
    pub preview: Option<(Key, Result<SessionDetail, String>)>,
    pub preview_pending: HashSet<Key>,
    pub preview_scroll: u16,
    pub preview_revision: Option<i64>,
}
impl App {
    pub fn new(config: &Config) -> Self {
        Self {
            dialog: None,
            task_busy: false,
            pending_open: None,
            hidden: HashSet::new(),
            pinned: HashSet::new(),
            show_hidden: false,
            state_path: None,
            hosts: config
                .hosts
                .iter()
                .cloned()
                .map(Snapshot::pending)
                .collect(),
            selected: None,
            filter: 0,
            host_filter: None,
            group: Group::Host,
            query: String::new(),
            searching: false,
            help: false,
            help_scroll: 0,
            detail: None,
            detail_scroll: 0,
            message: String::new(),
            archiving: HashSet::new(),
            confirm_archive: None,
            preview: None,
            preview_pending: HashSet::new(),
            preview_scroll: 0,
            preview_revision: None,
        }
    }
    pub fn apply(&mut self, event: Event) {
        match event {
            Event::TaskCompleted {
                host,
                target,
                label,
                result,
            } => {
                self.task_busy = false;
                match result {
                    Err(e) => self.message = format!("{label} failed: {e}"),
                    Ok(outcome) => {
                        self.message = format!("{label} completed");
                        match outcome {
                            Outcome::Created(thread) => {
                                let thread = *thread;
                                let key = (host.clone(), thread.id.clone());
                                if let Some(h) = self.hosts.iter_mut().find(|h| h.host.id == host) {
                                    h.sessions.retain(|t| t.id != thread.id);
                                    h.sessions.push(thread);
                                }
                                self.show_hidden = false;
                                self.query.clear();
                                self.filter = 0;
                                self.host_filter = None;
                                self.selected = Some(key.clone());
                                self.pending_open = Some(key);
                            }
                            Outcome::Deleted => {
                                if let Some(id) = target
                                    && let Some(h) =
                                        self.hosts.iter_mut().find(|h| h.host.id == host)
                                {
                                    h.sessions.retain(|t| t.id != id);
                                }
                            }
                            Outcome::Renamed(name) => {
                                if let Some(id) = target
                                    && let Some(h) =
                                        self.hosts.iter_mut().find(|h| h.host.id == host)
                                    && let Some(t) = h.sessions.iter_mut().find(|t| t.id == id)
                                {
                                    t.name = Some(name);
                                }
                            }
                            Outcome::Stopped => {
                                self.message = "Stop requested".into();
                            }
                        }
                        self.preview = None;
                    }
                }
            }
            Event::Archived { key, result } => {
                self.archiving.remove(&key);
                match result {
                    Ok(()) => {
                        let keys = self.keys();
                        let index = keys.iter().position(|k| k == &key).unwrap_or(0);
                        let was_selected = self.selected.as_ref() == Some(&key);
                        if let Some(h) = self.hosts.iter_mut().find(|h| h.host.id == key.0) {
                            h.sessions.retain(|t| t.id != key.1);
                        }
                        if was_selected {
                            let keys = self.keys();
                            self.selected =
                                keys.get(index.min(keys.len().saturating_sub(1))).cloned();
                        }
                        self.message =
                            "Archived. Restore later with codex unarchive on that device.".into();
                    }
                    Err(e) => self.message = format!("Archive failed: {e}"),
                }
            }
            Event::Snapshot(mut snapshot) => {
                for t in &mut snapshot.sessions {
                    t.is_pinned = self
                        .pinned
                        .contains(&(snapshot.host.id.clone(), t.id.clone()));
                }
                if let Some(h) = self
                    .hosts
                    .iter_mut()
                    .find(|h| h.host.id == snapshot.host.id)
                {
                    *h = snapshot;
                }
            }
            Event::Details { key, result } => {
                self.preview_pending.remove(&key);
                if self.detail.as_ref().is_some_and(|(k, _)| k == &key) {
                    self.detail = Some((
                        key.clone(),
                        result
                            .as_ref()
                            .map(|d| d.body.clone())
                            .unwrap_or_else(|e| e.clone()),
                    ));
                }
                if self.selected.as_ref() == Some(&key) {
                    self.preview_revision = self.session(&key).map(|(_, t)| t.updated_at);
                    self.preview = Some((key, result));
                }
            }
        }
        self.reconcile();
    }
    pub fn session(&self, key: &Key) -> Option<(&Snapshot, &Thread)> {
        let h = self.hosts.iter().find(|h| h.host.id == key.0)?;
        Some((h, h.sessions.iter().find(|t| t.id == key.1)?))
    }
    fn matches(&self, h: &Snapshot, t: &Thread, filter: bool) -> bool {
        if self.hidden.contains(&(h.host.id.clone(), t.id.clone())) != self.show_hidden {
            return false;
        }
        if self.host_filter.as_ref().is_some_and(|id| id != &h.host.id) {
            return false;
        }
        if filter && FILTERS[self.filter].is_some_and(|s| !h.online || t.state() != s) {
            return false;
        }
        let q = self.query.to_lowercase();
        q.is_empty()
            || format!(
                "{} {} {} {} {}",
                t.title(),
                t.cwd,
                h.host.name,
                h.host.id,
                t.id
            )
            .to_lowercase()
            .contains(&q)
    }
    pub fn counts(&self) -> [usize; 7] {
        let mut counts = [0; 7];
        for h in &self.hosts {
            for t in &h.sessions {
                if self.matches(h, t, false) {
                    counts[0] += 1;
                    if h.online {
                        for i in 1..7 {
                            if FILTERS[i] == Some(t.state()) {
                                counts[i] += 1;
                            }
                        }
                    }
                }
            }
        }
        counts
    }
    pub fn rows(&self) -> Vec<Row> {
        if self.group == Group::Host {
            let mut rows = vec![];
            for h in &self.hosts {
                if self.host_filter.as_ref().is_some_and(|id| id != &h.host.id) {
                    continue;
                }
                let mut sessions: Vec<_> = h
                    .sessions
                    .iter()
                    .filter(|t| self.matches(h, t, true))
                    .collect();
                sessions.sort_by(|a, b| {
                    b.is_pinned
                        .cmp(&a.is_pinned)
                        .then(b.updated_at.cmp(&a.updated_at))
                        .then(a.id.cmp(&b.id))
                });
                if sessions.is_empty() && (!self.query.is_empty() || self.filter != 0) {
                    continue;
                }
                let label = if h.online {
                    h.host.name.clone()
                } else {
                    format!("{} · offline", h.host.name)
                };
                rows.push(Row::Heading(label, sessions.len()));
                let mut folders: BTreeMap<&str, Vec<&Thread>> = BTreeMap::new();
                for session in sessions {
                    let folder = if session.cwd.is_empty() {
                        "(no project)"
                    } else {
                        &session.cwd
                    };
                    folders.entry(folder).or_default().push(session);
                }
                for (folder, sessions) in folders {
                    rows.push(Row::Folder(folder.into(), sessions.len()));
                    rows.extend(
                        sessions
                            .into_iter()
                            .map(|t| Row::Session((h.host.id.clone(), t.id.clone()))),
                    );
                }
            }
            return rows;
        }
        let mut groups: BTreeMap<String, Vec<(&Snapshot, &Thread)>> = BTreeMap::new();
        for h in &self.hosts {
            for t in &h.sessions {
                if self.matches(h, t, true) {
                    let group = match self.group {
                        Group::Project => {
                            if t.cwd.is_empty() {
                                "(no project)".into()
                            } else {
                                t.cwd.clone()
                            }
                        }
                        Group::Host => h.host.name.clone(),
                        Group::None => String::new(),
                    };
                    groups.entry(group).or_default().push((h, t));
                }
            }
        }
        let mut rows = vec![];
        for (group, mut sessions) in groups {
            sessions.sort_by(|(ha, a), (hb, b)| {
                b.is_pinned.cmp(&a.is_pinned).then(
                    b.updated_at
                        .cmp(&a.updated_at)
                        .then(ha.host.id.cmp(&hb.host.id))
                        .then(a.id.cmp(&b.id)),
                )
            });
            if self.group != Group::None {
                rows.push(Row::Heading(group, sessions.len()));
            }
            rows.extend(
                sessions
                    .into_iter()
                    .map(|(h, t)| Row::Session((h.host.id.clone(), t.id.clone()))),
            );
        }
        rows
    }
    pub fn keys(&self) -> Vec<Key> {
        self.rows()
            .into_iter()
            .filter_map(|r| match r {
                Row::Session(k) => Some(k),
                _ => None,
            })
            .collect()
    }
    pub fn reconcile(&mut self) {
        let keys = self.keys();
        if self.selected.as_ref().is_none_or(|k| !keys.contains(k)) {
            self.selected = keys.first().cloned();
        }
    }
    pub fn move_selection(&mut self, delta: isize) {
        let keys = self.keys();
        if keys.is_empty() {
            self.selected = None;
            return;
        }
        let index = self
            .selected
            .as_ref()
            .and_then(|k| keys.iter().position(|x| x == k))
            .unwrap_or(0);
        self.selected =
            Some(keys[(index as isize + delta).clamp(0, keys.len() as isize - 1) as usize].clone());
    }
    pub fn cycle_host(&mut self) {
        let index = self
            .host_filter
            .as_ref()
            .and_then(|id| self.hosts.iter().position(|h| &h.host.id == id));
        self.host_filter = match index {
            None => self.hosts.first().map(|h| h.host.id.clone()),
            Some(i) => self.hosts.get(i + 1).map(|h| h.host.id.clone()),
        };
        self.reconcile();
    }
    pub fn cycle_group(&mut self) {
        self.group = match self.group {
            Group::Host => Group::Project,
            Group::Project => Group::None,
            Group::None => Group::Host,
        };
        self.reconcile();
    }
    pub fn cycle_filter(&mut self, delta: isize) {
        self.filter = (self.filter as isize + delta).rem_euclid(FILTERS.len() as isize) as usize;
        self.reconcile();
    }
    pub fn show_detail(&mut self) -> Option<Key> {
        let key = self.selected.clone()?;
        let (h, t) = self.session(&key)?;
        let summary = format!(
            "{}\nHost: {}\nProject: {}\nState: {}{}\nUpdated: {} ago\nID: {}\n\n{}\n\nLoading recent conversation…",
            t.title(),
            h.host.name,
            t.cwd,
            t.state().label(),
            if h.online {
                ""
            } else {
                " (cached; host offline)"
            },
            age(t.updated_at),
            t.id,
            t.preview
        );
        let body = self
            .preview
            .as_ref()
            .filter(|(k, _)| k == &key)
            .map(|(_, r)| {
                r.as_ref()
                    .map(|d| d.body.clone())
                    .unwrap_or_else(|e| e.clone())
            })
            .unwrap_or(summary);
        self.detail = Some((key.clone(), body));
        self.detail_scroll = 0;
        self.preview_pending.insert(key.clone());
        Some(key)
    }
    pub fn request_preview(&mut self) -> Option<Key> {
        let key = self.selected.clone()?;
        let (host, thread) = self.session(&key)?;
        if !host.online || self.preview_pending.contains(&key) {
            return None;
        }
        if self.preview.as_ref().is_some_and(|(k, _)| k == &key)
            && self.preview_revision == Some(thread.updated_at)
        {
            return None;
        }
        self.preview_pending.insert(key.clone());
        Some(key)
    }
    pub fn request_archive(&mut self) -> Option<Key> {
        let key = self.selected.clone()?;
        if self.archiving.contains(&key) {
            return None;
        }
        let (h, t) = self.session(&key)?;
        if !h.online {
            self.message = "Device is offline. Reconnect before archiving.".into();
            return None;
        }
        if matches!(t.state(), State::Working | State::NeedsYou | State::Unknown) {
            self.confirm_archive = Some(key);
            return None;
        }
        self.begin_archive(key.clone());
        Some(key)
    }
    pub fn begin_archive(&mut self, key: Key) {
        self.archiving.insert(key);
        self.message = "Archiving…".into();
    }
    pub fn begin_task(&mut self, host: &str, action: &TaskAction) -> bool {
        if self.task_busy {
            self.message = "Waiting for the previous task action…".into();
            return false;
        }
        if !self.hosts.iter().any(|h| h.host.id == host && h.online) {
            self.message = "Device is offline".into();
            return false;
        }
        self.message = format!("{}…", action.label());
        self.task_busy = true;
        true
    }
    pub fn open_new(&mut self, worktree: bool) {
        let selected = self.selected.as_ref().and_then(|key| self.session(key));
        let Some(host) = selected
            .map(|(h, _)| h)
            .or_else(|| {
                self.host_filter
                    .as_ref()
                    .and_then(|id| self.hosts.iter().find(|h| &h.host.id == id))
            })
            .or_else(|| self.hosts.first())
        else {
            return;
        };
        let cwd = selected
            .map(|(_, t)| t.cwd.clone())
            .unwrap_or_else(|| ".".into());
        let kind = if worktree {
            DialogKind::Worktree {
                host: host.host.id.clone(),
                cwd: cwd.clone(),
            }
        } else {
            DialogKind::New {
                host: host.host.id.clone(),
            }
        };
        let value = if worktree {
            format!("codex-fleet/task-{}", crate::model::now())
        } else {
            cwd
        };
        self.dialog = Some(Dialog { kind, value });
    }
    pub fn toggle_hidden(&mut self) -> anyhow::Result<()> {
        let Some(key) = self.selected.clone() else {
            return Ok(());
        };
        let mut hidden = self.hidden.clone();
        let hiding = hidden.insert(key.clone());
        if !hiding {
            hidden.remove(&key);
        }
        if let Some(path) = &self.state_path {
            crate::state::save(
                path,
                &crate::state::UiState {
                    hidden: hidden.clone(),
                    pinned: self.pinned.clone(),
                },
            )?;
        }
        self.hidden = hidden;
        self.message = if hiding {
            "Hidden locally. H shows hidden tasks; h restores them."
        } else {
            "Task restored to the normal list."
        }
        .into();
        self.reconcile();
        Ok(())
    }
    pub fn toggle_pin(&mut self) -> anyhow::Result<()> {
        let Some(key) = self.selected.clone() else {
            return Ok(());
        };
        let mut pinned = self.pinned.clone();
        let pinning = pinned.insert(key.clone());
        if !pinning {
            pinned.remove(&key);
        }
        if let Some(path) = &self.state_path {
            crate::state::save(
                path,
                &crate::state::UiState {
                    hidden: self.hidden.clone(),
                    pinned: pinned.clone(),
                },
            )?;
        }
        self.pinned = pinned;
        if let Some(h) = self.hosts.iter_mut().find(|h| h.host.id == key.0)
            && let Some(t) = h.sessions.iter_mut().find(|t| t.id == key.1)
        {
            t.is_pinned = pinning;
        }
        self.message = if pinning {
            "Pinned locally"
        } else {
            "Unpinned locally"
        }
        .into();
        self.reconcile();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_pins_and_hidden_tasks_are_host_scoped_and_survive_snapshots() {
        let mut app = sample();
        let key = app.selected.clone().unwrap();
        app.toggle_pin().unwrap();
        assert!(app.pinned.contains(&key));
        let mut snapshot = app.hosts[0].clone();
        snapshot.sessions[0].is_pinned = false;
        app.apply(Event::Snapshot(snapshot));
        assert!(app.session(&key).unwrap().1.is_pinned);
        assert!(!app.hosts[1].sessions[0].is_pinned);
        app.toggle_hidden().unwrap();
        assert_eq!(app.keys().len(), 1);
        app.show_hidden = true;
        app.reconcile();
        assert_eq!(app.selected, Some(key));
        app.toggle_hidden().unwrap();
        app.show_hidden = false;
        app.reconcile();
        assert_eq!(app.keys().len(), 2);
    }
    #[test]
    fn previews_ignore_stale_selection_and_refresh_after_snapshot_changes() {
        let mut app = sample();
        let first = app.request_preview().unwrap();
        assert!(app.request_preview().is_none());
        let thread = app.session(&first).unwrap().1.clone();
        let detail = SessionDetail {
            thread,
            body: "history".into(),
            last_message: Some("latest".into()),
            last_prompt: None,
            history_error: None,
        };
        app.move_selection(1);
        app.apply(Event::Details {
            key: first,
            result: Ok(detail),
        });
        assert!(app.preview.is_none());
        let second = app.request_preview().unwrap();
        app.apply(Event::Details {
            key: second,
            result: Err("offline".into()),
        });
        assert!(app.request_preview().is_none());
        app.hosts[1].sessions[0].updated_at += 1;
        assert!(app.request_preview().is_some());
    }
    #[test]
    fn archive_failure_keeps_session_and_success_only_removes_owning_host() {
        let mut app = sample();
        for h in &mut app.hosts {
            h.sessions[0].status.kind = "idle".into();
        }
        let key = app.selected.clone().unwrap();
        assert_eq!(app.request_archive(), Some(key.clone()));
        assert!(app.request_archive().is_none());
        app.apply(Event::Archived {
            key: key.clone(),
            result: Err("denied".into()),
        });
        assert_eq!(app.keys().len(), 2);
        assert!(!app.archiving.contains(&key));
        app.apply(Event::Archived {
            key: key.clone(),
            result: Ok(()),
        });
        assert_eq!(app.keys().len(), 1);
        assert_ne!(app.selected.as_ref(), Some(&key));
    }
    #[test]
    fn active_archive_needs_confirmation_and_offline_is_blocked() {
        let mut app = sample();
        assert!(app.request_archive().is_none());
        assert_eq!(app.confirm_archive, app.selected);
        assert!(app.archiving.is_empty());
        app.confirm_archive = None;
        for h in &mut app.hosts {
            h.online = false;
        }
        assert!(app.request_archive().is_none());
        assert!(app.confirm_archive.is_none());
    }
    #[test]
    fn device_sections_preserve_config_order_even_with_equal_display_names() {
        let mut app = sample();
        app.hosts[1].host.name = app.hosts[0].host.name.clone();
        assert_eq!(app.group, Group::Host);
        let rows = app.rows();
        assert_eq!(
            rows.iter()
                .filter(|r| matches!(r, Row::Heading(_, _)))
                .count(),
            2
        );
        assert_eq!(app.keys()[0].0, "local");
        assert_eq!(app.keys()[1].0, "remote");
    }
    #[test]
    fn folders_are_scoped_to_device_and_empty_folders_disappear() {
        let mut app = sample();
        let mut extra = app.hosts[0].sessions[0].clone();
        extra.id = "extra".into();
        extra.cwd = "/another".into();
        app.hosts[0].sessions.push(extra);
        let folders: Vec<_> = app
            .rows()
            .into_iter()
            .filter_map(|row| match row {
                Row::Folder(name, count) => Some((name, count)),
                _ => None,
            })
            .collect();
        assert_eq!(
            folders,
            vec![
                ("/another".into(), 1),
                ("/project".into(), 1),
                ("/project".into(), 1)
            ]
        );
        app.apply(Event::Archived {
            key: ("local".into(), "extra".into()),
            result: Ok(()),
        });
        assert!(
            !app.rows()
                .iter()
                .any(|row| matches!(row,Row::Folder(name,_) if name=="/another"))
        );
        app.query = "unmatched".into();
        assert!(app.rows().is_empty());
    }
    fn sample() -> App {
        let mut c = Config::default();
        let mut second = c.hosts[0].clone();
        second.id = "remote".into();
        second.name = "Remote".into();
        c.hosts.push(second);
        let mut app = App::new(&c);
        for h in &mut app.hosts {
            h.online = true;
            h.sessions = vec![Thread {
                id: "same-id".into(),
                name: Some("한글 작업".into()),
                cwd: "/project".into(),
                status: crate::model::ThreadStatus {
                    kind: "active".into(),
                    active_flags: vec![],
                },
                ..Default::default()
            }];
        }
        app.reconcile();
        app
    }
    #[test]
    fn host_identity_selection_and_offline_counts() {
        let mut app = sample();
        assert_eq!(app.keys().len(), 2);
        app.move_selection(1);
        let key = app.selected.clone();
        app.hosts.reverse();
        app.reconcile();
        assert_eq!(app.selected, key);
        app.hosts[0].online = false;
        assert_eq!(app.counts()[0], 2);
        assert_eq!(app.counts()[2], 1);
        app.filter = 2;
        assert_eq!(app.keys().len(), 1);
    }
    #[test]
    fn korean_search_and_filters() {
        let mut app = sample();
        app.query = "한글".into();
        assert_eq!(app.keys().len(), 2);
        app.cycle_host();
        assert_eq!(app.keys().len(), 1);
        app.query = "absent".into();
        app.reconcile();
        assert!(app.selected.is_none());
    }
}
