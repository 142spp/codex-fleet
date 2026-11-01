use crate::{config::Host, model::Thread, protocol::program_command};
use anyhow::{Context, Result, bail};
use std::{
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug)]
pub enum TaskAction {
    New { cwd: String },
    Worktree { cwd: String, branch: String },
    Fork { id: String },
    Rename { id: String, name: String },
    Stop { id: String },
    Delete { id: String },
}
impl TaskAction {
    pub fn id(&self) -> Option<&str> {
        match self {
            Self::New { .. } | Self::Worktree { .. } => None,
            Self::Fork { id }
            | Self::Rename { id, .. }
            | Self::Stop { id }
            | Self::Delete { id } => Some(id),
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::New { .. } => "New task",
            Self::Worktree { .. } => "New worktree",
            Self::Fork { .. } => "Fork",
            Self::Rename { .. } => "Rename",
            Self::Stop { .. } => "Stop",
            Self::Delete { .. } => "Delete",
        }
    }
}
#[derive(Debug)]
pub enum Outcome {
    Created(Box<Thread>),
    Renamed(String),
    Deleted,
    Stopped,
}

async fn git(host: &Host, args: Vec<String>) -> Result<String> {
    let output = tokio::time::timeout(
        Duration::from_secs(60),
        program_command(host, "git", &args, false)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .context("Git command timed out")??;
    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(String::from_utf8(output.stdout)?.trim().into())
}
pub async fn create_worktree(host: &Host, cwd: &str, branch: &str) -> Result<String> {
    if branch.trim().is_empty() || branch.starts_with('-') || branch.contains(['\n', '\r']) {
        bail!("Enter a valid branch name");
    }
    git(
        host,
        vec!["check-ref-format".into(), "--branch".into(), branch.into()],
    )
    .await?;
    let root = git(
        host,
        vec![
            "-C".into(),
            cwd.into(),
            "rev-parse".into(),
            "--show-toplevel".into(),
        ],
    )
    .await?;
    let path = Path::new(&root);
    let name = path
        .file_name()
        .context("Repository has no directory name")?
        .to_string_lossy();
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let target = path
        .parent()
        .context("Repository has no parent")?
        .join(".codex-fleet-worktrees")
        .join(format!("{name}-{stamp}"));
    let target = target
        .to_str()
        .context("Non-UTF-8 worktree path")?
        .to_string();
    git(
        host,
        vec![
            "-C".into(),
            root.clone(),
            "worktree".into(),
            "add".into(),
            "-b".into(),
            branch.into(),
            target.clone(),
            "HEAD".into(),
        ],
    )
    .await?;
    let relative = Path::new(cwd).strip_prefix(&root).unwrap_or(Path::new(""));
    Ok(Path::new(&target).join(relative).to_string_lossy().into())
}
