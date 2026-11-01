use crate::{
    actions::{Outcome, TaskAction, create_worktree},
    config::{Config, Host},
    model::{SessionDetail, Snapshot, now},
    protocol::Rpc,
};
use anyhow::Context;
use tokio::{
    sync::mpsc,
    task::JoinHandle,
    time::{Duration, sleep},
};

pub enum Event {
    TaskCompleted {
        host: String,
        target: Option<String>,
        label: &'static str,
        result: Result<Outcome, String>,
    },
    Archived {
        key: (String, String),
        result: Result<(), String>,
    },
    Snapshot(Snapshot),
    Details {
        key: (String, String),
        result: Result<SessionDetail, String>,
    },
}
pub enum Action {
    Task(TaskAction),
    Archive(String, bool),
    Refresh,
    Details(String),
}
pub struct Fleet {
    pub events: mpsc::UnboundedReceiver<Event>,
    controls: Vec<(String, mpsc::UnboundedSender<Action>)>,
    tasks: Vec<JoinHandle<()>>,
}
impl Fleet {
    pub fn task(&self, host: &str, action: TaskAction) {
        if let Some((_, tx)) = self.controls.iter().find(|(id, _)| id == host) {
            let _ = tx.send(Action::Task(action));
        }
    }
    pub fn start(config: &Config) -> Self {
        let (tx, events) = mpsc::unbounded_channel();
        let mut controls = vec![];
        let mut tasks = vec![];
        for host in &config.hosts {
            let (control, commands) = mpsc::unbounded_channel();
            controls.push((host.id.clone(), control));
            tasks.push(tokio::spawn(worker(
                host.clone(),
                config.clone(),
                tx.clone(),
                commands,
            )));
        }
        Self {
            events,
            controls,
            tasks,
        }
    }
    pub fn refresh(&self) {
        for (_, tx) in &self.controls {
            let _ = tx.send(Action::Refresh);
        }
    }
    pub fn details(&self, key: &(String, String)) {
        if let Some((_, tx)) = self.controls.iter().find(|(id, _)| id == &key.0) {
            let _ = tx.send(Action::Details(key.1.clone()));
        }
    }
    pub fn archive(&self, key: &(String, String), allow_active: bool) {
        if let Some((_, tx)) = self.controls.iter().find(|(id, _)| id == &key.0) {
            let _ = tx.send(Action::Archive(key.1.clone(), allow_active));
        }
    }
}
impl Drop for Fleet {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

async fn worker(
    host: Host,
    config: Config,
    tx: mpsc::UnboundedSender<Event>,
    mut commands: mpsc::UnboundedReceiver<Action>,
) {
    let mut snapshot = Snapshot::pending(host.clone());
    let mut rpc: Option<Rpc> = None;
    let mut backoff = 1;
    loop {
        let result = async {
            if rpc.is_none() {
                rpc = Some(Rpc::connect(&host, config.timeout_seconds).await?);
            }
            let rpc = rpc.as_mut().expect("connected");
            let sessions = rpc.threads().await?;
            Ok::<_, anyhow::Error>((sessions, rpc.socket.clone()))
        }
        .await;
        match result {
            Ok((sessions, socket)) => {
                snapshot.sessions = sessions;
                snapshot.socket = Some(socket);
                snapshot.online = true;
                snapshot.error = None;
                snapshot.last_seen = Some(now());
                backoff = 1;
            }
            Err(e) => {
                snapshot.online = false;
                snapshot.error = Some(format!("{e:#}"));
                rpc = None;
            }
        }
        if tx.send(Event::Snapshot(snapshot.clone())).is_err() {
            return;
        }
        let wait = if snapshot.online {
            config.poll_seconds
        } else {
            let n = backoff;
            backoff = (backoff * 2).min(30);
            n
        };
        let delay = sleep(Duration::from_secs(wait));
        tokio::pin!(delay);
        loop {
            tokio::select! {
                _=&mut delay=>break,
                command=commands.recv()=>match command {
                    Some(Action::Refresh)=>break,
                    Some(Action::Details(id))=>{
                        let result=if let Some(r)=rpc.as_mut() {r.details(&id).await.map_err(|e|format!("{e:#}"))}
                            else {Err("Host is offline; showing cached metadata only.".into())};
                        let _=tx.send(Event::Details {key:(host.id.clone(),id),result});
                    },
                    Some(Action::Archive(id,allow_active))=>{
                        let result=if let Some(r)=rpc.as_mut() {r.archive(&id,allow_active).await.map_err(|e|format!("{e:#}"))}
                            else {Err("Device is offline".into())};
                        let _=tx.send(Event::Archived {key:(host.id.clone(),id),result});
                        break;
                    },
                    Some(Action::Task(action))=>{
                        let target=action.id().map(str::to_string);let label=action.label();
                        let result=async {
                            let rpc=rpc.as_mut().ok_or_else(||anyhow::anyhow!("Device is offline"))?;
                            Ok::<_,anyhow::Error>(match action {
                                TaskAction::New{cwd}=>Outcome::Created(Box::new(rpc.start(&cwd).await?)),
                                TaskAction::Worktree{cwd,branch}=>{
                                    let path=create_worktree(&host,&cwd,&branch).await?;
                                    Outcome::Created(Box::new(rpc.start(&path).await.with_context(||format!("Worktree retained at {path}"))?))
                                },
                                TaskAction::Fork{id}=>Outcome::Created(Box::new(rpc.fork(&id).await?)),
                                TaskAction::Rename{id,name}=>{rpc.rename(&id,&name).await?;Outcome::Renamed(name)},
                                TaskAction::Delete{id}=>{rpc.delete(&id).await?;Outcome::Deleted},
                                TaskAction::Stop{id}=>{rpc.stop(&id).await?;Outcome::Stopped},
                            })
                        }.await.map_err(|e|format!("{e:#}"));
                        let _=tx.send(Event::TaskCompleted{host:host.id.clone(),target,label,result});
                        break;
                    },
                    None=>return,
                }
            }
        }
    }
}

pub async fn snapshot_once(host: Host, timeout: u64) -> Snapshot {
    let mut snapshot = Snapshot::pending(host.clone());
    match Rpc::connect(&host, timeout).await {
        Ok(mut rpc) => match rpc.threads().await {
            Ok(sessions) => {
                snapshot.sessions = sessions;
                snapshot.socket = Some(rpc.socket);
                snapshot.online = true;
                snapshot.last_seen = Some(now());
            }
            Err(e) => snapshot.error = Some(format!("{e:#}")),
        },
        Err(e) => snapshot.error = Some(format!("{e:#}")),
    }
    snapshot
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use serde_json::{Value, json};
    use tokio_tungstenite::{accept_async, tungstenite::Message};

    #[tokio::test]
    async fn disconnected_host_keeps_cache_and_recovers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.sock");
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let server = tokio::spawn(async move {
            for id in ["before", "after"] {
                let (stream, _) = listener.accept().await.unwrap();
                let mut ws = accept_async(stream).await.unwrap();
                while let Some(Ok(Message::Text(s))) = ws.next().await {
                    let v: Value = serde_json::from_str(&s).unwrap();
                    if v.get("id").is_none() {
                        continue;
                    }
                    let is_list = v["method"] == "thread/list";
                    let result = if is_list {
                        json!({"data":[{"id":id,"status":{"type":"idle"}}],"nextCursor":null})
                    } else {
                        json!({})
                    };
                    ws.send(Message::Text(
                        json!({"id":v["id"],"result":result}).to_string().into(),
                    ))
                    .await
                    .unwrap();
                    if v["method"] == "thread/loaded/list" {
                        break;
                    }
                }
            }
        });
        let mut config = Config {
            poll_seconds: 1,
            timeout_seconds: 1,
            ..Config::default()
        };
        config.hosts[0].socket = Some(path.to_string_lossy().into());
        let mut fleet = Fleet::start(&config);
        let mut samples = vec![];
        tokio::time::timeout(Duration::from_secs(6), async {
            while samples.len() < 3 {
                if let Some(Event::Snapshot(s)) = fleet.events.recv().await {
                    samples.push(s);
                }
            }
        })
        .await
        .unwrap();
        assert!(samples[0].online);
        assert_eq!(samples[0].sessions[0].id, "before");
        assert!(!samples[1].online);
        assert_eq!(samples[1].sessions[0].id, "before");
        assert_eq!(samples[0].last_seen, samples[1].last_seen);
        assert!(samples[2].online);
        assert_eq!(samples[2].sessions[0].id, "after");
        server.await.unwrap();
    }
}
