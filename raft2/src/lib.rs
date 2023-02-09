
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
        RwLock,
    },
    time::sleep,
};
use tracing::{debug, info, log::trace, warn};

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
struct VariableState {
    role: Role,
    file: fs::File,
    persistent: Persistent,
    commit_index: usize,
    last_applied: usize,
}

#[derive(Debug)]
pub struct State {
    state: RwLock<VariableState>,
    timeout_cancel_rx: broadcast::Receiver<()>,
    timeout_cancel_tx: broadcast::Sender<()>,
    servers: HashSet<SocketAddr>,
    me: SocketAddr,
}

impl VariableState {
    async fn save_persistent(&mut self) -> anyhow::Result<()> {
        let s = serde_json::to_string_pretty(&self.persistent)?;
        self.file.seek(std::io::SeekFrom::Start(0)).await?;
        self.file.write_all(s.as_bytes()).await?;
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
                state: RwLock::new(VariableState {
                    persistent: (persistent),
                    commit_index: (0),
                    last_applied: (0),
                    role: (Role::Follower),
                    file: (file),
                }),
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
                state: RwLock::new(VariableState {
                    persistent: (persistent),
                    commit_index: (0),
                    last_applied: (0),
                    role: (Role::Follower),
                    file: (file),
                }),
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
async fn send_append_entries(state: Arc<State>, timeout: Duration) -> anyhow::Result<()> {
    info!("send_append_entries");
    let responses = join_all(state.servers.iter().map(|addr| {
        let state = state.clone();
        async move {
            let shared = state.state.read().await;
            let Role::Leader {
                next_index,
                ..
            } = &shared.role
            else { return Ok(true) };
            let prev_log_index = next_index
                .write()
                .await
                .get(addr)
                .ok_or_else(|| anyhow::anyhow!("{addr} not found"))?
                .checked_sub(1)
                .unwrap_or(0);
            let send_logs = shared
                .persistent
                .logs
                .get(prev_log_index + 1..)
                .unwrap_or(&[])
                .to_vec();
            let prev_log_term = shared
                .persistent
                .logs
                .get(prev_log_index)
                .map(|log| log.term)
                .unwrap_or(0);

            let req = AppendEntriesRequest {
                term: shared.persistent.current_term,
                prev_log_index,
                prev_log_term,
                entries: send_logs,
                leader_commit: shared.commit_index,
            };
            drop(shared);

            let res = reqwest::Client::new()
                .post(format!("http://{addr}/append_entries"))
                .timeout(timeout / 3)
                .json(&req)
                .send()
                .await?
                .json::<AppendEntriesResponse>()
                .await?
                .success;

            let shared = state.state.read().await;
            let Role::Leader {
                next_index,
                match_index,
            } = &shared.role else {
                return Ok(true)
            };

            if res {
                next_index
                    .write()
                    .await
                    .insert(*addr, shared.persistent.logs.len());
                match_index
                    .write()
                    .await
                    .insert(*addr, shared.persistent.logs.len());
            } else {
                next_index
                    .write()
                    .await
                    .entry(*addr)
                    .and_modify(|next_index| {
                        if *next_index != 0 {
                            *next_index -= 1
                        }
                    });
            }
            Ok::<_, anyhow::Error>(res)
        }
    }))
    .await;
    for res in &responses {
        debug!(res = format!("{res:?}"), "response");
    }
    if responses.into_iter().all(|res| res.unwrap_or(false)) {
        info!("exit_append_entries");
        Ok(())
    } else {
        //tokio::spawn(send_append_entries(state.clone(), timeout));
        info!("retry_append_entries");
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
    dbg!(&state.state);
    let mut shared = state.state.write().await;
    if !shared.role.is_follower() {
        debug!("i_am_not_follower");
        return Ok(());
    }
    info!("start_election");
    shared.role = Role::Candidate;
    shared.persistent.current_term += 1;
    shared.save_persistent().await?;
    shared.persistent.voted_for = None;
    drop(shared);
    let vote_result = join_all(state.servers.iter().map(|addr| {
        let state = state.clone();
        async move {
            let shared = state.state.read().await;
            if *addr == state.me {
                return Ok(true);
            }
            let granted = reqwest::Client::new()
                .post(format!("http://{addr}/request_vote"))
                .timeout(timeout / 3)
                .json(&RequestVoteRequest {
                    term: shared.persistent.current_term,
                    candidate_id: state.me,
                    last_log_index: shared.persistent.logs.len() - 1,
                    last_log_term: shared
                        .persistent
                        .logs
                        .last()
                        .map(|log| log.term)
                        .unwrap_or(0),
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
        debug!(res = format!("{result:?}"), "vote_response");
        result.as_ref().map(|b| *b).unwrap_or(false)
    })
    .count();

    info!(
        total = state.servers.len(),
        gain = vote_result,
        "vote_result"
    );
    let mut shared = state.state.write().await;
    if vote_result * 2 > state.servers.len() {
        if !shared.role.is_candidate() {
            return Ok(());
        }
        info!("vote_win");
        // 当選
        let next_index_base = shared.persistent.logs.len() - 1;
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
        shared.persistent.voted_for = None;
        shared.save_persistent().await?;
        shared.role = Role::Leader {
            next_index: RwLock::new(next_index),
            match_index: RwLock::new(match_index),
        };
        let state = state.clone();
        tokio::spawn(async move {
            loop {
                let now = Instant::now();
                if let Err(e) = send_append_entries(state.clone(), timeout).await {
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
        info!("vote_loose");
        // 落選
        shared.persistent.voted_for = None;
        shared.save_persistent().await?;
        shared.role = Role::Follower;
        Ok(())
    }
}

pub async fn launch(state: Arc<State>, timeout: Duration, heatbeat: Duration) {
    let mut rx = state.timeout_cancel_rx.resubscribe();
    tokio::spawn(async move {
        loop {
            let rx2 = rx.resubscribe();
            if let Err(e) = tokio::select! {
                _ = rx.recv() => {
                    Ok(())
                },
                _ = tokio::time::sleep(timeout) => {
                    trace!("append_entries_timeout");
                    open_election(state.clone(), timeout, heatbeat, rx2).await
                }
            } {
                warn!("{e}");
            }
        }
    });
}

async fn putoff_timeout(state: &State) -> anyhow::Result<()> {
    state.timeout_cancel_tx.send(())?;
    Ok(())
}

async fn post_log(_state: Arc<State>, _logs: Vec<String>) -> anyhow::Result<()> {
    unimplemented!()
}

pub async fn append_entries(
    state: Arc<State>,
    req: AppendEntriesRequest,
) -> anyhow::Result<AppendEntriesResponse> {
    let mut shared = state.state.write().await;
    if req.term < shared.persistent.current_term {
        debug!("deny_append_entry_by_term");
        return Ok(AppendEntriesResponse { success: false });
    }
    putoff_timeout(&state).await?;
    if req.term > shared.persistent.current_term {
        shared.role = Role::Follower;
        shared.persistent.voted_for = None;
    }
    if req.prev_log_index >= shared.persistent.logs.len()
        || shared.persistent.logs[req.prev_log_index].term != req.prev_log_term
    {
        debug!(
            logs = format!("{:?}", shared.persistent.logs),
            prev_log_index = req.prev_log_index,
            prev_log_term = req.prev_log_term,
            "deny_append_entry_by_conflicting"
        );
        return Ok(AppendEntriesResponse { success: false });
    }
    shared.persistent.logs.resize_with(
        req.prev_log_index + 1 + req.entries.len(),
        || unreachable!(),
    );
    for (offset, log) in req.entries.into_iter().enumerate() {
        shared
            .persistent
            .logs
            .insert(offset + req.prev_log_index + 1, log);
    }
    if req.leader_commit > shared.commit_index {
        shared.commit_index = req.leader_commit.min(shared.persistent.logs.len() - 1);
    }
    shared.save_persistent().await?;
    Ok(AppendEntriesResponse { success: true })
}

pub async fn request_vote(
    state: Arc<State>,
    req: RequestVoteRequest,
) -> anyhow::Result<RequestVoteResponse> {
    info!("try_request_vote");
    let mut shared = state.state.write().await;
    if req.term < shared.persistent.current_term {
        info!("deny_request_vote");
        return Ok(RequestVoteResponse {
            term: req.term,
            vote_granted: false,
        });
    }
    if req.term > shared.persistent.current_term {
        shared.role = Role::Follower;
    }
    let vote_granted = shared
        .persistent
        .voted_for
        .map(|candidate| candidate == req.candidate_id)
        .unwrap_or(true);
    if vote_granted {
        putoff_timeout(&state).await?;
    }
    shared.persistent.voted_for = Some(req.candidate_id);
    shared.save_persistent().await?;
    info!("finish_request_vote");
    Ok(RequestVoteResponse {
        term: req.term,
        vote_granted,
    })
}
