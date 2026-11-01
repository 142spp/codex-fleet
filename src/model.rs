use crate::config::Host;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub preview: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub status: ThreadStatus,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub git_info: Option<GitInfo>,
    #[serde(default)]
    pub is_pinned: bool,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct GitInfo {
    pub branch: Option<String>,
}
#[derive(Clone, Debug)]
pub struct SessionDetail {
    pub thread: Thread,
    pub body: String,
    pub last_message: Option<String>,
    pub last_prompt: Option<String>,
    pub history_error: Option<String>,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadStatus {
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub active_flags: Vec<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    NeedsYou,
    Working,
    Ready,
    Saved,
    Error,
    Unknown,
}
impl State {
    pub fn label(self) -> &'static str {
        match self {
            Self::NeedsYou => "Needs you",
            Self::Working => "Working",
            Self::Ready => "Ready",
            Self::Saved => "Saved",
            Self::Error => "Error",
            Self::Unknown => "Unknown",
        }
    }
    pub fn symbol(self) -> &'static str {
        match self {
            Self::NeedsYou => "!",
            Self::Working => "●",
            Self::Ready => "○",
            Self::Saved => "·",
            Self::Error => "×",
            Self::Unknown => "?",
        }
    }
}
impl Thread {
    pub fn title(&self) -> String {
        let s = self
            .name
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(&self.preview);
        s.lines()
            .next()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("Untitled")
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect()
    }
    pub fn state(&self) -> State {
        match self.status.kind.as_str() {
            "active"
                if self
                    .status
                    .active_flags
                    .iter()
                    .any(|f| matches!(f.as_str(), "waitingOnApproval" | "waitingOnUserInput")) =>
            {
                State::NeedsYou
            }
            "active" => State::Working,
            "idle" => State::Ready,
            "notLoaded" => State::Saved,
            "systemError" => State::Error,
            _ => State::Unknown,
        }
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    pub host: Host,
    pub online: bool,
    pub error: Option<String>,
    pub last_seen: Option<i64>,
    pub socket: Option<String>,
    pub sessions: Vec<Thread>,
}
impl Snapshot {
    pub fn pending(host: Host) -> Self {
        Self {
            host,
            online: false,
            error: None,
            last_seen: None,
            socket: None,
            sessions: vec![],
        }
    }
}
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
/// Preserve line breaks but never emit terminal control bytes from transcript data.
pub fn plain_text(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_control() && c != '\n' && c != '\t' {
                '�'
            } else {
                c
            }
        })
        .collect()
}
pub fn age(timestamp: i64) -> String {
    let n = (now() - timestamp).max(0);
    if n < 60 {
        format!("{n}s")
    } else if n < 3600 {
        format!("{}m", n / 60)
    } else if n < 86400 {
        format!("{}h", n / 3600)
    } else {
        format!("{}d", n / 86400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn status_does_not_conflate_saved_with_done() {
        let t: Thread =
            serde_json::from_value(serde_json::json!({"id":"x","status":{"type":"notLoaded"}}))
                .unwrap();
        assert_eq!(t.state(), State::Saved);
        for flag in ["waitingOnApproval", "waitingOnUserInput"] {
            let mut t = t.clone();
            t.status.kind = "active".into();
            t.status.active_flags = vec![flag.into()];
            assert_eq!(t.state(), State::NeedsYou);
        }
    }
    #[test]
    fn unknown_status_is_visible() {
        let t: Thread =
            serde_json::from_value(serde_json::json!({"id":"x","status":{"type":"futureState"}}))
                .unwrap();
        assert_eq!(t.state(), State::Unknown);
    }
}
