use std::{
    collections::{HashMap, HashSet},
    io,
    net::SocketAddr,
    path::Path,
    process::exit,
    sync::Arc,
    time::{Duration, Instant},
};

pub mod types {
    use serde::{Deserialize, Serialize};

    use super::proto::{
        AppendEntriesRequest, AppendEntriesResponse, RequestVoteRequest, RequestVoteResponse,
    };

    #[derive(Serialize, Deserialize, Clone, Debug)]
    pub struct Log<T> {
        pub inner: T,
        pub term: Term,
    }

    pub struct TypedAppendEntriesRequest {
        pub term: Term,
        pub leader_id: String,
        pub prev_log_index: LogIndex,
        pub prev_log_term: Term,
        pub entries: Vec<Vec<u8>>,
        pub leader_commit: LogIndex,
    }

    pub struct TypedRequestVoteRequest {
        pub term: Term,
        pub candidate_id: String,
        pub last_log_index: LogIndex,
        pub last_log_term: Term,
    }

    pub struct TypedRequestVoteResponse {
        pub vote_granted: bool,
        pub term: Term,
    }

    impl From<RequestVoteResponse> for TypedRequestVoteResponse {
        fn from(value: RequestVoteResponse) -> Self {
            TypedRequestVoteResponse {
                vote_granted: value.vote_granted,
                term: Term(value.term),
            }
        }
    }

    impl From<TypedRequestVoteResponse> for RequestVoteResponse {
        fn from(value: TypedRequestVoteResponse) -> Self {
            RequestVoteResponse {
                term: value.term.0,
                vote_granted: value.vote_granted,
            }
        }
    }

    impl From<RequestVoteRequest> for TypedRequestVoteRequest {
        fn from(value: RequestVoteRequest) -> Self {
            Self {
                term: Term(value.term),
                candidate_id: value.candidate_id,
                last_log_index: LogIndex(value.last_log_index),
                last_log_term: Term(value.last_log_term),
            }
        }
    }

    impl From<TypedRequestVoteRequest> for RequestVoteRequest {
        fn from(value: TypedRequestVoteRequest) -> Self {
            Self {
                term: value.term.0,
                candidate_id: value.candidate_id,
                last_log_index: value.last_log_index.0,
                last_log_term: value.last_log_term.0,
            }
        }
    }

    pub struct TypedAppendEntriesResponse {
        pub term: Term,
        pub success: bool,
    }

    impl From<TypedAppendEntriesResponse> for AppendEntriesResponse {
        fn from(value: TypedAppendEntriesResponse) -> Self {
            Self {
                term: value.term.0,
                success: value.success,
            }
        }
    }

    impl From<AppendEntriesRequest> for TypedAppendEntriesRequest {
        fn from(value: AppendEntriesRequest) -> Self {
            Self {
                term: Term(value.term),
                leader_id: value.leader_id,
                prev_log_index: LogIndex(value.prev_log_index),
                prev_log_term: Term(value.prev_log_term),
                entries: value.entries,
                leader_commit: LogIndex(value.leader_commit),
            }
        }
    }

    impl From<TypedAppendEntriesRequest> for AppendEntriesRequest {
        fn from(value: TypedAppendEntriesRequest) -> Self {
            Self {
                term: value.term.0,
                leader_id: value.leader_id,
                prev_log_index: value.prev_log_index.0,
                prev_log_term: value.prev_log_term.0,
                entries: value.entries,
                leader_commit: value.leader_commit.0,
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
    pub struct Term(u64);

    impl valuable::Valuable for Term {
        fn as_value(&self) -> valuable::Value<'_> {
            self.0.as_value()
        }

        fn visit(&self, visit: &mut dyn valuable::Visit) {
            u64::visit(&self.0, visit)
        }
    }

    impl Term {
        pub fn zero() -> Self {
            Self(0)
        }
        pub fn incl(&self) -> Self {
            Self(self.0 + 1)
        }
    }

    impl PartialOrd for Term {
        fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
            self.0.partial_cmp(&other.0)
        }
    }

    impl Ord for Term {
        fn cmp(&self, other: &Self) -> std::cmp::Ordering {
            self.0.cmp(&other.0)
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
    pub struct LogIndex(u64);

    impl PartialOrd for LogIndex {
        fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
            self.0.partial_cmp(&other.0)
        }
    }

    impl Ord for LogIndex {
        fn cmp(&self, other: &Self) -> std::cmp::Ordering {
            self.0.cmp(&other.0)
        }
    }

