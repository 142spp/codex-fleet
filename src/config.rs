use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, path::Path};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default = "default_poll")]
    pub poll_seconds: u64,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
    pub hosts: Vec<Host>,
}
fn default_poll() -> u64 {
    5
}
fn default_timeout() -> u64 {
    15
}
fn default_codex() -> String {
    "codex".into()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Host {
    pub id: String,
    pub name: String,
    pub ssh: Option<String>,
    pub socket: Option<String>,
    #[serde(default = "default_codex")]
    pub codex: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            poll_seconds: default_poll(),
            timeout_seconds: default_timeout(),
            hosts: vec![Host {
                id: "local".into(),
                name: "Laptop".into(),
                ssh: None,
                socket: None,
                codex: default_codex(),
            }],
        }
    }
}
impl Config {
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let config: Self = if let Some(path) = path {
            toml::from_str(
                &std::fs::read_to_string(path)
                    .with_context(|| format!("Read {}", path.display()))?,
            )?
        } else {
            Self::default()
        };
        config.validate()?;
        Ok(config)
    }
    pub fn validate(&self) -> Result<()> {
        if self.hosts.is_empty() {
            bail!("At least one host is required");
        }
        if !(1..=3600).contains(&self.poll_seconds) || !(1..=300).contains(&self.timeout_seconds) {
            bail!("poll_seconds must be 1..3600; timeout_seconds must be 1..300");
        }
        let mut ids = HashSet::new();
        for h in &self.hosts {
            if h.id.is_empty() || h.name.is_empty() || h.codex.is_empty() || !ids.insert(&h.id) {
                bail!("Host IDs must be nonempty and unique; name and codex must be nonempty");
            }
            if let Some(ssh) = &h.ssh
                && (ssh.is_empty() || ssh.starts_with('-') || ssh.chars().any(char::is_whitespace))
            {
                bail!("Invalid SSH alias for {}", h.id);
            }
            if h.socket.as_ref().is_some_and(|s| !s.starts_with('/')) {
                bail!("socket must be an absolute path on host {}", h.id);
            }
        }
        Ok(())
    }
}

/// Quote one argument for the POSIX shell used by OpenSSH on the remote host.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

pub fn remote_command(program: &str, args: &[String]) -> String {
    std::iter::once(program)
        .chain(args.iter().map(String::as_str))
        .map(shell_quote)
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_duplicate_and_option_hosts() {
        let mut c = Config::default();
        c.hosts.push(c.hosts[0].clone());
        assert!(c.validate().is_err());
        c.hosts.pop();
        c.hosts[0].ssh = Some("-oProxyCommand=bad".into());
        assert!(c.validate().is_err());
    }
    #[test]
    fn shell_arguments_cannot_inject_commands() {
        let input = "a'b;$(touch /tmp/no)\n한글";
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("printf %s {}", shell_quote(input)))
            .output()
            .unwrap();
        assert_eq!(String::from_utf8(output.stdout).unwrap(), input);
    }
}
