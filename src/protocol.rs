use crate::{
    config::{Host, remote_command},
    model::{SessionDetail, Thread},
};
use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashSet},
    pin::Pin,
    process::Stdio,
    task::{Context as TaskContext, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::UnixStream,
    process::{Child, ChildStdin, ChildStdout, Command},
    time::timeout,
};
use tokio_tungstenite::{WebSocketStream, client_async, tungstenite::Message};

pub trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}
struct ChildIo {
    input: ChildStdin,
    output: ChildStdout,
}
impl AsyncRead for ChildIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.output).poll_read(cx, buf)
    }
}
impl AsyncWrite for ChildIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.input).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.input).poll_flush(cx)
    }
    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.input).poll_shutdown(cx)
    }
}

pub fn host_command(host: &Host, args: &[String], interactive: bool) -> Command {
    program_command(host, &host.codex, args, interactive)
}
pub fn program_command(host: &Host, program: &str, args: &[String], interactive: bool) -> Command {
    if let Some(alias) = &host.ssh {
        let mut c = Command::new("ssh");
        c.args([
            if interactive { "-t" } else { "-T" },
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=7",
            "-o",
            "ServerAliveInterval=10",
            "-o",
            "ServerAliveCountMax=2",
            "--",
            alias,
        ]);
        c.arg(format!("exec {}", remote_command(program, args)));
        c
    } else {
        let mut c = Command::new(program);
        c.args(args);
        c
    }
}