    #[derive(Debug, Clone, Deserialize, Serialize)]
    pub struct Logs<T>(Vec<Log<T>>);

    impl LogIndex {
        pub fn zero() -> Self {
            Self(0)
        }

        pub fn incl(&self) -> Self {
            Self(self.0 + 1)
        }

        pub fn decl(&self) -> Self {
            if self.0 != 0 {
                Self(self.0 - 1)
            }
            else {
                Self(0)
            }
        }
    }

    impl<T> Default for Logs<T> {
        fn default() -> Self {
            Logs(Vec::default())
        }
    }

    impl<T: Clone> Logs<T> {
        pub fn get_mut(&mut self, index: LogIndex) -> Option<&mut Log<T>> {
            if index.0 == 0 {
                None
            } else {
                self.0.get_mut(index.0 as usize - 1)
            }
        }

        pub fn get(&self, index: LogIndex) -> Option<&Log<T>> {
            if index.0 == 0 {
                None
            } else {
                self.0.get(index.0 as usize - 1)
            }
        }

        pub fn last_log_index(&self) -> LogIndex {
            LogIndex(self.0.len() as u64)
        }

        pub fn last_log_term(&self) -> Term {
            self.0.last().map(|last| last.term).unwrap_or(Term::zero())
        }

        pub fn insert(&mut self, index: LogIndex, log: Log<T>) {
            self.0.insert(index.0 as usize, log)
        }

        pub fn logs_from(&self, from: LogIndex) -> Vec<Log<T>> {
            self.0[from.0 as usize - 1..].to_vec()
        }
    }
}

use itertools::Itertools;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    sync::RwLock,
    time::sleep,
};
use tonic::{Request, Response};
use tracing::{debug, error, info, trace, warn};
use uuid::Uuid;

use crate::{Error, Log, LogConsumer};

use self::{
    proto::{
        AppendEntriesRequest, AppendEntriesResponse, RequestVoteRequest, RequestVoteResponse,
        SendLogRequest, SendLogResponse,
    },
    types::{
        LogIndex, Logs, Term, TypedAppendEntriesRequest, TypedAppendEntriesResponse,
        TypedRequestVoteRequest, TypedRequestVoteResponse,
    },
};

pub mod proto {
    use tonic::include_proto;

    include_proto!("raft");
}

enum Repository {
    LocalJson(fs::File),
}

#[derive(Serialize, Deserialize)]
struct PersistentState<T> {
    current_term: Term,
    voted_for: Option<String>,
    log: Logs<T>,
}

struct PersistentStateImpl<T> {
    state: PersistentState<T>,
    repo: Repository,
}

impl<T: Serialize + DeserializeOwned> PersistentStateImpl<T> {
    pub async fn save(&mut self) -> io::Result<()> {
        match &mut self.repo {
            Repository::LocalJson(file) => {
                let json = serde_json::to_vec_pretty(&self.state)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
                file.seek(io::SeekFrom::Start(0)).await?;
                file.write_all(&json).await?;
                Ok(())
            }
        }
    }

    pub async fn from_local_json<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let path = path.as_ref();
        let server = if path.exists() {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .read(true)
                .create(false)
                .append(false)
                .open(path)
                .await?;
            let mut json = String::new();
            file.read_to_string(&mut json).await?;
            let state = serde_json::from_str(&json)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;
            Self {
                state,
                repo: Repository::LocalJson(file),
            }
        } else {
            let mut file = fs::File::create(path).await?;
            let state: PersistentState<T> = PersistentState {
                current_term: Term::zero(),
                voted_for: None,
                log: Default::default(),
            };
            let json = serde_json::to_vec_pretty(&state)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
            file.write_all(&json).await?;
            Self {
                state,
                repo: Repository::LocalJson(file),
            }
        };
        Ok(server)
    }
}

struct VolatileState {
    commit_index: LogIndex,
    #[allow(unused)]
    last_applied: LogIndex,
}

impl VolatileState {
    fn new() -> Self {
        Self {
            commit_index: LogIndex::zero(),
            last_applied: LogIndex::zero(),
        }
    }
}

struct VolatileStateOnLeader {
    next_index: RwLock<HashMap<String, LogIndex>>,
    match_index: RwLock<HashMap<String, LogIndex>>,
}

