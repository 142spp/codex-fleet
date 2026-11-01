use crate::{config::Host, protocol::host_command};
use anyhow::{Context, Result, bail};
use std::process::Stdio;

pub fn resume_args(socket: &str, thread: &str, cwd: &str) -> Vec<String> {
    let mut args = vec![
        "resume".into(),
        "--remote".into(),
        format!("unix://{socket}"),
    ];
    if !cwd.is_empty() {
        args.extend(["--cd".into(), cwd.into()]);
    }
    args.extend(["--".into(), thread.into()]);
    args
}
pub async fn resume(host: &Host, socket: &str, thread: &str, cwd: &str) -> Result<()> {
    let status = host_command(host, &resume_args(socket, thread, cwd), true)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .await
        .context("Could not launch Codex")?;
    if !status.success() {
        bail!("Codex exited with {status}. Existing session was not replaced or forked.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicitly_reuses_selected_server_without_prompt() {
        let args = resume_args(
            "/home/test user/control.sock",
            "thread-id",
            "/project with spaces",
        );
        assert_eq!(
            args,
            vec![
                "resume",
                "--remote",
                "unix:///home/test user/control.sock",
                "--cd",
                "/project with spaces",
                "--",
                "thread-id"
            ]
        );
    }
}