pub struct Rpc {
    ws: WebSocketStream<Box<dyn Io>>,
    _child: Option<Child>,
    next_id: u64,
    timeout: Duration,
    pub socket: String,
}
impl Rpc {
    pub async fn connect(host: &Host, seconds: u64) -> Result<Self> {
        let duration = Duration::from_secs(seconds);
        timeout(duration, Self::connect_inner(host, duration))
            .await
            .context("Connection timed out")?
    }
    async fn connect_inner(host: &Host, duration: Duration) -> Result<Self> {
        let socket = if let Some(path) = &host.socket {
            path.clone()
        } else {
            let output = host_command(
                host,
                &["app-server".into(), "daemon".into(), "version".into()],
                false,
            )
            .kill_on_drop(true)
            .output()
            .await
            .context("Could not run Codex; check executable path and SSH")?;
            if !output.status.success() {
                bail!(
                    "Codex discovery failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                );
            }
            let value: Value = serde_json::from_slice(&output.stdout)
                .context("Invalid daemon version response")?;
            if value["status"] != "running" {
                bail!("Shared Codex server is not running (start it separately)");
            }
            value["socketPath"]
                .as_str()
                .context("Daemon did not report socketPath; configure socket explicitly")?
                .to_string()
        };
        let (stream, child): (Box<dyn Io>, _) = if host.ssh.is_some() {
            let mut child = host_command(
                host,
                &[
                    "app-server".into(),
                    "proxy".into(),
                    "--sock".into(),
                    socket.clone(),
                ],
                false,
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()?;
            let stream = ChildIo {
                input: child.stdin.take().context("Missing proxy stdin")?,
                output: child.stdout.take().context("Missing proxy stdout")?,
            };
            (Box::new(stream), Some(child))
        } else {
            (
                Box::new(
                    UnixStream::connect(&socket)
                        .await
                        .with_context(|| format!("Connect to {socket}"))?,
                ),
                None,
            )
        };
        Self::from_stream(stream, child, socket, duration).await
    }
    async fn from_stream(
        stream: Box<dyn Io>,
        child: Option<Child>,
        socket: String,
        duration: Duration,
    ) -> Result<Self> {
        let (ws, _) = client_async("ws://localhost/rpc", stream)
            .await
            .context("WebSocket handshake with shared server failed")?;
        let mut rpc = Self {
            ws,
            _child: child,
            next_id: 1,
            timeout: duration,
            socket,
        };
        rpc.request("initialize", json!({"clientInfo":{"name":"codex_fleet","title":"Codex Fleet","version":env!("CARGO_PKG_VERSION")}})).await?;
        rpc.ws
            .send(Message::Text(
                json!({"method":"initialized","params":{}})
                    .to_string()
                    .into(),
            ))
            .await?;
        Ok(rpc)
    }
    pub async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let duration = self.timeout;
        timeout(duration, self.request_inner(method, params))
            .await
            .with_context(|| format!("{method} timed out"))?
    }
    async fn request_inner(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        self.ws
            .send(Message::Text(
                json!({"id":id,"method":method,"params":params})
                    .to_string()
                    .into(),
            ))
            .await?;
        while let Some(frame) = self.ws.next().await {
            match frame? {
                Message::Text(s) => {
                    let v: Value = serde_json::from_str(&s)?;
                    if v["id"] == id {
                        if let Some(error) = v.get("error") {
                            bail!("{method}: {error}");
                        }
                        return v.get("result").cloned().context("Missing JSON-RPC result");
                    }
                    // This monitor never approves requests or resumes threads while browsing.
                    if v.get("method").is_some() && v.get("id").is_some() {
                        self.ws.send(Message::Text(json!({"id":v["id"],"error":{"code":-32601,"message":"Read-only monitor"}}).to_string().into())).await?;
                    }
                }
                Message::Ping(p) => self.ws.send(Message::Pong(p)).await?,
                Message::Close(_) => bail!("Server closed the connection"),
                _ => {}
            }
        }
        bail!("Server disconnected")
    }
    pub async fn threads(&mut self) -> Result<Vec<Thread>> {
        let mut cursor = Value::Null;
        let mut seen = HashSet::new();
        let mut all = BTreeMap::new();
        loop {
            let result = self.request("thread/list", json!({"limit":100,"cursor":cursor,"sortKey":"updated_at","sortDirection":"desc"})).await?;
            let data: Vec<Thread> = serde_json::from_value(
                result
                    .get("data")
                    .cloned()
                    .context("Missing thread list data")?,
            )?;
            for t in data {
                all.insert(t.id.clone(), t);
            }
            cursor = result.get("nextCursor").cloned().unwrap_or(Value::Null);
            if cursor.is_null() {
                break;
            }
            if !seen.insert(cursor.to_string()) {
                bail!("Server repeated a pagination cursor");
            }
        }
        // Newly opened chats may be loaded but not yet present in persisted history.
        // Merge them without resuming or claiming a writer for any thread.
        let mut cursor = Value::Null;
        let mut seen = HashSet::new();
        loop {
            let loaded = self
                .request("thread/loaded/list", json!({"cursor":cursor,"limit":100}))
                .await?;
            if let Some(ids) = loaded["data"].as_array() {
                for id in ids.iter().filter_map(Value::as_str) {
                    if !all.contains_key(id) {
                        let value = self.request("thread/read", json!({"threadId":id})).await?;
                        let t: Thread = serde_json::from_value(value["thread"].clone())?;
                        all.insert(t.id.clone(), t);
                    }
                }
            }
            cursor = loaded.get("nextCursor").cloned().unwrap_or(Value::Null);
            if cursor.is_null() {
                break;
            }
            if !seen.insert(cursor.to_string()) {
                bail!("Server repeated a loaded-thread cursor");
            }
        }
        let mut all: Vec<_> = all.into_values().collect();
        all.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then(a.id.cmp(&b.id)));
        Ok(all)
    }
    pub async fn archive(&mut self, id: &str, allow_active: bool) -> Result<()> {
        let metadata = self.request("thread/read", json!({"threadId":id})).await?;
        let kind = metadata["thread"]["status"]["type"]
            .as_str()
            .unwrap_or("unknown");
        if !allow_active && !matches!(kind, "idle" | "notLoaded" | "systemError") {
            bail!("Session is now active or has unknown status. Refresh and press a to confirm.");
        }
        self.request("thread/archive", json!({"threadId":id}))
            .await?;
        Ok(())
    }
    pub async fn start(&mut self, cwd: &str) -> Result<Thread> {
        let result = self.request("thread/start", json!({"cwd":cwd})).await?;
        Ok(serde_json::from_value(result["thread"].clone())?)
    }
    pub async fn fork(&mut self, id: &str) -> Result<Thread> {
        let result = self
            .request("thread/fork", json!({"threadId":id,"excludeTurns":true}))
            .await?;
        Ok(serde_json::from_value(result["thread"].clone())?)
    }
    pub async fn rename(&mut self, id: &str, name: &str) -> Result<()> {
        if name.trim().is_empty() {
            bail!("Name cannot be empty");
        }
        self.request("thread/name/set", json!({"threadId":id,"name":name}))
            .await?;
        Ok(())
    }
    pub async fn delete(&mut self, id: &str) -> Result<()> {
        let metadata = self.request("thread/read", json!({"threadId":id})).await?;
        if metadata["thread"]["status"]["type"] == "active" {
            bail!("Stop the running task with x before deleting it");
        }
        self.request("thread/delete", json!({"threadId":id}))
            .await?;
        Ok(())
    }
    pub async fn stop(&mut self, id: &str) -> Result<()> {
        let turns = self
            .request(
                "thread/turns/list",
                json!({"threadId":id,"limit":20,"sortDirection":"desc","itemsView":"notLoaded"}),
            )
            .await?;
        let turn = turns["data"]
            .as_array()
            .and_then(|turns| turns.iter().find(|t| t["status"] == "inProgress"))
            .and_then(|t| t["id"].as_str());
        if let Some(turn) = turn {
            self.request("turn/interrupt", json!({"threadId":id,"turnId":turn}))
                .await?;
        } else {
            bail!("No running turn on this task");
        }
        Ok(())
    }
    pub async fn details(&mut self, id: &str) -> Result<SessionDetail> {
        let metadata = self.request("thread/read", json!({"threadId":id})).await?;
        let t: Thread = serde_json::from_value(metadata["thread"].clone())?;
        let mut text = format!(
            "{}\n{}\n{} · {}\n\n",
            t.title(),
            t.cwd,
            t.state().label(),
            t.id
        );
        let mut last_message = None;
        let mut last_prompt = None;
        let mut history_error = None;
        match self
            .request(
                "thread/turns/list",
                json!({"threadId":id,"limit":6,"sortDirection":"desc","itemsView":"full"}),
            )
            .await
        {
            Ok(turns) => {
                if let Some(turns) = turns["data"].as_array() {
                    for turn in turns {
                        if let Some(items) = turn["items"].as_array() {
                            for item in items.iter().rev() {
                                if last_message.is_none() && item["type"] == "agentMessage" {
                                    last_message = message_text(item);
                                }
                                if last_prompt.is_none() && item["type"] == "userMessage" {
                                    last_prompt = message_text(item);
                                }
                            }
                        }
                    }
                    for turn in turns.iter().rev() {
                        text.push_str(&format!(
                            "── {} ──\n",
                            turn["status"].as_str().unwrap_or("Turn")
                        ));
                        if let Some(items) = turn["items"].as_array() {
                            for item in items {
                                render_item(item, &mut text);
                            }
                        }
                    }
                }
            }
            Err(e) => {
                let error = format!("History unavailable on this server: {e}");
                text.push_str(&error);
                history_error = Some(error);
            }
        }
        Ok(SessionDetail {
            thread: t,
            body: text,
            last_message,
            last_prompt,
            history_error,
        })
    }
}
fn message_text(item: &Value) -> Option<String> {
    let text = if let Some(s) = item["text"].as_str() {
        s.to_string()
    } else {
        item["content"]
            .as_array()?
            .iter()
            .filter_map(|c| c["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n")
    };
    if text.is_empty() { None } else { Some(text) }
}
fn render_item(item: &Value, text: &mut String) {
    let kind = item["type"].as_str().unwrap_or("Item");
    text.push_str(&format!("\n[{kind}]\n"));
    for field in ["text", "command", "aggregatedOutput"] {
        if let Some(s) = item[field].as_str() {
            text.extend(s.chars().take(4000));
            text.push('\n');
        }
    }
    if let Some(content) = item["content"].as_array() {
        for c in content {
            if let Some(s) = c["text"].as_str() {
                text.extend(s.chars().take(4000));
                text.push('\n');
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_tungstenite::accept_async;
    #[test]
    fn user_prompt_extracts_all_text_segments() {
        assert_eq!(
            message_text(
                &json!({"content":[{"type":"text","text":"first"},{"type":"image","url":"unused"},{"type":"text","text":"second"}]})
            ),
            Some("first\nsecond".into())
        );
        assert_eq!(message_text(&json!({"content":[]})), None);
    }
    #[tokio::test]
    async fn archive_rechecks_live_status_before_mutating() {
        let (client, server) = tokio::io::duplex(16384);
        let task = tokio::spawn(async move {
            let mut ws = accept_async(server).await.unwrap();
            let mut archives = 0;
            while let Some(Ok(Message::Text(s))) = ws.next().await {
                let v: Value = serde_json::from_str(&s).unwrap();
                if v.get("id").is_none() {
                    continue;
                }
                let result = match v["method"].as_str().unwrap() {
                    "initialize" => json!({}),
                    "thread/read" => json!({"thread":{"status":{"type":"active"}}}),
                    "thread/archive" => {
                        archives += 1;
                        assert_eq!(v["params"]["threadId"], "target");
                        json!({})
                    }
                    _ => panic!("unexpected method"),
                };
                ws.send(Message::Text(
                    json!({"id":v["id"],"result":result}).to_string().into(),
                ))
                .await
                .unwrap();
                if archives == 1 {
                    break;
                }
            }
        });
        let mut rpc = Rpc::from_stream(
            Box::new(client),
            None,
            "fake".into(),
            Duration::from_secs(2),
        )
        .await
        .unwrap();
        assert!(rpc.archive("target", false).await.is_err());
        rpc.archive("target", true).await.unwrap();
        task.await.unwrap();
    }
    #[tokio::test]
    async fn handshake_pagination_notifications_and_errors() {
        let (client, server) = tokio::io::duplex(16384);
        let task = tokio::spawn(async move {
            let mut ws = accept_async(server).await.unwrap();
            let mut pages = 0;
            while let Some(Ok(Message::Text(s))) = ws.next().await {
                let v: Value = serde_json::from_str(&s).unwrap();
                if v.get("id").is_none() {
                    continue;
                }
                let result = match v["method"].as_str().unwrap() {
                    "initialize" => json!({}),
                    "thread/loaded/list" => json!({"data":["unsaved"]}),
                    "thread/read" => json!({"thread":{"id":"unsaved","updatedAt":3}}),
                    "thread/list" => {
                        pages += 1;
                        ws.send(Message::Text(
                            json!({"method":"thread/status/changed","params":{}})
                                .to_string()
                                .into(),
                        ))
                        .await
                        .unwrap();
                        if pages == 1 {
                            json!({"data":[{"id":"one","updatedAt":1}],"nextCursor":"next"})
                        } else {
                            assert_eq!(v["params"]["cursor"], "next");
                            json!({"data":[{"id":"two","updatedAt":2}],"nextCursor":null})
                        }
                    }
                    _ => {
                        ws.send(Message::Text(
                            json!({"id":v["id"],"error":{"code":-32601,"message":"No method"}})
                                .to_string()
                                .into(),
                        ))
                        .await
                        .unwrap();
                        continue;
                    }
                };
                ws.send(Message::Text(
                    json!({"id":v["id"],"result":result}).to_string().into(),
                ))
                .await
                .unwrap();
            }
        });
        let mut rpc = Rpc::from_stream(
            Box::new(client),
            None,
            "fake".into(),
            Duration::from_secs(2),
        )
        .await
        .unwrap();
        let t = rpc.threads().await.unwrap();
        assert_eq!(t.len(), 3);
        assert_eq!(t[0].id, "unsaved");
        assert_eq!(t[1].id, "two");
        assert!(
            rpc.request("missing", json!({}))
                .await
                .unwrap_err()
                .to_string()
                .contains("No method")
        );
        task.abort();
    }
    #[tokio::test]
    async fn request_timeout_is_bounded() {
        let (client, server) = tokio::io::duplex(4096);
        let task = tokio::spawn(async move {
            let mut ws = accept_async(server).await.unwrap();
            let Some(Ok(Message::Text(s))) = ws.next().await else {
                panic!()
            };
            let v: Value = serde_json::from_str(&s).unwrap();
            ws.send(Message::Text(
                json!({"id":v["id"],"result":{}}).to_string().into(),
            ))
            .await
            .unwrap();
            tokio::time::sleep(Duration::from_secs(5)).await;
        });
        let mut rpc = Rpc::from_stream(
            Box::new(client),
            None,
            "fake".into(),
            Duration::from_millis(50),
        )
        .await
        .unwrap();
        assert!(
            rpc.request("thread/list", json!({}))
                .await
                .unwrap_err()
                .to_string()
                .contains("timed out")
        );
        task.abort();
    }
}