impl VolatileStateOnLeader {
    fn new<I: IntoIterator<Item = String>>(servers: I, last_log_index: LogIndex) -> Self {
        let next_index: HashMap<_, _> = servers
            .into_iter()
            .map(|server| (server, last_log_index.incl()))
            .collect();
        let match_index = next_index
            .keys()
            .map(|server| (server.clone(), LogIndex::zero().incl()))
            .collect();
        Self {
            next_index: RwLock::new(next_index),
            match_index: RwLock::new(match_index),
        }
    }
}

enum RaftState {
    Candidate,
    Leader(VolatileStateOnLeader),
    Follower,
}

pub struct Server<T, C>(Arc<Raft<T, C>>);

pub struct Raft<T, C> {
    persistent_state: RwLock<PersistentStateImpl<T>>,
    #[allow(unused)]
    consumer: C,
    volatile: RwLock<VolatileState>,
    raft_state: RwLock<RaftState>,
    my_addr: SocketAddr,
    servers: Arc<HashSet<SocketAddr>>,
    heatbeat: Duration,
    timeout: Duration,
    timeout_stamp: Arc<RwLock<Uuid>>,
}

impl<
        T: Serialize + DeserializeOwned + Send + Sync + Clone + 'static,
        C: LogConsumer<Target = T> + Send + Sync + 'static,
    > Raft<T, C>
{
    async fn am_i_leader(raft: Arc<Self>) -> bool {
        matches!(&*raft.raft_state.read().await, &RaftState::Leader(_))
    }

    async fn am_i_follower(raft: Arc<Self>) -> bool {
        matches!(&*raft.raft_state.read().await, &RaftState::Follower)
    }

    async fn am_i_candidate(raft: Arc<Self>) -> bool {
        matches!(&*raft.raft_state.read().await, &RaftState::Candidate)
    }

    // heatbeatの定期実行を行う。
    // start_electionで選挙に勝利した場合にkickされる
    // もしリーダーから外れた場合は終了
    async fn leader_process_start(raft: Arc<Self>) {
        tokio::spawn(async move {
            loop {
                let now = Instant::now();
                if !Self::am_i_leader(raft.clone()).await {
                    warn!("iam_not_leader");
                    break;
                }
                info!("iam_leader");
                Self::heatbeat(raft.clone()).await;
                let elapsed = now.elapsed();
                if elapsed < raft.heatbeat {
                    sleep(raft.heatbeat - elapsed).await;
                }
            }
        });
    }

    #[async_recursion::async_recursion]
    async fn append_entries(raft: Arc<Self>, entries: Vec<Log<T>>) {
        let leader_id = raft.my_addr.to_string();
        let leader_commit = raft.volatile.read().await.commit_index;
        let mut persistent = raft.persistent_state.write().await;
        for entry in entries {
            let insert_pos = persistent.state.log.last_log_index().incl();
            persistent.state.log.insert(insert_pos, entry);
        }
        if let Err(e) = persistent.save().await {
            warn!(e = e.to_string(), "failed_to_save_persistent");
            exit(1);
        }
        let persistent = persistent.downgrade();
        let term = persistent.state.current_term;
        let last_log_index = persistent.state.log.last_log_index();
        let (results, errors): (Vec<_>, Vec<_>) =
            futures::future::join_all(raft.servers.iter().map(|server| {
                let leader_id = leader_id.clone();
                let raft = raft.clone();
                async move {
                    if server.to_string() != leader_id {
                        let mut client =
                            proto::raft_client::RaftClient::connect(format!("http://{server}"))
                                .await?;
                        if let RaftState::Leader(state) = &*raft.raft_state.write().await {
                            let prev_log_index = {
                                state
                                    .next_index
                                    .write()
                                    .await
                                    .entry(server.to_string())
                                    .or_insert(last_log_index.incl())
                                    .decl()
                            };
                            let prev_log_term = {
                                raft.persistent_state
                                    .read()
                                    .await
                                    .state
                                    .log
                                    .get(prev_log_index)
                                    .map(|log| log.term)
                                    .unwrap_or(Term::zero())
                            };
                            let success = client
                                .append_entries(Into::<AppendEntriesRequest>::into(
                                    TypedAppendEntriesRequest {
                                        term,
                                        leader_id,
                                        leader_commit,
                                        entries: raft
                                            .persistent_state
                                            .read()
                                            .await
                                            .state
                                            .log
                                            .logs_from(prev_log_index.incl())
                                            .into_iter()
                                            .map(|log| serde_json::to_vec(&log.inner))
                                            .collect::<Result<_, _>>()?,
                                        prev_log_index,
                                        prev_log_term,
                                    },
                                ))
                                .await?
                                .into_inner()
                                .success;
                            if !success {
                                // next indexをデクリメント
                                state
                                    .next_index
                                    .write()
                                    .await
                                    .entry(server.to_string())
                                    .and_modify(|index| *index = index.decl())
                                    .or_insert(LogIndex::zero().incl());
                            } else {
                                state
                                    .next_index
                                    .write()
                                    .await
                                    .insert(server.to_string(), last_log_index);
                                state
                                    .match_index
                                    .write()
                                    .await
                                    .insert(server.to_string(), last_log_index);
                            }
                            return Ok::<_, anyhow::Error>(success);
                        }
                        Ok::<_, anyhow::Error>(true)
                    } else {
                        // nothing to do
                        Ok(true)
                    }
                }
            }))
            .await
            .into_iter()
            .partition_result();
        for e in &errors {
            info!(e = e.to_string(), "failed_to_append_entries");
        }
        if results.iter().filter(|result| **result).count() * 2 > raft.servers.len() {
            let commit_index = raft
                .persistent_state
                .read()
                .await
                .state
                .log
                .last_log_index();
            if commit_index != LogIndex::zero() {
                raft.volatile.write().await.commit_index = commit_index.decl();
            }
            else {
                raft.volatile.write().await.commit_index = LogIndex::zero();
            }
        }
        if !errors.is_empty() || results.iter().any(|result| !result) {
            Self::append_entries(raft.clone(), Vec::new()).await;
        }
    }

    async fn heatbeat(raft: Arc<Self>) {
        Self::append_entries(raft.clone(), Vec::new()).await;
    }

    async fn call_for_voting(raft: Arc<Self>) -> (usize, Vec<anyhow::Error>) {
        let persistent_state = raft.persistent_state.read().await;
        let term = persistent_state.state.current_term;
        let last_log_index = persistent_state.state.log.last_log_index();
        let last_log_term = persistent_state.state.log.last_log_term();
        drop(persistent_state);
        let (voted, errs): (Vec<_>, Vec<_>) =
            futures::future::join_all(raft.servers.iter().map(|server| {
                let me = raft.my_addr.to_string();
                let timeout = raft.timeout;
                async move {
                    if server.to_string() != me {
                        let channel = tonic::transport::channel::Channel::builder(
                            format!("http://{server}").parse().unwrap(),
                        )
                        .timeout(Duration::from_millis((timeout.as_millis() / 3) as u64))
                        .connect_timeout(Duration::from_millis((timeout.as_millis() / 3) as u64))
                        .connect()
                        .await?;
                        let mut client = proto::raft_client::RaftClient::new(channel);
                        info!(to = server.to_string(), me = me, "request_vote");
                        let res = client
                            .request_vote(Into::<RequestVoteRequest>::into(
                                TypedRequestVoteRequest {
                                    term,
                                    candidate_id: me,
                                    last_log_index,
                                    last_log_term,
                                },
                            ))
                            .await?
                            .into_inner();
                        Ok::<TypedRequestVoteResponse, anyhow::Error>(res.into())
                    } else {
                        // vote myself
                        info!(to = me, "request_vote");
                        Ok(TypedRequestVoteResponse {
                            term,
                            vote_granted: true,
                        }
                        .into())
                    }
                }
            }))
            .await
            .into_iter()
            .partition_result();
        (
            voted.into_iter().filter(|voted| voted.vote_granted).count(),
            errs,
        )
    }

    // 候補者になり選挙を開始する
    #[async_recursion::async_recursion]
    async fn start_election(raft: Arc<Self>) -> io::Result<()> {
        {
            // フォロワーからしか候補者にならない
            // リーダーに選出された場合でもtimeoutは関係なく設定されるため、
            // timeoutからのkickで再選挙しないように
            if Self::am_i_follower(raft.clone()).await {
                let mut raft_state = raft.raft_state.write().await;
                *raft_state = RaftState::Candidate;
                let mut persistent = raft.persistent_state.write().await;
                persistent.state.voted_for = Some(raft.my_addr.to_string());
                persistent.save().await?;
            }
        }
        // ランダムタイムアウト
        let actual_timeout = raft.timeout
            + Duration::from_millis(
                (raft.timeout.as_millis() as f64 * rand::random::<f64>()) as u64,
            );
        info!(
            timeout = humantime::format_duration(actual_timeout).to_string(),
            "election_timeout"
        );
        sleep(actual_timeout).await;
        // ランダムタイムアウト後も候補者なら実際に選挙を開始する
        if !Self::am_i_candidate(raft.clone()).await {
            return Ok(());
        }
        // タームを増加させ、自分自身に投票したことを記録する
        let mut persistent_state = raft.persistent_state.write().await;
        persistent_state.state.voted_for = Some(raft.my_addr.to_string());
        persistent_state.state.current_term = persistent_state.state.current_term.incl();
        persistent_state.save().await?;
        drop(persistent_state);
        let (count, errs) = Self::call_for_voting(raft.clone()).await;
        for e in errs {
            info!(err = e.to_string(), "vote_res_fail");
        }
        info!(
            count = count,
            server_count = raft.servers.len(),
            "my_election_result"
        );
        // 途中で候補者以外になっていた場合は何もしない
        // シナリオとしては選挙中に他のリーダーが選出されるなど
        if !Self::am_i_candidate(raft.clone()).await {
            return Ok(());
        }
        if count * 2 > raft.servers.len() {
            let term = { raft.persistent_state.read().await.state.current_term };
            info!(term = format!("{term:?}"), "vote_win");
            *raft.raft_state.write().await = RaftState::Leader(VolatileStateOnLeader::new(
                raft.servers.iter().map(|addr| addr.to_string()),
                raft.persistent_state
                    .read()
                    .await
                    .state
                    .log
                    .last_log_index(),
            ));
            tokio::spawn(Self::leader_process_start(raft.clone()));
        } else {
            // 一旦フォロワーに戻り次の選挙まで待つ
            let mut raft_state = raft.raft_state.write().await;
            *raft_state = RaftState::Follower;
            let mut persistent = raft.persistent_state.write().await;
            persistent.state.voted_for = None;
            persistent.save().await?;
            tokio::spawn(Self::start_election(raft.clone()));
        }
        Ok(())
    }

    async fn schedule_timeout(raft: Arc<Self>) -> Result<(), Error> {
        let local_timeout_id = Uuid::new_v4();
        *raft.timeout_stamp.write().await = local_timeout_id;
        let _timeout_checker = tokio::spawn(async move {
            sleep(raft.timeout).await;
            if *raft.timeout_stamp.read().await == local_timeout_id {
                info!("timeout");
                if let Err(e) = Self::start_election(raft).await {
                    warn!("{e}");
                }
            } else {
                trace!("timeout_invalidated");
            }
        });
        Ok(())
    }
    async fn start(raft: Arc<Self>) -> Result<(), Error> {
        Self::schedule_timeout(raft).await?;
        Ok(())
    }

    pub async fn from_local_json(
        my_addr: SocketAddr,
        path: impl AsRef<Path>,
        timeout: Duration,
        heatbeat: Duration,
        servers: HashSet<SocketAddr>,
        consumer: C,
    ) -> Result<Arc<Self>, Error> {
        let server = Self {
            consumer,
            timeout,
            heatbeat,
            my_addr,
            timeout_stamp: Arc::new(RwLock::new(Uuid::new_v4())),
            servers: Arc::new(servers),
            volatile: RwLock::new(VolatileState::new()),
            raft_state: RwLock::new(RaftState::Follower),
            persistent_state: RwLock::new(
                PersistentStateImpl::from_local_json(path)
                    .await
                    .map_err(Error::Init)?,
            ),
        };
        let server = Arc::new(server);
        Self::start(server.clone()).await?;
        Ok(server)
    }

    async fn append_entries_handler(
        raft: Arc<Self>,
        req: TypedAppendEntriesRequest,
    ) -> TypedAppendEntriesResponse {
        debug!("recv_append_entries");
        let now = Instant::now();
        let mut persistent = raft.persistent_state.write().await;
        let current_term = persistent.state.current_term;
        if req.term < current_term {
            return TypedAppendEntriesResponse {
                term: current_term,
                success: false,
            };
        }
        {
            *raft.raft_state.write().await = RaftState::Follower;
        }
        if let Err(e) = Self::schedule_timeout(raft.clone()).await {
            error!(e = e.to_string(), "failed_to_schedule_timeout");
            exit(1);
        }
        let decoded_req_entries = match req
            .entries
            .iter()
            .map(|data| {
                let inner = serde_json::from_slice(&data[..])?;
                Ok(Log {
                    inner,
                    term: req.term,
                })
            })
            .collect::<Result<Vec<Log<T>>, serde_json::Error>>()
        {
            Ok(ok) => ok,
            Err(e) => {
                error!(e = e.to_string(), "byzantine_message");
                exit(1); //TODO
            }
        };

        // エントリの整合は同じLog indexで同じtermであれば整合しているとみなす
        if req.prev_log_index == LogIndex::zero()
            || persistent
                .state
                .log
                .get(req.prev_log_index)
                .map(|log| log.term == req.prev_log_term)
                .unwrap_or(false)
        {
            // エントリが整合したので追記する
            let mut log_index = req.prev_log_index;
            for append_entry in decoded_req_entries {
                log_index = log_index.incl();
                persistent.state.log.insert(log_index, append_entry);
            }
            if let Err(e) = persistent.save().await {
                error!(e = e.to_string(), "failed_to_save_persistent");
                exit(1); // TODO
            }
        } else {
            // エントリ不整合を通知
            return TypedAppendEntriesResponse {
                term: req.term,
                success: false,
            };
        }

        let mut volatile = raft.volatile.write().await;
        if req.leader_commit > volatile.commit_index {
            volatile.commit_index = req.leader_commit.min(persistent.state.log.last_log_index());
        }
        debug!(
            elapsed = humantime::format_duration(now.elapsed()).to_string(),
            "append_entries_elapsed"
        );
        TypedAppendEntriesResponse {
            term: req.term,
            success: true,
        }
    }

    async fn request_vote_handler(
        raft: Arc<Self>,
        req: TypedRequestVoteRequest,
    ) -> TypedRequestVoteResponse {
        let now = std::time::Instant::now();
        if let Err(e) = Self::schedule_timeout(raft.clone()).await {
            error!(e = e.to_string(), "failed_to_schedule_timeout");
            exit(1);
        }
        let res = {
            let persistent = raft.persistent_state.read().await;
            if req.term < persistent.state.current_term {
                info!("refuse_vote_by_term");
                return TypedRequestVoteResponse {
                    term: req.term,
                    vote_granted: false,
                };
            };
            TypedRequestVoteResponse {
                term: req.term,
                vote_granted: persistent
                    .state
                    .voted_for
                    .as_ref()
                    .map(|candidate| candidate == &req.candidate_id)
                    .unwrap_or(true),
            }
        };
        let mut persistent = raft.persistent_state.write().await;
        persistent.state.voted_for = Some(req.candidate_id);
        if let Err(e) = persistent.save().await {
            error!(e = e.to_string(), "failed_to_save_persistent");
            exit(1); // TODO
        }

        debug!(
            elapsed = humantime::format_duration(now.elapsed()).to_string(),
            "request_vote_elapsed"
        );
        res
    }

    async fn send_log_handler(raft: Arc<Self>, logs: Vec<Vec<u8>>) {
        unimplemented!()
    }
}

