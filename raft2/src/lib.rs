use futures::future::join_all;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    fs::{self, OpenOptions},
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    sync::{
        broadcast::{self, Receiver},
        Mutex, RwLock,
    },
    time::sleep,
};
use tracing::{debug, info, warn};

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Log {
    text: String,
    term: u64,
}

#[derive(Serialize, Deserialize, Debug)]
struct Persistent {
    current_term: u64,
    voted_for: Option<SocketAddr>,
    logs: Vec<Log>,
}

#[derive(Debug)]
pub struct State {
    persistent: RwLock<Persistent>,
    commit_index: RwLock<usize>,
    last_applied: RwLock<usize>,
    role: RwLock<Role>,
    file: Mutex<fs::File>,
    timeout_cancel_rx: broadcast::Receiver<()>,
    timeout_cancel_tx: broadcast::Sender<()>,
    servers: HashSet<SocketAddr>,
    me: SocketAddr,
}

impl Persistent {
    async fn save_persistent(&self, file: &Mutex<fs::File>) -> anyhow::Result<()> {
        let s = serde_json::to_string_pretty(&self)?;
        let mut file = file.lock().await;
        file.seek(std::io::SeekFrom::Start(0)).await?;
        file.write_all(s.as_bytes()).await?;
        Ok(())
    }
}

impl State {
    pub async fn from_file<P: AsRef<Path>>(
        path: P,
        me: SocketAddr,
        servers: HashSet<SocketAddr>,
    ) -> anyhow::Result<Self> {
        let path = path.as_ref();

        let (tx, rx) = broadcast::channel(1024);
        if path.exists() {
            let mut file = OpenOptions::new()
                .write(true)
                .read(true)
                .append(false)
                .open(path)
                .await?;
            let mut s = String::new();
            file.read_to_string(&mut s).await?;
            let persistent = serde_json::from_str(&s)?;
            Ok(Self {
                servers,
                persistent: RwLock::new(persistent),
                commit_index: RwLock::new(0),
                last_applied: RwLock::new(0),
                role: RwLock::new(Role::Follower),
                file: Mutex::new(file),
                timeout_cancel_rx: rx,
                timeout_cancel_tx: tx,
                me,
            })
        } else {
            let persistent = Persistent {
                current_term: 0,
                voted_for: None,
                logs: vec![Log {
                    term: 0,
                    text: "init".to_owned(),
                }],
            };
            let mut file = fs::File::create(path).await?;
            file.write_all(&serde_json::to_vec(&persistent)?).await?;
            Ok(Self {
                servers,
                persistent: RwLock::new(persistent),
                commit_index: RwLock::new(0),
                last_applied: RwLock::new(0),
                role: RwLock::new(Role::Follower),
                file: Mutex::new(file),
                timeout_cancel_rx: rx,
                timeout_cancel_tx: tx,
                me,
            })
        }
    }
}

#[derive(Debug)]
enum Role {
    Follower,
    Candidate,
    Leader {
        next_index: RwLock<HashMap<SocketAddr, usize>>,
        match_index: RwLock<HashMap<SocketAddr, usize>>,
    },
}

impl Role {
    fn is_follower(&self) -> bool {
        matches!(&self, &Role::Follower)
    }

    fn is_candidate(&self) -> bool {
        matches!(&self, &Role::Candidate)
    }

