use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};
#[derive(Default, Serialize, Deserialize)]
pub struct UiState {
    #[serde(default)]
    pub hidden: HashSet<(String, String)>,
    #[serde(default)]
    pub pinned: HashSet<(String, String)>,
}
pub fn default_path() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state")
        })
        .join("codex-fleet/state.json")
}
pub fn load(path: &Path) -> Result<UiState> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes).context("Invalid Fleet state file")?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(UiState::default()),
        Err(e) => Err(e.into()),
    }
}
pub fn save(path: &Path, state: &UiState) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::write(&temporary, serde_json::to_vec_pretty(state)?)?;
    std::fs::rename(temporary, path)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn hidden_state_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut state = super::UiState::default();
        state.hidden.insert(("host".into(), "thread".into()));
        state.pinned.insert(("other-host".into(), "thread".into()));
        super::save(&path, &state).unwrap();
        assert_eq!(super::load(&path).unwrap().hidden, state.hidden);
        assert_eq!(super::load(&path).unwrap().pinned, state.pinned);
    }
}