impl<
        T: Serialize + DeserializeOwned + Send + Sync + Clone + 'static,
        C: LogConsumer<Target = T> + Send + Sync + 'static,
    > Server<T, C>
{
    pub async fn from_local_json(
        my_addr: SocketAddr,
        path: impl AsRef<Path>,
        timeout: Duration,
        heatbeat: Duration,
        servers: HashSet<SocketAddr>,
        consumer: C,
    ) -> Result<Self, Error> {
        Ok(Self(
            Raft::from_local_json(my_addr, path, timeout, heatbeat, servers, consumer).await?,
        ))
    }
}

#[async_trait::async_trait]
impl<
        T: Send + Sync + 'static + DeserializeOwned + Serialize + Clone,
        C: Send + Sync + 'static + LogConsumer<Target = T>,
    > proto::raft_server::Raft for Server<T, C>
{
    async fn append_entries(
        &self,
        req: Request<AppendEntriesRequest>,
    ) -> Result<Response<AppendEntriesResponse>, tonic::Status> {
        let req = req.into_inner();
        let req: TypedAppendEntriesRequest = req.into();
        Ok(Response::new(
            Raft::append_entries_handler(self.0.clone(), req)
                .await
                .into(),
        ))
    }

    async fn request_vote(
        &self,
        req: Request<RequestVoteRequest>,
    ) -> Result<Response<RequestVoteResponse>, tonic::Status> {
        let req = req.into_inner();
        let req: TypedRequestVoteRequest = req.into();
        Ok(Response::new(
            Raft::request_vote_handler(self.0.clone(), req).await.into(),
        ))
    }

    async fn send_log(
        &self,
        req: Request<SendLogRequest>,
    ) -> Result<Response<SendLogResponse>, tonic::Status> {
        let req = req.into_inner().entries;
        Raft::send_log_handler(self.0.clone(), req).await;
        Ok(Response::new(SendLogResponse {}))
    }
}