    fn is_leader(&self) -> bool {
        matches!(&self, &Role::Leader { .. })
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AppendEntriesRequest {
    term: u64,
    prev_log_index: usize,
    prev_log_term: u64,
    entries: Vec<Log>,
    leader_commit: usize,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AppendEntriesResponse {
    success: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RequestVoteRequest {
    term: u64,
    candidate_id: SocketAddr,
    last_log_index: usize,
    last_log_term: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RequestVoteResponse {
    term: u64,
    vote_granted: bool,
}

#[async_recursion::async_recursion]
async fn send_append_entries(state: Arc<State>) -> anyhow::Result<()> {
    let responses = join_all(state.servers.iter().map(|addr| {
        let state = state.clone();
        async move {
            if let Role::Leader {
                next_index,
                match_index,
            } = &*state.role.read().await
            {
                let prev_log_index = next_index
                    .write()
                    .await
                    .get(&addr)
                    .ok_or_else(|| anyhow::anyhow!("{addr} not found"))?
                    .checked_sub(1)
                    .unwrap_or(0);
                let persistent = state.persistent.read().await;
                let send_logs = persistent
                    .logs
                    .get(prev_log_index + 1..)
                    .unwrap_or(&[])
                    .to_vec();
                let prev_log_term = persistent
                    .logs
                    .get(prev_log_index)
                    .map(|log| log.term)
                    .unwrap_or(0);
                let res = reqwest::Client::new()
                    .post(format!("http://{addr}/append_entries"))
                    .json(&AppendEntriesRequest {
                        term: persistent.current_term,
                        prev_log_index,
                        prev_log_term,
                        entries: send_logs,
                        leader_commit: *state.commit_index.read().await,
                    })
                    .send()
                    .await?
                    .json::<AppendEntriesResponse>()
                    .await?
                    .success;
                if res {
                    next_index
                        .write()
                        .await
                        .insert(*addr, persistent.logs.len());
                    match_index
                        .write()
                        .await
                        .insert(*addr, persistent.logs.len());
                } else {
                    next_index
                        .write()
                        .await
                        .entry(*addr)
                        .and_modify(|next_index| {
                            if *next_index != 0 {
                                *next_index = *next_index - 1
                            }
                        });
                }
                Ok::<_, anyhow::Error>(res)
            } else {
                Ok(true)
            }
        }
    }))
    .await;
    if responses.into_iter().all(|res| {
        debug!(res = format!("{:?}", res), "response");
        res.unwrap_or(false)
    }) {
        Ok(())
    } else {
        tokio::spawn(send_append_entries(state.clone()));
        Ok(())
    }
}

async fn open_election(
    state: Arc<State>,
    timeout: Duration,
    heatbeat: Duration,
    mut rx: Receiver<()>,
) -> anyhow::Result<()> {
    let ratio: f64 = rand::random();
    let timeout_ms = timeout.as_millis() as f64 * ratio;
    let timeout = Duration::from_millis(timeout_ms as u64);
    sleep(timeout).await;
    // cancel
    if rx.try_recv().is_ok() {
        return Ok(());
    }
    let mut persistent = state.persistent.write().await;
    let mut role = state.role.write().await;
    if !role.is_follower() {
        return Ok(());
    }
    info!("start_election");
    *role = Role::Candidate;
    persistent.current_term += 1;
    persistent.save_persistent(&state.file).await?;
    persistent.voted_for = None;
    drop(persistent);
    drop(role);
    let vote_result = join_all(state.servers.iter().map(|addr| {
        let state = state.clone();
        async move {
            let persistent = state.persistent.read().await;
            if *addr == state.me {
                return Ok(true);
            }
            let granted = reqwest::Client::new()
                .post(format!("http://{addr}/request_vote"))
                .json(&RequestVoteRequest {
                    term: persistent.current_term,
                    candidate_id: state.me,
                    last_log_index: persistent.logs.len() - 1,
                    last_log_term: persistent.logs.last().map(|log| log.term).unwrap_or(0),
                })
                .send()
                .await?
                .json::<RequestVoteResponse>()
                .await?
                .vote_granted;
            Ok::<_, anyhow::Error>(granted)
        }
    }))
    .await
    .into_iter()
    .filter(|result| {
        debug!(res = format!("{:?}", result), "vote_response");
        result.as_ref().map(|b| *b).unwrap_or(false)
    })
    .count();
    info!(
        total = state.servers.len(),
        gain = vote_result,
        "vote_result"
    );
    if vote_result * 2 > state.servers.len() {
        let mut role = state.role.write().await;
        if !role.is_candidate() {
            return Ok(());
        }
        // 当選
        let next_index_base = state.persistent.read().await.logs.len() - 1;
        let next_index = state
            .servers
            .iter()
            .map(|addr| (*addr, next_index_base))
            .collect::<HashMap<_, _>>();
        let match_index = state
            .servers
            .iter()
            .map(|addr| (*addr, 0))
            .collect::<HashMap<_, _>>();
        let mut persistent = state.persistent.write().await;
        persistent.voted_for = None;
        persistent.save_persistent(&state.file).await?;
        *role = Role::Leader {
            next_index: RwLock::new(next_index),
            match_index: RwLock::new(match_index),
        };
        let state = state.clone();
        tokio::spawn(async move {
            loop {
                let now = Instant::now();
                if let Err(e) = send_append_entries(state.clone()).await {
                    warn!("{e}");
                }
                let elapsed = now.elapsed();
                if heatbeat > elapsed {
                    sleep(heatbeat - elapsed).await;
                }
            }
        });
        Ok(())
    } else {
        let mut persistent = state.persistent.write().await;
        // 落選
        persistent.voted_for = None;
        persistent.save_persistent(&state.file).await?;
        *state.role.write().await = Role::Follower;
        Ok(())
    }
}

pub async fn launch(state: Arc<State>, timeout: Duration, heatbeat: Duration) {
    let mut rx = state.timeout_cancel_rx.resubscribe();
    tokio::spawn(async move {
        loop {
            let rx2 = rx.resubscribe();
            if let Err(e) = tokio::select! {
                _ = rx.recv() => Ok(()),
                _ = tokio::time::sleep(timeout) => {
                    open_election(state.clone(), timeout, heatbeat, rx2).await
                }
            } {
                warn!("{e}");
            }
        }
    });
}

async fn putoff_timeout(state: Arc<State>) -> anyhow::Result<()> {
    state.timeout_cancel_tx.send(())?;
    Ok(())
}

async fn post_log(state: Arc<State>, logs: Vec<String>) -> anyhow::Result<()> {
    unimplemented!()
}

pub async fn append_entries(
    state: Arc<State>,
    req: AppendEntriesRequest,
) -> anyhow::Result<AppendEntriesResponse> {
    let mut persistent = state.persistent.write().await;
    if req.term < persistent.current_term {
        return Ok(AppendEntriesResponse { success: false });
    }
    if req.term > persistent.current_term {
        *state.role.write().await = Role::Follower;
        persistent.voted_for = None;
    }
    if req.prev_log_index >= persistent.logs.len()
        || persistent.logs[req.prev_log_index].term != req.term
    {
        return Ok(AppendEntriesResponse { success: false });
    }
    persistent.logs.resize_with(
        req.prev_log_index + 1 + req.entries.len(),
        || unreachable!(),
    );
    for (offset, log) in req.entries.into_iter().enumerate() {
        persistent.logs.insert(offset + req.prev_log_index + 1, log);
    }
    let mut commit_index = state.commit_index.write().await;
    if req.leader_commit > *commit_index {
        *commit_index = req.leader_commit.min(persistent.logs.len() - 1);
    }
    persistent.save_persistent(&state.file).await?;
    Ok(AppendEntriesResponse { success: true })
}

pub async fn request_vote(
    state: Arc<State>,
    req: RequestVoteRequest,
) -> anyhow::Result<RequestVoteResponse> {
    let mut persistent = state.persistent.write().await;
    if req.term < persistent.current_term {
        return Ok(RequestVoteResponse {
            term: req.term,
            vote_granted: false,
        });
    }
    if req.term > persistent.current_term {
        *state.role.write().await = Role::Follower;
    }
    let vote_granted = persistent
        .voted_for
        .map(|candidate| candidate == req.candidate_id)
        .unwrap_or(true);
    persistent.voted_for = Some(req.candidate_id);
    persistent.save_persistent(&state.file).await?;
    Ok(RequestVoteResponse {
        term: req.term,
        vote_granted,
    })
}
