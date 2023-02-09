use anyhow::ensure;
use futures::future::join_all;
use itertools::Itertools;
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
use tracing::{debug, info, log::trace, warn};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Log {
    pub text: String,
    pub term: u64,
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
}

#[derive(Debug)]
pub struct State {
    state: RwLock<VariableState>,
    timeout_cancel_rx: broadcast::Receiver<()>,
    timeout_cancel_tx: broadcast::Sender<()>,
    servers: HashSet<SocketAddr>,
    append_entries_lock: Mutex<()>,
    me: SocketAddr,
}

impl VariableState {
    async fn save_persistent(&mut self) -> anyhow::Result<()> {
        let s = serde_json::to_string_pretty(&self.persistent)?;
        self.file.seek(std::io::SeekFrom::Start(0)).await?;
        self.file.set_len(s.bytes().len() as u64).await?;
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
                    role: (Role::Follower),
                    file: (file),
                }),
                timeout_cancel_rx: rx,
                timeout_cancel_tx: tx,
                append_entries_lock: Mutex::new(()),
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
                    role: (Role::Follower),
                    file: (file),
                }),
                timeout_cancel_rx: rx,
                timeout_cancel_tx: tx,
                append_entries_lock: Mutex::new(()),
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
async fn send_append_entries(
    state: Arc<State>,
    timeout: Duration,
    append_logs: Vec<String>,
) -> anyhow::Result<bool> {
    // persistent.logsが途中で書き換えられたらまずい
    // append_entriesが複数発行されると同じタームを持つリーダーが二人いるのと同じ状態になる
    // そもそもappend_entriesを並列して実行する必要はないのでロックを取る
    // ここ以外にpersistent_logsが変更される可能性のある場所はappend_entriesの受けてハンドラのみ
    // ただそこではfollowerでなければ拒否される
    // followerに落ちているならばappend_entriesは過半数から拒否されるはずなので問題なし。どうせ捨てられる
    // followerに落ちていないならば変更されないので問題なし
    // append_entriesが適用されているが応答が帰ってない場合はリーダーから転落することはない
    // なぜならappend_entriesのタイムアウトはリーダーのタイムアウトより短い
    // よってappend_entries適用後の応答がリーダーがタイムアウト出来るほど長いならその前に
    // こちら側でタイムアウトが起きる
    let _ = state.append_entries_lock.lock().await;
    let mut shared = state.state.write().await;
    ensure!(shared.role.is_leader(), "i_am_not_leader");
    let mut logs = append_logs
        .into_iter()
        .map(|text| Log {
            term: shared.persistent.current_term,
            text,
        })
        .collect_vec();
    shared.persistent.logs.append(&mut logs);
    shared.save_persistent().await?;
    drop(shared);
    info!("send_append_entries");
    let responses = join_all(state.servers.iter().map(|addr| {
        let state = state.clone();
        async move {
            let shared = state.state.read().await;

            let Role::Leader {
                next_index,
                match_index,
            } = &shared.role
            else { return Ok(true) };
            if *addr == state.me {
                next_index
                    .write()
                    .await
                    .insert(state.me, shared.persistent.logs.len());
                match_index
                    .write()
                    .await
                    .insert(state.me, shared.persistent.logs.len() - 1);
                return Ok(true);
            }
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
                .put(format!("http://{addr}/api/log"))
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
                    .insert(*addr, shared.persistent.logs.len() - 1);
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
    let success_count = responses
        .into_iter()
        .filter(|res| if let Ok(b) = res { *b } else { false })
        .count();
    // 過半数が成功
    if success_count * 2 > state.servers.len() {
        let mut shared = state.state.write().await;
        shared.commit_index = shared.persistent.logs.len() - 1;
        info!("exit_append_entries");
        Ok(true)
    } else {
        //tokio::spawn(send_append_entries(state.clone(), timeout));
        info!("retry_append_entries");
        Ok(false)
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
    let mut shared = state.state.write().await;
    if !shared.role.is_follower() {
        debug!("i_am_not_follower");
        return Ok(());
    }
    info!("start_election");
    putoff_timeout(&state).await?;
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
                .post(format!("http://{addr}/api/vote"))
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
    // この後起こりうる状態は選挙での敗北か勝利の二択
    // この間ログ関連は変更される可能性があるがそれは選挙とは関係しないので問題ない
    // * 敗北したのに途中の変更により勝利できる状態になるかもしれないが、別の勝者がいるならそれで問題ない
    //   * Raftでは自己判断でリーダーに昇格はしない。必ず承認が必要なのでリーダーが複数現れることにはならない
    // また別のリーダーが現れてフォロワーに落ちる場合がある
    // * 選挙で勝ったならば新しいリーダーになれば良い
    //   * ただしフォロワーに転落したということはおそらく選挙にも負けているはず……
    //   * 誤って勝者となっても既に過半数からはAppendEntriesが拒否されるはずなので問題ない
    // * が、フォロワーに落ちているなら落ちたままが望ましいのでチェックしておく
    // * 途中でtermが増加する可能性がある
    //   * 選挙中にも関わらずさらに新しく選挙を開始した場合がこれ
    //     * これは発生しない。なぜなら選挙が始まった時点でタイムアウトの延期を行う。選挙はタイムアウトより短い時間でタイムアウトする。
    // . * フォロワーに転落する場合もあり得る
    // .   * 前述の状態なので問題ない
    if !shared.role.is_candidate() {
        return Ok(());
    }
    if vote_result * 2 > state.servers.len() {
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
                if !state.state.read().await.role.is_leader() {
                    break;
                }
                let now = Instant::now();
                if let Err(e) = send_append_entries(state.clone(), timeout, Vec::new()).await {
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

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GetLogResponse {
    commited: Vec<Log>,
    all: Vec<Log>,
}

pub async fn get_log(state: Arc<State>) -> GetLogResponse {
    let shared = state.state.read().await;
    let logs = &shared.persistent.logs;
    GetLogResponse {
        commited: logs.get(0..shared.commit_index + 1).unwrap_or(&[]).to_vec(),
        all: logs.clone(),
    }
}

pub async fn post_log(
    state: Arc<State>,
    logs: Vec<String>,
    timeout: Duration,
) -> anyhow::Result<bool> {
    send_append_entries(state.clone(), timeout, logs).await
}

pub async fn append_entries(
    state: Arc<State>,
    req: AppendEntriesRequest,
) -> anyhow::Result<AppendEntriesResponse> {
    let mut shared = state.state.write().await;
    if req.term < shared.persistent.current_term {
        debug!(by = "term", "deny_append_entry");
        return Ok(AppendEntriesResponse { success: false });
    }
    putoff_timeout(&state).await?;
    if req.term > shared.persistent.current_term {
        info!("turn_to_follower");
        shared.role = Role::Follower;
        shared.persistent.current_term = req.term;
        shared.persistent.voted_for = None;
        shared.save_persistent().await?;
    }
    if !shared.role.is_follower() {
        debug!(by = "role", "deny_append_entry");
        return Ok(AppendEntriesResponse { success: false });
    }
    if req.prev_log_index >= shared.persistent.logs.len()
        || shared.persistent.logs[req.prev_log_index].term != req.prev_log_term
    {
        debug!(
            by = "conflicting",
            logs = format!("{:?}", shared.persistent.logs),
            prev_log_index = req.prev_log_index,
            prev_log_term = req.prev_log_term,
            "deny_append_entry"
        );
        return Ok(AppendEntriesResponse { success: false });
    }
    if shared.persistent.logs.len() > req.prev_log_index + 1 + req.entries.len() {
        shared.persistent.logs.resize_with(
            req.prev_log_index + 1 + req.entries.len(),
            || unreachable!(),
        );
    }
    for (offset, log) in req.entries.into_iter().enumerate() {
        shared
            .persistent
            .logs
            .insert(offset + req.prev_log_index + 1, log);
    }
    if req.leader_commit > shared.commit_index {
        shared.commit_index = req.leader_commit.min(shared.persistent.logs.len() - 1);
    }
    debug!(logs = format!("{:?}", shared.persistent.logs), "saved_log");
    shared.save_persistent().await?;
    Ok(AppendEntriesResponse { success: true })
}

pub async fn request_vote(
    state: Arc<State>,
    req: RequestVoteRequest,
) -> anyhow::Result<RequestVoteResponse> {
    info!("try_request_vote");
    let mut shared = state.state.write().await;
    // 古いタームからの投票要求は拒否
    if req.term < shared.persistent.current_term {
        info!("deny_request_vote");
        return Ok(RequestVoteResponse {
            term: req.term,
            vote_granted: false,
        });
    }

    // 自身のタームが古い場合はFollowerに戻る
    if req.term > shared.persistent.current_term {
        info!(by = "vote", "turn_to_follower");
        shared.role = Role::Follower;
        shared.persistent.current_term = req.term;
        // 投票も無効なのでリセット
        shared.persistent.voted_for = None;
        shared.save_persistent().await?;
    }

    // logに関する投票可否
    // 最新のlogのタームが相手の方が新しければ投票可能
    // どちらも同じ場合は投票しても良い
    let log_condition = req.last_log_term
        > shared
            .persistent
            .logs
            .last()
            .map(|log| log.term)
            .unwrap_or(0)
        || req.last_log_index + 1 >= shared.persistent.logs.len();

    // ログにより投票不可なら拒否
    if !log_condition {
        if Some(req.candidate_id) == shared.persistent.voted_for {
            shared.persistent.voted_for = None;
            shared.save_persistent().await?;
        }
        return Ok(RequestVoteResponse {
            term: req.term.max(shared.persistent.current_term),
            vote_granted: false,
        });
    }

    // 既に投票していた場合は投票不可
    let vote_granted = shared
        .persistent
        .voted_for
        .map(|candidate| candidate == req.candidate_id)
        .unwrap_or(true);

    // 投票可能なら投票先を保存
    // 投票が有効なのでAppendEntriesのタイムアウトも延期する
    // ↑元論文にはない
    if vote_granted {
        putoff_timeout(&state).await?;
        shared.persistent.voted_for = Some(req.candidate_id);
        shared.save_persistent().await?;
    }

    info!("finish_request_vote");
    Ok(RequestVoteResponse {
        term: req.term.max(shared.persistent.current_term),
        vote_granted,
    })
}
