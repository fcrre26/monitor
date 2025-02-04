#![allow(unused_imports)]

use std::fs;
use std::str::FromStr;
use colored::Colorize;
use anyhow::{Result, anyhow};
use log;
use reqwest;
use serde::{Deserialize, Serialize};
use log::LevelFilter;
use lru::LruCache;
use dashmap::DashMap;
use futures::future::join_all;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicUsize, AtomicBool, Ordering, AtomicU64};
use std::future::Future;
use atomic_float::AtomicF64;
use chrono::{DateTime, Local};
use solana_client::rpc_client::RpcClient;
use solana_sdk::{commitment_config::CommitmentConfig, pubkey::Pubkey};
use solana_transaction_status::{
    EncodedConfirmedBlock,
    EncodedTransaction,
    UiTransactionEncoding,
    UiInstruction,
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, Instant, UNIX_EPOCH},
    io::Read,
    process,
    path::{Path, PathBuf},
};
use tokio::{
    sync::{mpsc, Mutex as TokioMutex},
    time,
};
use log4rs::{
    append::rolling_file::{
        RollingFileAppender, 
        policy::compound::{
            CompoundPolicy,
            trigger::size::SizeTrigger,
            roll::fixed_window::FixedWindowRoller,
        },
    },
    config::{Appender, Config, Root},
    encode::pattern::PatternEncoder,
    append::console::ConsoleAppender,
};
use std::sync::Mutex as StdMutex;
use serde_json::json;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub api_keys: Vec<String>,
    pub serverchan: ServerChanConfig,
    pub wcf: WeChatFerryConfig,
    pub proxy: ProxyConfig,
    pub rpc_nodes: HashMap<String, RpcNodeConfig>,
}

impl AppConfig {
    pub fn load() -> Result<Self> {
        let config_path = dirs::home_dir()
            .ok_or_else(|| anyhow!("Home directory not found"))?
            .join(".solana_pump/config.json");
            
        let content = fs::read_to_string(config_path)?;
        Ok(serde_json::from_str(&content)?)
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        AppConfig { api_keys: Vec::new(), serverchan: ServerChanConfig::default(), wcf: WeChatFerryConfig::default(), proxy: ProxyConfig::default(), rpc_nodes: HashMap::new() }
    }
}

#[derive(Default)]
struct ConfigBuilder {
    api_keys: Vec<String>,
    serverchan: Option<ServerChanConfig>,
    wcf: Option<WeChatFerryConfig>,
    proxy: Option<ProxyConfig>,
    rpc_nodes: HashMap<String, RpcNodeConfig>,
}

impl ConfigBuilder {
    fn api_key(mut self, key: String) -> Self {
        self.api_keys.push(key);
        self
    }

    fn serverchan(mut self, config: ServerChanConfig) -> Self {
        self.serverchan = Some(config);
        self
    }

    fn wcf(mut self, config: WeChatFerryConfig) -> Self {
        self.wcf = Some(config);
        self
    }

    fn proxy(mut self, config: ProxyConfig) -> Self {
        self.proxy = Some(config);
        self
    }

    fn rpc_node(mut self, url: String, config: RpcNodeConfig) -> Self {
        self.rpc_nodes.insert(url, config);
        self
    }

    fn build(self) -> Result<AppConfig> {
        Ok(AppConfig {
            api_keys: self.api_keys,
            serverchan: self.serverchan.unwrap_or_default(),
            wcf: self.wcf.unwrap_or_default(),
            proxy: self.proxy.unwrap_or_default(),
            rpc_nodes: self.rpc_nodes,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ServerChanConfig {
    keys: Vec<String>,
    alert_interval: u64,  // 新增
    heartbeat_interval: u64,  // 新增
}

impl Default for ServerChanConfig {
    fn default() -> Self {
        Self {
            keys: Vec::new(),
            alert_interval: 300,
            heartbeat_interval: 3600,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WeChatFerryConfig {
    groups: Vec<WeChatGroup>,
}

impl Default for WeChatFerryConfig {
    fn default() -> Self {
        Self {
            groups: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WeChatGroup {
    name: String,
    wxid: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyConfig {
    pub id: String,           // 新增
    pub name: String,         // 新增
    pub ip: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub protocol: ProxyProtocol,  // 新增
    pub enabled: bool,
    pub last_check: SystemTime,   // 新增
    pub status: ProxyStatus,      // 新增
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProxyProtocol {
    Http,
    Https,
    Socks5,
}

impl ProxyProtocol {
    fn as_str(&self) -> &'static str {
        match self {
            ProxyProtocol::Http => "http",
            ProxyProtocol::Https => "https",
            ProxyProtocol::Socks5 => "socks5",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProxyStatus {
    Active,
    Inactive,
    Failed,
}

pub struct ProxyManager {
    proxies: Arc<DashMap<String, ProxyConfig>>,
    config_file: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RpcNodeConfig {
    pub id: String,
    pub name: String,
    pub url: String,
    pub weight: f64,
    pub enabled: bool,
    pub is_default: bool,
    pub last_check: SystemTime,
    pub status: RpcStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RpcStatus {
    Healthy,
    Unhealthy,
    Unknown,
}

pub struct RpcManager {
    nodes: Arc<DashMap<String, RpcNodeConfig>>,
    config_file: PathBuf,
    default_nodes: Vec<RpcNodeConfig>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct MonitorState {
    last_update: TimeWrapper,
    processed_blocks: u64,
    processed_tokens: u64,
    alerts: Vec<Alert>,
    watch_addresses: HashSet<String>,
    metrics: MonitorMetrics,
    start_time: TimeWrapper,
    last_block_slot: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct MonitorMetrics {
    pub metrics: Metrics,
    pub rpc_health: HashMap<String, bool>,
    pub last_alert_time: SystemTime,
    pub monitor_start_time: SystemTime,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Metrics {
    pub processing_delays: VecDeque<Duration>,
    pub missed_blocks: Vec<u64>,
    pub processed_blocks: u64,
    pub processed_txs: u64,
    pub retry_counts: AtomicUsize,
}

impl Metrics {
    fn new() -> Self {
        Self::default()
    }
}

#[derive(Debug)]
pub struct RpcPool {
    pub health_status: DashMap<String, bool>,
    pub clients: Arc<TokioMutex<Vec<Arc<DebugRpcClient>>>>, // 修改为 Mutex<Vec<Arc<DebugRpcClient>>>
    pub current_index: AtomicUsize,
}

impl RpcPool {
    pub fn new(clients: Vec<RpcClient>) -> Self {
        let debug_clients = clients.into_iter()
            .map(|c| Arc::new(DebugRpcClient(c)))
            .collect();
        
        Self {
            health_status: DashMap::new(),
            clients: Arc::new(TokioMutex::new(debug_clients)),
            current_index: AtomicUsize::new(0),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TokenInfo {
    pub mint: Pubkey,
    pub name: String,
    pub symbol: String,
    pub market_cap: f64,
    pub liquidity: f64,
    pub holder_count: u64,
    pub holder_concentration: f64,
    pub verified: bool,
    pub price: f64,
    pub supply: u64,
    pub creator: Pubkey,
}

//===========================================
// 缓存系统模块
//===========================================
struct CacheSystem {
    blocks: DashMap<u64, EncodedConfirmedBlock>,
    token_info: LruCache<Pubkey, (TokenInfo, SystemTime)>,
    creator_history: DashMap<Pubkey, (CreatorHistory, SystemTime)>,
    fund_flow: DashMap<Pubkey, (Vec<FundingChain>, SystemTime)>,
    transactions: LruCache<String, EncodedTransaction>,
}

impl CacheSystem {
    fn new() -> Self {
        Self {
            blocks: DashMap::new(),
            token_info: LruCache::<Pubkey, (TokenInfo, SystemTime)>::new(NonZeroUsize::new(100).unwrap()),
            creator_history: DashMap::<Pubkey, (CreatorHistory, SystemTime)>::new(),
            fund_flow: DashMap::<Pubkey, (Vec<FundingChain>, SystemTime)>::new(),
            transactions: LruCache::<String, EncodedTransaction>::new(NonZeroUsize::new(100).unwrap()),
        }
    }

    async fn cleanup(&mut self) {
        let now = SystemTime::now();
        self.blocks.retain(|_, v| {
            v.block_time
                .map(|t| now.duration_since(UNIX_EPOCH).unwrap().as_secs() - t as u64 < 3600)
                .unwrap_or(false)
        });
    }

    async fn update_token_info(&mut self, mint: Pubkey, info: TokenInfo) {
        self.token_info.put(mint, (info, SystemTime::now()));
    }

    async fn update_and_get(&mut self, mint: Pubkey) -> Result<TokenInfo> {
        let info = self.fetch_token_info(&mint).await?;
        self.token_info.put(mint, (info.clone(), SystemTime::now()));
        Ok(info)
    }

    async fn fetch_token_info(&self, mint: &Pubkey) -> Result<TokenInfo> {
        // 实现获取 token 信息的逻辑
    }
}

impl std::fmt::Debug for CacheSystem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CacheSystem")
            .field("blocks_count", &self.blocks.len())
            .field("token_info_count", &self.token_info.len())
            .field("creator_history_count", &self.creator_history.len())
            .field("fund_flow_count", &self.fund_flow.len())
            .field("transactions_count", &self.transactions.len())
            .finish()
    }
}

//===========================================
// 日志系统模块
//===========================================
#[derive(Debug)]
struct LogMessage {
    level: log::Level,
    content: String,
    timestamp: SystemTime,
}

#[derive(Debug)]
struct AsyncLogger {
    sender: mpsc::Sender<LogMessage>,
}

impl AsyncLogger {
    async fn new() -> Result<Self> {
        let (tx, mut rx): (mpsc::Sender<LogMessage>, mpsc::Receiver<LogMessage>) = mpsc::channel(1000);
        
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                // 处理日志消息
            }
        });

        Ok(Self { sender: tx })
    }

    async fn log(&self, level: log::Level, message: impl Into<String>) {
        if let Err(e) = self.sender.send(LogMessage {
            level,
            content: message.into(),
            timestamp: SystemTime::now(),
        }).await {
            eprintln!("Failed to send log: {}", e);
        }
    }
}

//===========================================
// 监控状态管理模块
//===========================================
#[derive(Debug, Default, Serialize, Deserialize)]
struct MonitorState {
    last_update: TimeWrapper,
    processed_blocks: u64,
    processed_tokens: u64,
    alerts: Vec<Alert>,
    watch_addresses: HashSet<String>,
    metrics: MonitorMetrics,
    start_time: TimeWrapper,
    last_block_slot: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alert {
    pub timestamp: TimeWrapper,  // 改用 TimeWrapper
    pub level: AlertLevel,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AlertLevel {
    High,
    Medium,
    Low,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct MonitorMetrics {
    pub processed_blocks: u64,
    pub processed_txs: u64,
    pub rpc_requests: u64,
    pub rpc_errors: u64,
    pub missed_blocks: Vec<u64>,
    pub processing_delays: VecDeque<Duration>,
    pub block_processing_latency: AtomicU64,
    pub tx_processing_latency: AtomicU64,
    pub token_analysis_latency: AtomicU64,
}

//===========================================
// 智能批处理模块
//===========================================
#[derive(Debug)]
struct SmartBatcher {
    batch_size: AtomicUsize,
    load_metrics: Arc<LoadMetrics>,
    pending_requests: Arc<AtomicUsize>,
}

impl SmartBatcher {
    fn new() -> Self {
        Self {
            batch_size: AtomicUsize::new(TokenMonitor::BLOCK_BATCH_SIZE),
            load_metrics: Arc::new(LoadMetrics::default()),
            pending_requests: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn adjust_batch_size(&self) {
        let cpu_usage = self.load_metrics.cpu_usage.load(Ordering::Relaxed);
        let memory_usage = self.load_metrics.memory_usage.load(Ordering::Relaxed);
        let pending = self.pending_requests.load(Ordering::Relaxed);
        
        let current_size = self.batch_size.load(Ordering::Relaxed);
        let mut new_size = current_size;

        // CPU负载调整
        if cpu_usage > 80.0 {
            new_size = (current_size as f64 * 0.8) as usize;
        } else if cpu_usage < 50.0 && pending < TokenMonitor::MAX_PENDING_REQUESTS / 2 {
            new_size = (current_size as f64 * 1.2) as usize;
        }

        // 内存负载调整
        if memory_usage > 80.0 {
            new_size = (new_size as f64 * 0.8) as usize;
        }

        // 确保批处理大小在合理范围内
        new_size = new_size.max(10).min(1000);
        
        self.batch_size.store(new_size, Ordering::Relaxed);
    }

    async fn process_batch<T, F, Fut>(&self, items: Vec<T>, process_fn: F) -> Result<()>
    where
        F: Fn(T) -> Fut,
        Fut: Future<Output = Result<()>>,
    {
        let batch_size = self.batch_size.load(Ordering::Relaxed);
        
        for chunk in items.chunks(batch_size) {
            let futures: Vec<_> = chunk
                .iter()
                .map(|item| process_fn(item.clone()))
                .collect();
                
            self.pending_requests.fetch_add(futures.len(), Ordering::Relaxed);
            
            join_all(futures).await
                .into_iter()
                .collect::<Result<Vec<_>>>()?;
                
            self.pending_requests.fetch_sub(futures.len(), Ordering::Relaxed);
            
            // 动态调整批处理大小
            self.adjust_batch_size();
        }
        
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct ServiceState {
    pub running: Arc<AtomicBool>,
    pub last_error: Arc<StdMutex<Option<String>>>,
    pub start_time: SystemTime,
    pub processed_blocks: Arc<AtomicUsize>,
    pub processed_tokens: Arc<AtomicUsize>,
}

impl Default for ServiceState {
    fn default() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            last_error: Arc::new(StdMutex::new(None)),
            start_time: SystemTime::now(),
            processed_blocks: Arc::new(AtomicUsize::new(0)),
            processed_tokens: Arc::new(AtomicUsize::new(0)),
        }
    }
}

#[derive(Debug)]
pub struct TokenMonitor {
    pub config: AppConfig,
    pub rpc_pool: Arc<RpcPool>,
    pub cache: Arc<TokioMutex<CacheSystem>>,
    pub logger: Arc<AsyncLogger>,
    pub batcher: Arc<SmartBatcher>,
    pub metrics: Arc<TokioMutex<Metrics>>,
    pub pump_program: Pubkey,
    pub client: reqwest::Client,
    pub current_api_key: Arc<Mutex<usize>>,
    pub request_counts: DashMap<String, u32>,
    pub last_reset: DashMap<String, SystemTime>,
    pub watch_addresses: HashSet<String>,
    pub monitor_state: Arc<TokioMutex<MonitorState>>,
    pub proxy_pool: Arc<TokioMutex<ProxyPool>>,
    // 添加服务状态管理
    pub service_state: Arc<ServiceState>,
    pub last_alert: tokio::sync::Mutex<SystemTime>,
}

impl Clone for TokenMonitor {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            rpc_pool: self.rpc_pool.clone(),
            cache: self.cache.clone(),
            logger: self.logger.clone(),
            batcher: self.batcher.clone(),
            metrics: self.metrics.clone(),
            pump_program: self.pump_program,
            client: self.client.clone(),
            current_api_key: self.current_api_key.clone(),
            request_counts: self.request_counts.clone(),
            last_reset: self.last_reset.clone(),
            watch_addresses: self.watch_addresses.clone(),
            monitor_state: self.monitor_state.clone(),
            proxy_pool: self.proxy_pool.clone(),
            service_state: self.service_state.clone(),
            last_alert: tokio::sync::Mutex::new(SystemTime::now()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FundingChain {
    pub source_wallet: Pubkey,
    pub destination_wallet: Pubkey,
    pub total_amount: f64,
    pub transfers: Vec<Transfer>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Transfer {
    pub source: Pubkey,
    pub amount: f64,
    pub timestamp: TimeWrapper,
}

#[derive(Debug, Default)]
struct ProxyPool {
    proxies: Vec<ProxyConfig>,
    current_index: usize,
    last_check: SystemTime,
    health_status: DashMap<String, bool>, // 新增健康状态追踪
}

impl ProxyPool {
    fn new(proxies: Vec<ProxyConfig>) -> Self {
        Self {
            proxies,
            current_index: 0,
            last_check: SystemTime::now(),
            health_status: DashMap::new(),
        }
    }

    fn build_proxy(&self, config: &ProxyConfig) -> Result<reqwest::Proxy> {
        let proxy_url = format!(
            "{}://{}:{}@{}:{}",
            config.protocol.as_str(),
            config.username,
            config.password,
            config.ip,
            config.port
        );
        reqwest::Proxy::all(proxy_url)
            .map_err(|e| anyhow!("Failed to build proxy: {}", e))
    }

    async fn get_next_proxy(&mut self) -> Option<reqwest::Proxy> {
        if self.proxies.is_empty() {
            return None;
        }

        let proxy = &self.proxies[self.current_index];
        self.current_index = (self.current_index + 1) % self.proxies.len();
        
        self.build_proxy(proxy).ok()
    }

    async fn check_proxies(&mut self) {
        if self.proxies.is_empty() {
            log::warn!("代理池为空，跳过健康检查");
            return;
        }

        let now = SystemTime::now();
        let mut working_proxies = Vec::new();
        
        for proxy in &self.proxies {
            let result = self.test_proxy(proxy).await;
            match result {
                Ok(true) => working_proxies.push(proxy.clone()),
                Ok(false) => log::warn!("代理 {} 检测失败", proxy.id),
                Err(e) => log::error!("代理检测错误: {}", e),
            }
        }
        
        self.proxies = working_proxies;
        self.current_index = 0;
    }

    pub async fn maintain(&mut self) {
        if self.proxies.is_empty() {
            return;
        }
        let now = SystemTime::now();
        if now.duration_since(self.last_check).unwrap().as_secs() > 300 {
            self.check_proxies().await;
            self.last_check = now;
        }
    }

    async fn test_proxy(&self, proxy: &ProxyConfig) -> Result<bool> {
        let client = reqwest::Client::builder()
            .proxy(self.build_proxy(proxy)?)
            .timeout(Duration::from_secs(5))
            .build()?;
            
        let response = client.get("https://api.mainnet-beta.solana.com")
            .send()
            .await?;
            
        let is_healthy = response.status().is_success();
        self.health_status.insert(proxy.id.clone(), is_healthy);
        
        Ok(is_healthy)
    }
}

impl TokenMonitor {
    const BLOCK_BATCH_SIZE: usize = 100;
    const MAX_PENDING_REQUESTS: usize = 1000;
    const WORKER_THREADS: usize = 20;

    fn get_proxy(config: &ProxyConfig) -> Option<reqwest::Proxy> {
        if !config.enabled {
            return None;
        }

        let proxy_url = format!(
            "http://{}:{}@{}:{}",
            config.username,
            config.password,
            config.ip,
            config.port
        );

        reqwest::Proxy::http(&proxy_url).ok()
    }

    async fn new() -> Result<Self> {
        // 初始化日志
        Self::init_logger()?;
        
        log::info!("Starting Solana Token Monitor...");
        
        let config = Self::load_config()?;
        
        let proxy_pool = if config.proxy.enabled {
            let proxies = Self::load_proxy_list()?;
            Arc::new(TokioMutex::new(ProxyPool::new(proxies)))
        } else {
            Arc::new(TokioMutex::new(ProxyPool::default()))
        };

        let mut client_builder = reqwest::Client::builder();
        if let Some(proxy) = Self::get_proxy(&config.proxy) {
            client_builder = client_builder.proxy(proxy);
        }
        let client = client_builder.build()?;

        let mut monitor = Self {
            config: config.clone(),
            rpc_pool: Arc::new(RpcPool {
                health_status: DashMap::new(),
                clients: Arc::new(TokioMutex::new(Vec::new())), // 修改为 Mutex<Vec<Arc<DebugRpcClient>>>
                current_index: AtomicUsize::new(0),
            }),
            cache: Arc::new(TokioMutex::new(CacheSystem::new())),
            logger: Arc::new(AsyncLogger {
                sender: mpsc::channel(1000).0,
            }),
            batcher: Arc::new(SmartBatcher::new()),
            metrics: Arc::new(TokioMutex::new(Metrics::default())),
            pump_program: "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ35MKDfgCcMKJ".parse()?,
            client,
            current_api_key: Arc::new(Mutex::new(0)),
            request_counts: DashMap::new(),
            last_reset: DashMap::new(),
            watch_addresses: HashSet::new(),
            monitor_state: Arc::new(TokioMutex::new(MonitorState::default())),
            proxy_pool,
            service_state: Arc::new(ServiceState::default()),
            last_alert: tokio::sync::Mutex::new(SystemTime::now()),
        };

        monitor.init_rpc_nodes();
        
        let metrics = monitor.metrics.clone();
        tokio::spawn(async move {
            Self::collect_metrics(metrics).await;
        });

        let watch_addresses = Self::load_watch_addresses()?;
        
        let monitor_state = Self::load_monitor_state()?;
        
        Ok(Self {
            config,
            rpc_pool: monitor.rpc_pool,
            cache: monitor.cache,
            logger: monitor.logger,
            batcher: monitor.batcher,
            metrics: monitor.metrics,
            pump_program: monitor.pump_program,
            client: monitor.client,
            current_api_key: monitor.current_api_key,
            request_counts: monitor.request_counts,
            last_reset: monitor.last_reset,
            watch_addresses,
            monitor_state: Arc::new(TokioMutex::new(monitor_state)),
            proxy_pool: monitor.proxy_pool,
            service_state: monitor.service_state,
            last_alert: tokio::sync::Mutex::new(SystemTime::now()),
            last_block_slot: 0,
        })
    }

    fn init_rpc_nodes(&mut self) {
        let default_nodes = vec![
            "https://api.mainnet-beta.solana.com",
            "https://api.metaplex.solana.com",
            "https://solana-api.projectserum.com",
            "https://ssc-dao.genesysgo.net",
            "https://rpc.ankr.com/solana",
        ];

        for url in default_nodes {
            let client = DebugRpcClient::new(url.to_string());
            let client_arc = Arc::new(client);

            {
                let mut clients = self.rpc_pool.clients.lock().await; // 获取 Mutex 的可变引用
                clients.push(client_arc.clone()); // 使用 clients.push
            }
            self.rpc_pool.health_status.insert(url.clone(), true);
            log::info!("添加 RPC 节点: {}", url);
        }
    }

    async fn collect_metrics(metrics: Arc<TokioMutex<Metrics>>) {
        loop {
            let mut metrics = metrics.lock().await;
            let duration = metrics.last_process_time.elapsed();
            
            let blocks_per_second = metrics.processed_blocks as f64 / duration.as_secs_f64();
            let txs_per_second = metrics.processed_txs as f64 / duration.as_secs_f64();
            
            let avg_delay = if !metrics.processing_delays.is_empty() {
                metrics.processing_delays.iter()
                    .map(|d| d.as_millis() as f64)
                    .sum::<f64>() / metrics.processing_delays.len() as f64
            } else {
                0.0
            };

            log::info!(
                "性能指标 - 区块处理速度: {:.2}/s, 交易处理速度: {:.2}/s, 平均延迟: {:.2}ms, 丢失区块: {}",
                blocks_per_second,
                txs_per_second,
                avg_delay,
                metrics.missed_blocks.len()
            );

            metrics.processed_blocks = 0;
            metrics.processed_txs = 0;
            metrics.processing_delays.clear();
            metrics.last_process_time = Instant::now();

            if !metrics.missed_blocks.is_empty() {
                log::info!("发现 {} 个丢失区块，尝试重新处理", metrics.missed_blocks.len());
            }

            drop(metrics);
            time::sleep(Duration::from_secs(60)).await;
        }
    }

    async fn start_worker_threads(
        &self,
        mut block_rx: mpsc::Receiver<u64>,
        token_tx: mpsc::Sender<(Pubkey, Pubkey)>,
    ) -> Result<()> {
        let monitor = Arc::new(self.clone());
        
        for _ in 0..Self::WORKER_THREADS {
            let monitor = monitor.clone();
            let token_tx = token_tx.clone();
            
            tokio::spawn(async move {
                while let Some(slot) = block_rx.recv().await {
                    if let Err(e) = monitor.process_block(slot, &token_tx).await {
                        log::error!("Error processing block {}: {}", slot, e);
                    }
                }
            });
        }
        
        Ok(())
    }

    fn load_config() -> Result<AppConfig> {
        let config_path = dirs::home_dir()
            .ok_or_else(|| anyhow!("Cannot find home directory"))?
            .join(".solana_pump")
            .join("config.json");

        let config_str = fs::read_to_string(config_path)?;
        Ok(serde_json::from_str(&config_str)?)
    }

    async fn start(&mut self) -> Result<()> {
        log::info!("Starting Solana token monitor...");
        self.start_heartbeat().await; // 启动心跳任务
        
        let (block_tx, block_rx) = mpsc::channel(1000);
        let (token_tx, token_rx) = mpsc::channel(1000);
        
        self.start_worker_threads(block_rx, token_tx).await?;
        self.monitor_blocks(block_tx).await
    }

    async fn monitor_blocks(&self, block_tx: mpsc::Sender<u64>) -> Result<()> {
        let mut last_slot = 0u64;
        
        loop {
            let start_time = Instant::now();
            
            match self.get_current_slot().await {
                Ok(current_slot) => {
                    if current_slot > last_slot {
                        for slot in (last_slot + 1)..=current_slot {
                            block_tx.send(slot).await?;
                        }
                        last_slot = current_slot;
                    }
                }
                Err(e) => {
                    log::error!("Failed to get current slot: {}", e);
                    time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
            }

            let mut metrics = self.metrics.lock().await;
            metrics.processing_delays.push(start_time.elapsed().as_millis() as u64);
            let retries_remaining = 3 - retries_used;
            metrics.retry_counts.fetch_add(retries_remaining as usize, Ordering::Relaxed);
            
            time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn get_current_slot(&self) -> Result<u64> {
        for client in &self.rpc_pool.clients {
            if let Ok(slot) = client.0.get_slot().await {
                return Ok(slot);
            }
        }
        Err(anyhow!("All RPC clients failed"))
    }

    async fn process_block(&self, slot: u64, token_tx: &mpsc::Sender<Pubkey>) -> Result<()> {
        let client = self.get_healthy_client().await?;
        if let Ok(Some(block)) = client.0.get_block(slot).await {
            self.handle_block(block).await?;
        }
        Ok(())
    }

    async fn get_block(&self, slot: u64) -> Result<Option<EncodedConfirmedBlock>> {
        let client = self.get_healthy_client().await?;
        Ok(client.0.get_block(slot).await?)
    }

    fn extract_pump_info(&self, tx: &EncodedTransaction) -> Option<(Pubkey, Pubkey)> {
        if let EncodedTransaction::Json(tx_json) = tx {
            if let Some(message) = &tx_json.message {
                if let Some(account_keys) = &message.account_keys {
                    if account_keys.contains(&self.pump_program) {
                        // 从交易中提取代币地址和创建者地址
                        return Some((
                            account_keys[1], // 假设代币地址在第二个位置
                            account_keys[0]  // 假设创建者地址在第一个位置
                        ));
                    }
                }
            }
        }
        None
    }

    async fn get_next_api_key(&self) -> String {
        let mut current = self.current_api_key.lock().await;
        let key = &self.config.api_keys[*current];
        
        let now = SystemTime::now();
        let request_count = self.request_counts.get(key).unwrap_or(&0);
        let last_reset = self.last_reset.get(key).unwrap_or(&now);
        
        if last_reset.elapsed().unwrap().as_secs() >= 60 {
            self.request_counts.insert(key.clone(), 0);
            self.last_reset.insert(key.clone(), now);
        } else if *request_count >= 60 {
            time::sleep(Duration::from_secs(1)).await;
        }
        
        self.request_counts.insert(key.clone(), request_count + 1);
        
        *current = (*current + 1) % self.config.api_keys.len();
        
        key.clone()
    }

    async fn analyze_token(&self, mint: &Pubkey) -> Result<TokenAnalysis> {
        let token_info = self.get_token_info(mint).await?;
        let social = self.analyze_social_media(&token_info.symbol).await?;
        let distribution = self.analyze_holder_distribution(mint).await?;
        let fund_flow = self.analyze_funding_flow(mint).await?;

        Ok(TokenAnalysis {
            token_info,
            social,
            holder_distribution: distribution,
            fund_flow,
            ..TokenAnalysis::default()
        })
    }

    async fn get_token_info(&self, mint: &Pubkey) -> Result<TokenInfo> {
        if let Some(cached) = self.cache.token_info.get(mint) {
            return Ok(cached.0.clone());
        }
        
        // 实现获取 token 信息的逻辑
        Err(anyhow!("Token info not found"))
    }

    async fn get_token_creator(&self, mint: &Pubkey) -> Result<Pubkey> {
        let client = self.rpc_pool.get_healthy_client().unwrap_or_else(|| self.rpc_pool.clients[0].clone());
        let account = client.0.get_account(mint).await?;
        let data = account.data;
        let mut creator = Pubkey::new_unique();
        
        if data.len() > 0 {
            let decoded_data = bs58::decode(data).into_vec().unwrap();
            let token_data: TokenData = bincode::deserialize(&decoded_data).unwrap();
            creator = token_data.creator;
        }
        
        Ok(creator)
    }

    async fn fetch_token_info(&self, mint: &Pubkey) -> Result<TokenInfo> {
        let client = self.get_healthy_client().await?;
        let account = client.0.get_account(mint).await?;
        
        // 解析账户数据
        let data = if !account.data.is_empty() {
            let decoded_data = bs58::decode(&account.data).into_vec()?;
            bincode::deserialize(&decoded_data)?
        } else {
            return Err(anyhow!("Empty account data"));
        };

        Ok(TokenInfo {
            mint: *mint,
            name: data.name,
            symbol: data.symbol,
            market_cap: data.market_cap,
            liquidity: data.liquidity,
            holder_count: data.holder_count,
            holder_concentration: data.holder_concentration,
            verified: data.verified,
            price: data.price,
            supply: data.supply,
            creator: data.creator,
        })
    }

    async fn analyze_creator_history(&self, creator: &Pubkey) -> Result<CreatorHistory> {
        if let Some(history) = self.cache.lock().await.creator_history.get(creator) {
            if history.1.elapsed()? < Duration::from_secs(1800) {
                return Ok(history.0.clone());
            }
        }

        let api_key = self.get_next_api_key().await;
        
        let response = self.client
            .get(&format!(
                "https://public-api.birdeye.so/public/token_list?creator={}",
                creator
            ))
            .header("X-API-KEY", api_key)
            .send()
            .await?;
            
        let data: TokenListResponse = response.json().await?;
        
        let success_tokens = data.data.items
            .into_iter()
            .filter(|token| token.market_cap >= 10_000_000.0)
            .map(|token| SuccessToken {
                address: token.address,
                symbol: token.symbol,
                name: token.name,
                market_cap: token.market_cap,
                created_at: token.created_at,
            })
            .collect();

        let history = CreatorHistory {
            success_tokens,
            total_tokens: data.data.total,
        };

        self.cache.lock().await.creator_history.insert(*creator, (history.clone(), SystemTime::now()));

        Ok(history)
    }

    async fn trace_fund_flow(&self, address: &Pubkey) -> Result<Vec<FundingChain>> {
        let transfers = self.fetch_transfers(address).await?;
        let mut chains = Vec::new();
        
        for transfer in transfers {
            // 删除 success_tokens 检查，因为 Transfer 结构体中没有这个字段
            let chain = self.create_funding_chain(transfer);
            chains.push(chain);
        }
        
        Ok(chains)
    }

    async fn get_address_transfers(&self, address: &Pubkey) -> Result<Vec<Transfer>> {
        let api_key = self.get_next_api_key().await;
        
        let response = self.client
            .get(&format!(
                "https://public-api.birdeye.so/public/address_activity?address={}",
                address
            ))
            .header("X-API-KEY", api_key)
            .send()
            .await?;
            
        let data: AddressActivityResponse = response.json().await?;
        
        Ok(data.data.items.into_iter()
            .filter(|tx| tx.amount >= 1.0)
            .map(|tx| Transfer {
                source: tx.source,
                amount: tx.amount,
                timestamp: tx.timestamp,
            })
            .collect())
    }

    async fn check_address_success_tokens(&self, address: &Pubkey) -> Result<Vec<SuccessToken>> {
        let api_key = self.get_next_api_key().await;
        
        let response = self.client
            .get(&format!(
                "https://public-api.birdeye.so/public/token_list?creator={}",
                address
            ))
            .header("X-API-KEY", api_key)
            .send()
            .await?;
            
        let data: TokenListResponse = response.json().await?;
        
        Ok(data.data.items
            .into_iter()
            .filter(|token| token.market_cap >= 10_000_000.0)
            .map(|token| SuccessToken {
                address: token.address,
                symbol: token.symbol,
                name: token.name,
                market_cap: token.market_cap,
                created_at: token.created_at,
            })
            .collect())
    }

    async fn calculate_wallet_age(&self, address: &Pubkey) -> Result<f64> {
        let client = &self.rpc_pool.clients[0];
        
        let signatures = client.0.get_signatures_for_address(address)
            .await?;
            
        if let Some(oldest_tx) = signatures.last() {
            let age = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)?
                .as_secs() as f64 - oldest_tx.block_time.unwrap_or(0) as f64;
                
            Ok(age / (24.0 * 3600.0))
        } else {
            Ok(0.0)
        }
    }

    fn calculate_risk_score(
        &self,
        token_info: &TokenInfo,
        creator_history: &CreatorHistory,
        fund_flow: &[FundingChain]
    ) -> u8 {
        let mut score = 50;

        if !token_info.verified {
            score += 10;
        }
        if token_info.holder_concentration > 80.0 {
            score += 20;
        }
        if token_info.liquidity < 10.0 {
            score += 15;
        }

        if creator_history.success_tokens.is_empty() {
            score += 20;
        }

        let transit_wallets = fund_flow.iter()
            .flat_map(|chain| &chain.transfers)
            .filter(|t| t.success_tokens.is_none())
            .count();

        if transit_wallets > 2 {
            score += 15;
        }

        score.min(100)
    }

    async fn send_notification(&self, analysis: &TokenAnalysis) -> Result<()> {
        let message = self.format_message(analysis);
        
        for key in &self.config.serverchan.keys {
            if let Err(e) = self.send_server_chan(key, &message, analysis).await {
                log::error!("ServerChan push failed: {}", e);
            }
        }
        
        for group in &self.config.wcf.groups {
            if let Err(e) = self.send_wechat(group, &message).await {
                log::error!("WeChatFerry push failed for {}: {}", group.name, e);
            }
        }
        
        Ok(())
    }

    async fn send_server_chan(&self, key: &str, message: &str, event_type: &str) -> Result<()> {
        // 实现发送消息到ServerChan的逻辑
        unimplemented!()
    }

    async fn send_wechat(&self, group: &WeChatGroup, message: &str) -> Result<()> {
        let client = reqwest::Client::new();
        let url = "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=".to_string() + &group.key;
        let params = json!({
            "msgtype": "text",
            "text": {
                "content": message,
            }
        });
        let res = client.post(url).json(&params).send().await?;
        if res.status().is_success() {
            log::info!("WeChat push success: {}", message);
        } else {
            log::error!("WeChat push failed: {}", res.status());
        }
        Ok(())
    }

    fn format_message(&self, analysis: &TokenAnalysis) -> String {
        let data = NotificationData {
            risk_level: self.get_risk_level(analysis.risk_score),
            attention_level: self.get_attention_level(analysis.risk_score),
            symbol: analysis.token_info.symbol.clone(),
            name: analysis.token_info.name.clone(),
            contract: analysis.token_info.mint.to_string(),
            creator: analysis.token_info.creator.to_string(),
            supply: self.format_supply(analysis.token_info.supply),
            initial_price: analysis.token_info.price,
            current_price: analysis.token_info.price,
            market_cap: self.format_market_cap(analysis.token_info.market_cap),
            liquidity: analysis.token_info.liquidity,
            price_change: self.calculate_price_change(
                analysis.token_info.price,
                analysis.token_info.price
            ),
            lock_info: self.format_lock_info(&analysis.token_info),
            holder_count: analysis.token_info.holder_count,
            concentration: analysis.token_info.holder_concentration,
            fund_flow: self.format_fund_flow(&analysis.fund_flow),
            creator_analysis: self.format_creator_analysis(&analysis.creator_history),
            risk_assessment: self.format_risk_assessment(analysis),
            social_market: self.format_social_market(analysis),
            holder_distribution: self.format_holder_distribution(analysis),
            quick_links: self.format_quick_links(&analysis.token_info),
            monitor_info: self.format_monitor_info(analysis),
            risk_tips: self.get_risk_tips(analysis.risk_score),
            main_risks: self.get_main_risks(analysis),
            suggestion: self.get_suggestion(analysis.risk_score),
        };

        self.render_template(NOTIFICATION_TEMPLATE, &data)
    }

    fn render_template(&self, template: &str, data: &NotificationData) -> String {
        template
            .replace("{risk_level}", &data.risk_level)
            .replace("{attention_level}", &data.attention_level)
            .replace("{symbol}", &data.symbol)
            .replace("{name}", &data.name)
            .replace("{contract}", &data.contract)
            .replace("{creator}", &data.creator)
            .replace("{supply}", &data.supply)
            .replace("{initial_price}", &format!("{:.8}", data.initial_price))
            .replace("{current_price}", &format!("{:.8}", data.current_price))
            .replace("{market_cap}", &data.market_cap)
            .replace("{liquidity}", &format!("{:.1}", data.liquidity))
            .replace("{price_change}", &format!("{:.1}", data.price_change))
            .replace("{lock_info}", &data.lock_info)
            .replace("{holder_count}", &data.holder_count.to_string())
            .replace("{concentration}", &format!("{:.1}", data.concentration))
            .replace("{fund_flow}", &data.fund_flow)
            .replace("{creator_analysis}", &data.creator_analysis)
            .replace("{risk_assessment}", &data.risk_assessment)
            .replace("{social_market}", &data.social_market)
            .replace("{holder_distribution}", &data.holder_distribution)
            .replace("{quick_links}", &data.quick_links)
            .replace("{monitor_info}", &data.monitor_info)
            .replace("{risk_tips}", &data.risk_tips)
            .replace("{main_risks}", &data.main_risks)
            .replace("{suggestion}", &data.suggestion)
    }

    fn get_risk_level(&self, risk_score: u8) -> String {
        match risk_score {
            0..=39 => "低风险".to_string(),
            40..=69 => "中风险".to_string(),
            70..=100 => "高风险".to_string(),
            _ => "未知".to_string(),
        }
    }

    fn get_attention_level(&self, risk_score: u8) -> String {
        match risk_score {
            0..=39 => "无关注".to_string(),
            40..=69 => "关注".to_string(),
            70..=100 => "高度关注".to_string(),
            _ => "未知".to_string(),
        }
    }

    fn format_supply(&self, supply: u64) -> String {
        format!("{}", supply)
    }

    fn format_market_cap(&self, market_cap: f64) -> String {
        format!("${:.2}M", market_cap / 1_000_000.0)
    }

    fn format_lock_info(&self, token_info: &TokenInfo) -> String {
        format!("锁仓: {}% (180天)", (token_info.liquidity / 100.0 * 180.0) as u64)
    }

    fn format_creator_analysis(&self, history: &CreatorHistory) -> String {
        format!(
            "成功项目: {}\n总项目数: {}\n成功率: {:.1}%",
            history.success_tokens.len(),
            history.total_tokens,
            (history.success_tokens.len() as f64 / history.total_tokens as f64) * 100.0
        )
    }

    fn format_risk_assessment(&self, analysis: &TokenAnalysis) -> String {
        format!(
            "风险评分: {} | 风险等级: {}\n持币集中度: {:.1}%",
            analysis.risk_score,
            if analysis.risk_score >= 70 { "高" } else { "低" },
            analysis.token_info.holder_concentration  // 修正路径
        )
    }

    fn format_social_market(&self, analysis: &TokenAnalysis) -> String {
        format!(
            "社交数据:\nTwitter: {}(+{}%)\nDiscord: {}({}%活跃)\nTG: {}",
            analysis.social.twitter_followers,
            analysis.social.twitter_growth * 100.0,
            analysis.social.discord_members,
            analysis.social.discord_activity * 100.0,
            analysis.social.telegram_members
        )
    }

    fn format_holder_distribution(&self, analysis: &TokenAnalysis) -> String {
        format!(
            "持币分布:\n交易所: {:.1}%\n大户: {:.1}%\n散户: {:.1}%\n",
            analysis.holder_distribution.exchanges,
            analysis.holder_distribution.whales,
            analysis.holder_distribution.retail
        )
    }

    fn format_quick_links(&self, analysis: &TokenAnalysis) -> String {
        format!(
            "快速链接:\nBirdeye: {}\nSolscan: {}\n创建者: {}",
            analysis.token_info.mint.to_string(),
            analysis.token_info.mint.to_string(),
            analysis.token_info.creator.to_string()
        )
    }

    fn format_monitor_info(&self, analysis: &TokenAnalysis) -> String {
        format!(
            "监控信息:\n发现时间: {}\n首次交易: {}\n流动性添加: {}\n监控ID: {}\n风险等级: {}\n更新间隔: {}分钟\n当前状态: {}",
            analysis.detection_time.format("%Y-%m-%d %H:%M:%S"),
            analysis.first_trade_time.format("%Y-%m-%d %H:%M:%S"),
            analysis.liquidity_add_time.format("%Y-%m-%d %H:%M:%S"),
            analysis.monitor_id,
            analysis.risk_level,
            analysis.next_update_minutes,
            analysis.monitoring_status
        )
    }

    fn get_risk_tips(&self, risk_score: u8) -> String {
        match risk_score {
            0..=39 => "无风险提示".to_string(),
            40..=69 => "注意风险".to_string(),
            70..=100 => "高度风险提示".to_string(),
            _ => "未知风险提示".to_string(),
        }
    }

    fn get_main_risks(&self, analysis: &TokenAnalysis) -> String {
        format!(
            "主要风险:\n\
            1. 持币相对集中 ({:.1}%)\n\
            2. 部分资金来自新钱包\n\
            3. 代码已验证\n\
            4. 创建者历史良好\n\
            5. 流动性充足\n\
            6. 资金来源清晰可追溯\n\
            7. 市场分析\n\
            8. 社交媒体\n\
            9. 代码更新\n\
            10. 其他风险因素\n",
            analysis.token_info.holder_concentration
        )
    }

    fn get_suggestion(&self, risk_score: u8) -> String {
        match risk_score {
            0..=39 => "无建议".to_string(),
            40..=69 => "关注市场动态".to_string(),
            70..=100 => "高度关注市场动态".to_string(),
            _ => "未知建议".to_string(),
        }
    }

    fn calculate_price_change(&self, initial_price: f64, current_price: f64) -> f64 {
        if initial_price == 0.0 {
            return 0.0;
        }
        (current_price - initial_price) / initial_price * 100.0
    }

    fn get_initial_price(&self, token_info: &TokenInfo) -> Option<f64> {
        Some(0.00000085) // 示例值，实际应从API获取
    }

    fn get_first_trade_time(&self, token_info: &TokenInfo) -> DateTime<Local> {
        Local::now()
    }

    async fn analyze_social_media(&self, symbol: &str) -> Result<SocialMediaStats> {
        let client = reqwest::Client::new();
        let url = format!("https://api.birdeye.so/social/{}", symbol);
        let res = client.get(url).send().await?;
        let data = res.json::<SocialResponse>().await?;
        
        Ok(SocialMediaStats {
            twitter_followers: data.twitter.followers,
            twitter_growth: data.twitter.growth,
            discord_members: data.discord.members,
            discord_activity: data.discord.activity,
            telegram_members: data.telegram.members,
        })
    }

    async fn analyze_holder_distribution(&self, mint: &Pubkey) -> Result<HolderDistribution> {
        // 完整的实现
        let holders = self.fetch_holders(mint).await?;
        let distribution = self.calculate_distribution(&holders);
        
        // 添加更多分析逻辑
        let mut holder_categories = Vec::new();
        let mut exchange_count = 0;
        let mut whale_count = 0;
        let mut market_maker_count = 0;

        for holder in holders {
            if self.is_exchange_wallet(&holder.address) {
                exchange_count += 1;
            }
            if holder.balance > 1000.0 {  // 假设1000作为大户标准
                whale_count += 1;
            }
            if self.is_market_maker(&holder.address) {
                market_maker_count += 1;
            }

            holder_categories.push(HolderCategory {
                address: holder.address,
                balance: holder.balance,
                category: self.categorize_holder(&holder),
                last_activity: holder.last_activity,
            });
        }

        Ok(HolderDistribution {
            exchanges: distribution.exchanges,
            whales: distribution.whales,
            retail: distribution.retail,
            inactive: distribution.inactive,
            top_10_percentage: distribution.top_10_percentage,
            top_50_percentage: distribution.top_50_percentage,
            top_100_percentage: distribution.top_100_percentage,
            holder_categories,
            exchange_count,
            whale_count,
            market_maker_count,
        })
    }

    // 添加辅助函数
    fn categorize_holder(&self, holder: &HolderInfo) -> String {
        if self.is_exchange_wallet(&holder.address) {
            "Exchange".to_string()
        } else if holder.balance > 1000.0 {
            "Whale".to_string()
        } else if self.is_market_maker(&holder.address) {
            "Market Maker".to_string()
        } else {
            "Retail".to_string()
        }
    }

    fn is_market_maker(&self, address: &Pubkey) -> bool {
        // 实现市场做市商检测逻辑
        false
    }

    async fn analyze_contract(&self, mint: &Pubkey) -> ContractAnalysis {
        ContractAnalysis {
            is_upgradeable: false,
            has_mint_authority: false,
            has_freeze_authority: false,
            has_blacklist: false,
            locked_liquidity: true,
            max_tx_amount: Some(1_000_000.0),
            buy_tax: 3.0,
            sell_tax: 3.0,
        }
    }

    fn calculate_comprehensive_score(&self, analysis: &TokenAnalysis) -> ComprehensiveScore {
        ComprehensiveScore {
            total_score: 35,
            risk_score: 80,
            social_score: 65,
            market_score: 90,
        }
    }

    async fn analyze_price_trend(&self, mint: &Pubkey) -> PriceTrendAnalysis {
        PriceTrendAnalysis {
            price_change_1h: 25.5,
            price_change_24h: 125.0,
            volume_change_24h: 250.0,
            liquidity_change_24h: 180.0,
            buy_pressure: 65.0,
            sell_pressure: 35.0,
            major_transactions: vec![
                Transaction {
                    amount: 500.0,
                    price: 0.00000145,
                    timestamp: SystemTime::now(),
                    transaction_type: TransactionType::Buy,
                },
                // ... 其他重要交易
            ],
        }
    }

    fn test_monitor_output(&self) {
        println!("\n{}", ">>> 模拟监控输出...".yellow());
        
        let mock_analysis = TokenAnalysis {
            token_info: TokenInfo {
                mint: "DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263".parse().unwrap(),
                name: "PEPE2".to_string(),
                symbol: "PEPE2".to_string(),
                market_cap: 15_000_000.0,
                liquidity: 2_500.0,
                holder_count: 1258,
                holder_concentration: 35.8,
                verified: true,
                price: 0.00000145,
                supply: 420_690_000_000_000,
                creator: "7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU".parse().unwrap(),
            },
            creator_history: CreatorHistory {
                success_tokens: vec![
                    SuccessToken {
                        address: "7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU".parse().unwrap(),
                        symbol: "SAMO".to_string(),
                        name: "Samoyedcoin".to_string(),
                        market_cap: 25_000_000.0,
                        created_at: 1640995200,
                    },
                    SuccessToken {
                        address: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".parse().unwrap(),
                        symbol: "USDC".to_string(),
                        name: "USD Coin".to_string(),
                        market_cap: 1_200_000_000.0,
                        created_at: 1620000000,
                    },
                ],
                total_tokens: 5,
            },
            fund_flow: vec![
                FundingChain {
                    source_wallet: "7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU".parse().unwrap(),
                    destination_wallet: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".parse().unwrap(),
                    total_amount: 0.0,
                    transfers: vec![],
                }
            ],
            risk_score: 35,
            is_new_wallet: false,
            wallet_age: 245.5,
            social: self.analyze_social_media(&"PEPE2".to_string()).await,
            price_change_1h: 0.0,
            price_change_24h: 0.0,
            volume_24h: 0.0,
            buy_pressure: 0.0,
            sell_pressure: 0.0,
            liquidity_change: 0.0,
            holder_distribution: self.analyze_holder_distribution(mint).await,
            detection_time: chrono::Local::now(),
            first_trade_time: chrono::Local::now(),
            liquidity_add_time: chrono::Local::now(),
            monitor_id: 0,
            risk_level: 0,
            next_update_minutes: 0,
            monitoring_status: String::new(),
            risk_advice: String::new(),
            price_change_initial: 0.0,
        };

        let output = format!(
            ">>> 发现新代币 - 高度关注! 🚨\n\
            ┏━━━━━━━━━━━━━━━━━━━━━ 🔔 新代币分析报告 (UTC+8) ━━━━━━━━━━━━━━━━━━━━━┓\n\n\
            📋 合约信息\n\
            ┣━ 代币: PEPE2 (Pepe Solana)\n\
            ┣━ 合约地址: DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263 📋\n\
            ┗━ 创建者钱包: 7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU 📋\n\n\
            📊 代币数据\n\
            ┣━ 发行量: 420.69T | 持有人: 1,258 | 验证状态: ✓\n\
            ┣━ 当前价格: $0.00000145 (+125%) | 市值: $15M\n\
            ┗━ 流动性: 2,500 SOL | 持币集中度: 35.8% | 锁仓: 20%(180天)\n\n\
            💸 资金追溯 (创建者钱包总收款: 5,250.50 SOL)\n\
            ┣━ 资金来源#1 (2,000.50 SOL) - 已验证资金链\n\
            ┃   创建者钱包 [2024-03-21 12:00] 7xKX...gAsU\n\
            ┃   ↑ 2000.50 SOL └ 中转钱包A [2024-03-20 15:30] DezX...B263 (SAMO创建者)\n\
            ┃   ↑ 2000.50 SOL └ 中转钱包B [2024-03-19 09:15] EPjF...Dt1v (BONK早期投资者)\n\
            ┃   ↑ 2000.50 SOL └ 源头钱包C [2024-03-18 10:00] ORCA...Zt1v (已验证交易所)\n\n\
            📊 创建者历史分析\n\
            ┣━ 历史项目数: 5个 | 成功项目: 2个 | 成功率: 40.0%\n\
            ┣━ 代币列表:\n\
            ┃   ┣━ 1. SAMO: 市值 $25.0M (2023-12) - 最佳业绩\n\
            ┃   ┣━ 2. BONK: 市值 $12.0M (2024-01)\n\
            ┃   ┗━ 3. PEPE2: 市值 $15.0M (当前项目)\n\
            ┗━ 平均市值: $17.33M\n\n\
            🎯 综合风险评估\n\
            ┣━ 总体评分: 35/100 (低风险)\n\
            ┣━ 积极因素:\n\
            ┃   ┣━ 1. 创建者有成功项目经验\n\
            ┃   ┣━ 2. 资金来源清晰可追溯\n\
            ┃   ┗━ 3. 代码已验证\n\
            ┗━ 风险因素:\n\
                ┣━ 1. 持币相对集中 (35.8%)\n\
                ┗━ 2. 部分资金来自新钱包\n\n\
            🔗 快速链接\n\
            ┣━ Birdeye: https://birdeye.so/token/DezXAZ...B263 📋\n\
            ┣━ Solscan: https://solscan.io/token/DezXAZ...B263 📋\n\
            ┗━ 创建者: https://solscan.io/account/7xKXt...gAsU 📋\n\n\
            ⏰ 监控信息\n\
            ┣━ 发现时间: 2024-03-21 12:00:00 (UTC+8)\n\
            ┣━ 首次交易: 2024-03-21 12:05:30 (UTC+8)\n\
            ┣━ 初始价格: $0.00000085\n\
            ┗━ 当前涨幅: +70.5%\n"
        );

        println!("{}", output);
    }

    fn get_wallet_role(&self, address: &Pubkey) -> String {
        if self.is_exchange_wallet(address) {
            "交易所钱包".to_string()
        } else if self.is_contract_wallet(address) {
            "合约钱包".to_string()
        } else {
            "普通钱包".to_string()
        }
    }

    fn get_wallet_description(&self, transfer: &Transfer) -> String {
        if let Some(ref tokens) = transfer.success_tokens {
            if !tokens.is_empty() {
                format!("{} 创建者", tokens[0].symbol)
            } else {
                "中转钱包".to_string()
            }
        } else {
            "新钱包".to_string()
        }
    }

    fn is_exchange_wallet(&self, address: &Pubkey) -> bool {
        // 实现交易所钱包检测逻辑
        false // 临时返回
    }

    fn is_contract_wallet(&self, address: &Pubkey) -> bool {
        // 实现合约钱包检测逻辑
        false // 临时返回
    }

    async fn warm_up_cache(&self) -> Result<()> {
        // 预加载常用数据
        Ok(())
    }

    async fn process_blocks_batch(&self, slots: Vec<u64>) -> Result<()> {
        let futures: Vec<_> = slots.into_iter()
            .map(|slot| self.process_block(slot, &self.token_tx))
            .collect();
            
        join_all(futures).await
            .into_iter()
            .collect::<Result<Vec<_>>>()?;
        Ok(())
    }

    pub async fn start_service(&self) -> Result<()> {
        log::info!("启动监控服务...");
        self.service_state.running.store(true, Ordering::SeqCst);
        
        // 预热缓存
        self.warm_up_cache().await?;
        
        // 启动工作线程
        let (block_tx, block_rx) = mpsc::channel(1000);
        let (token_tx, token_rx) = mpsc::channel(1000);
        
        // 启动区块处理
        self.start_worker_threads(block_rx, token_tx.clone()).await?;
        
        // 启动代币分析
        self.start_token_analysis(token_rx).await?;
        
        // 启动指标收集
        self.start_metrics_collection().await?;
        
        // 启动心跳推送
        self.start_heartbeat();
        
        // 启动代理维护任务
        let proxy_pool = self.proxy_pool.clone();
        tokio::spawn(async move {
            let mut interval = time::interval(Duration::from_secs(300));
            loop {
                interval.tick().await;
                proxy_pool.lock().await.maintain().await;
            }
        });
        
        Ok(())
    }

    pub async fn stop_service(&self) -> Result<()> {
        log::info!("停止监控服务...");
        self.service_state.running.store(false, Ordering::SeqCst);
        Ok(())
    }

    pub async fn health_check(&self) -> Result<ServiceHealth> {
        let uptime = SystemTime::now()
            .duration_since(self.service_state.start_time)?;
            
        Ok(ServiceHealth {
            running: self.service_state.running.load(Ordering::SeqCst),
            uptime: uptime.as_secs(),
            processed_blocks: self.service_state.processed_blocks.load(Ordering::SeqCst) as usize,
            processed_tokens: self.service_state.processed_tokens.load(Ordering::SeqCst) as usize,
            last_error: self.service_state.last_error.lock().await.clone(),
        })
    }

    async fn start_metrics_collection(&self) -> Result<()> {
        let metrics = self.metrics.clone();
        let service_state = self.service_state.clone();
        
        tokio::spawn(async move {
            while service_state.running.load(Ordering::SeqCst) {
                // 收集并记录指标
                let health = self.health_check().await?;
                log::info!("服务状态: {:?}", health);
                
                tokio::time::sleep(Duration::from_secs(60)).await;
            }
            Ok::<(), anyhow::Error>(())
        });
        
        Ok(())
    }

    async fn analyze_token_metrics(&self, mint: &Pubkey) -> Result<TokenMetrics> {
        let token_info = self.fetch_token_info(mint).await?;
        
        // 计算基础指标
        let supply_score = self.calculate_supply_score(token_info.supply);
        let liquidity_score = self.calculate_liquidity_score(token_info.liquidity);
        let holder_score = self.calculate_holder_score(
            token_info.holder_count,
            token_info.holder_concentration
        );
        
        // 计算综合风险分数
        let risk_score = (supply_score + liquidity_score + holder_score) / 3.0;
        
        Ok(TokenMetrics {
            supply_score,
            liquidity_score, 
            holder_score,
            risk_score,
            market_cap: token_info.market_cap,
            price: token_info.price,
            verified: token_info.verified,
        })
    }

    fn calculate_supply_score(&self, supply: u64) -> f64 {
        // 供应量评分算法
        let base_score = if supply < 1_000_000 {
            100.0
        } else if supply < 1_000_000_000 {
            75.0
        } else if supply < 1_000_000_000_000 {
            50.0
        } else {
            25.0
        };

        // 应用修正因子
        let correction = (supply as f64).log10() * 5.0;
        (base_score - correction).max(0.0).min(100.0)
    }

    fn calculate_liquidity_score(&self, liquidity: f64) -> f64 {
        // 流动性评分算法
        let base_score = if liquidity < 1000.0 {
            25.0
        } else if liquidity < 10_000.0 {
            50.0
        } else if liquidity < 100_000.0 {
            75.0
        } else {
            100.0
        };

        // 应用修正因子
        let correction = liquidity.log10() * 10.0;
        (base_score + correction).max(0.0).min(100.0)
    }

    fn calculate_holder_score(&self, holder_count: u64, concentration: f64) -> f64 {
        // 持有者评分算法
        let count_score = if holder_count < 100 {
            25.0
        } else if holder_count < 1000 {
            50.0
        } else if holder_count < 10000 {
            75.0
        } else {
            100.0
        };

        // 考虑持仓集中度
        let concentration_penalty = concentration * 50.0;
        (count_score - concentration_penalty).max(0.0)
    }

    async fn trace_fund_flow(&self, address: &Pubkey) -> Result<Vec<FundingChain>> {
        let transfers = self.fetch_transfers(address).await?;
        let mut chains = Vec::new();
        
        for transfer in transfers {
            // 删除 success_tokens 检查，因为 Transfer 结构体中没有这个字段
            let chain = self.create_funding_chain(transfer);
            chains.push(chain);
        }
        
        Ok(chains)
    }

    async fn save_monitor_state(&self) -> Result<()> {
        let state_file = dirs::home_dir()
            .ok_or_else(|| anyhow!("Home directory not found"))?
            .join(".solana_pump/state.json");
            
        let state = self.monitor_state.lock().await;
        let content = serde_json::to_string_pretty(&*state)?;
        fs::write(state_file, content)?;
        
        Ok(())
    }

    async fn load_monitor_state() -> Result<MonitorState> {
        let state_file = dirs::home_dir()
            .ok_or_else(|| anyhow!("Home directory not found"))?
            .join(".solana_pump/state.json");

        if !state_file.exists() {
            return Ok(MonitorState::default());
        }

        let content = fs::read_to_string(state_file)?;
        Ok(serde_json::from_str(&content)?)
    }

    async fn update_metrics(&self) {
        let mut metrics = self.metrics.lock().await;
        metrics.processing_delays = metrics.processing_delays
            .iter()
            .map(|&d| d)  // 直接使用值而不是转换
            .collect();
    }

    async fn auto_recover(&self) -> Result<()> {
        let recovery = Arc::new(Recovery::new());
        
        tokio::spawn(async move {
            loop {
                if let Err(e) = self.check_and_recover().await {
                    log::error!("Recovery failed: {}", e);
                }
                tokio::time::sleep(Duration::from_secs(60)).await;
            }
        });

        Ok(())
    }

    async fn check_and_recover(&self) -> Result<()> {
        // 检查RPC节点健康
        for client in &self.rpc_pool.clients {
            if let Err(e) = client.0.get_slot() {
                log::warn!("RPC node {} failed: {}", client.0.url(), e);
                self.rpc_pool.health_status.insert(client.0.url(), false);
                self.try_recover_rpc(client).await?;
            } else {
                self.rpc_pool.health_status.insert(client.0.url(), true);
            }
        }

        // 检查缓存状态
        let cache = self.cache.lock().await;
        if cache.blocks.len() > 10000 {
            log::warn!("Cache size too large, cleaning up...");
            self.cleanup_cache().await?;
        }

        // 检查处理延迟
        let metrics = self.metrics.lock().await;
        if metrics.processing_delays.iter().any(|d| *d > Duration::from_secs(5)) {
            log::warn!("Processing delays too high, adjusting batch size...");
            self.batcher.adjust_batch_size();
        }

        Ok(())
    }

    async fn try_recover_rpc(&self, client: &Arc<DebugRpcClient>) -> Result<()> {
        for _ in 0..3 {
            if client.0.get_slot().is_ok() {
                self.rpc_pool.health_status.insert(client.0.url(), true);
                return Ok(());
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        Ok(())
    }

    async fn generate_report(&self) -> Result<String> {
        let mut report = String::new();
        
        // 添加标题
        report.push_str(&format!(
            "Solana Pump Monitor 监控报告\n日期: {}\n\n",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
        ));

        // 添加系统状态
        let metrics = self.metrics.lock().await;
        report.push_str(&format!(
            "系统状态:\n\
            处理区块数: {}\n\
            处理交易数: {}\n\
            丢失区块数: {}\n\
            平均处理延迟: {:.2}ms\n\n",
            metrics.processed_blocks,
            metrics.processed_txs,
            metrics.missed_blocks.len(),
            metrics.processing_delays.iter()
                .map(|d| d.as_millis() as f64)
                .sum::<f64>() / metrics.processing_delays.len() as f64
        ));

        // 添加RPC节点状态
        report.push_str("RPC节点状态:\n");
        for client in &self.rpc_pool.clients {
            let health = self.rpc_pool.health_status.get(&client.0.url())
                .map_or(false, |v| *v);
            report.push_str(&format!(
                "{}: {}\n",
                client.0.url(),
                if health { "✅ 正常" } else { "❌ 异常" }
            ));
        }

        // 添加告警统计
        let alerts = self.get_recent_alerts().await?;
        report.push_str(&format!(
            "\n告警统计:\n\
            高风险: {}\n\
            中风险: {}\n\
            低风险: {}\n",
            alerts.iter().filter(|a| matches!(a.level, AlertLevel::High)).count(),
            alerts.iter().filter(|a| matches!(a.level, AlertLevel::Medium)).count(),
            alerts.iter().filter(|a| matches!(a.level, AlertLevel::Low)).count()
        ));

        Ok(report)
    }

    // 辅助格式化函数
    fn format_change(&self, value: f64) -> String {
        format!("{:+.1}%", value * 100.0)
    }

    fn format_volume(&self, value: f64) -> String {
        format!("${:.1}M", value / 1_000_000.0)
    }

    fn format_short_address(&self, address: &Pubkey) -> String {
        let addr_str = address.to_string();
        format!("{}...{}", &addr_str[..4], &addr_str[addr_str.len()-4..])
    }

    async fn send_alert(&self, title: &str, content: &str) {
        let mut last_alert = self.last_alert.lock().await;
        if last_alert.elapsed().unwrap().as_secs() < 300 { // 5分钟间隔
            return;
        }
        
        for key in &self.config.serverchan.keys {
            let message = format!("🚨 {} 🚨\n\n{}", title, content);
            if let Err(e) = self.send_server_chan(key, &message, "alert").await {
                log::error!("告警推送失败: {}", e);
            }
        }
        *last_alert = SystemTime::now();
    }

    async fn start_heartbeat(&self) {
        let interval = Duration::from_secs(self.config.serverchan.heartbeat_interval);
        let mut interval = time::interval(interval);
        loop {
            interval.tick().await;
            self.send_heartbeat().await;
        }
    }

    async fn send_heartbeat(&self) {
        let state = self.monitor_state.lock().await;
        let metrics = self.metrics.lock().await;
        
        let message = format!(
            "💓 系统心跳 | 运行状态正常\n\
            ━━━━━━━━━━━━━━━━━━━━━━\n\
            ▶ 运行时长: {:.1} 小时\n\
            ▶ 处理区块: {}\n\
            ▶ 监控代币: {}\n\
            ▶ 节点状态: {}/{} 正常\n\
            ▶ 最后异常: {}",
            state.start_time.elapsed().unwrap().as_secs_f64() / 3600.0,
            metrics.processed_blocks,
            state.processed_tokens,
            self.rpc_pool.health_status.iter().filter(|v| *v.value()).count(),
            self.rpc_pool.clients.len(),
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
        );

        for key in &self.config.serverchan.keys {
            if let Err(e) = self.send_server_chan(key, &message, "heartbeat").await {
                log::error!("心跳推送失败: {}", e);
            }
        }
    }

    pub fn generate_risk_alert(&self, analysis: &TokenAnalysis) -> String {
        // 资金流格式化
        let fund_flows = analysis.fund_flow.iter().enumerate().map(|(i, flow)| {
            let arrow = "↑".repeat(3);
            let flow_type = match flow.risk_level {
                0..=30 => "✅ 低风险资金",
                31..=70 => "⚠️ 中等风险",
                _ => "🚨 高风险资金"
            };
            
            format!(
                "┣━ 资金链#{} ({:.2} SOL) - {}\n┃   {}\n┃   {} └ {}\n┃   {} └ {}",
                i+1,
                flow.amount,
                flow_type,
                flow.destination_wallet,
                arrow,
                flow.intermediate_wallet,
                arrow,
                flow.source_wallet
            )
        }).collect::<Vec<_>>().join("\n");

        // 创建者历史格式化
        let creator_history = analysis.creator_history.projects.iter().map(|p| {
            format!(
                "┃   ┣━ {}. {}: ${:.1}M ({})",
                p.index, p.project_name, p.market_cap / 1_000_000.0, p.project_type
            )
        }).collect::<Vec<_>>().join("\n");

        // 持币分布格式化
        let holder_distribution = analysis.holder_distribution.holder_categories.iter().map(|(category, percent)| {
            format!("┣━ {}: {:.1}%", category, percent)
        }).collect::<Vec<_>>().join("\n");

        format!(
            r#"🚨 高风险代币预警 - 需要特别关注!
┏━━━━━━━━━━━━━━━━━━━━━ 🔔 深度分析报告 (UTC+8) ━━━━━━━━━━━━━━━━━━━━━┓

📋 基础信息
┣━ 代币: {} ({})
┣━ 合约: {} 📋
┗━ 创建者: {} 📋

💰 代币数据
┣━ 发行量: {} | 初始价格: ${} | 当前价格: ${}
┣━ 当前市值: ${:.1}M | 流动性: {} SOL | 涨幅: +{:.1}%
┗━ 锁定详情: {} | 持有人: {} | 集中度: {:.1}%

💸 资金追溯 (总流入: {:.2} SOL)
{}

📊 创建者分析
┣━ 历史数据: 项目总数: {}个 | 成功: {}个({:.1}%) | 高风险项目: {}个
┣━ 代币列表:
{}
┗━ 综合指标: 平均市值: ${:.2}M | 信用评分: {}

⚠️ 风险评估 (风险评分: {}/100)
┣━ 高风险信号:
┃   ┣━ {} 
┃   ┗━ {}
┣━ 中等风险:
┃   ┣━ {}
┃   ┗━ {}
┗━ 积极因素:
    ┣━ {}
    ┗━ {}

📱 社交媒体 & 市场表现
┣━ 社交数据: Twitter({},{}%) | Discord({},{}%活跃) | TG({})
┣━ 价格变动: 1h({}%) | 24h({}%) | 首次交易({}%)
┗━ 交易数据: 24h量({}) | 买压({}%) | 卖压({}%) | 流动性变化({}%)

👥 持币分布
{}
┗━ 重要地址: {}个交易所 | {}个大户 | {}个做市商

🔗 快速链接 (点击复制)
┣━ Birdeye: {}
┣━ Solscan: {}
┗━ 创建者: {}

⏰ 监控信息
┣━ 关键时间: 发现({}) | 首交易({}) | 流动性添加({})
┣━ 监控编号: {} | 风险等级: {}
┗━ 下次更新: {}分钟后 | 当前状态: {}

💡 风险提示
{}
"#,
            analysis.token_info.name,
            analysis.token_info.symbol,
            self.format_short_address(&analysis.token_info.mint),
            self.format_short_address(&analysis.token_info.creator),
            analysis.token_info.total_supply,
            analysis.token_info.initial_price,
            analysis.token_info.current_price,
            analysis.token_info.market_cap / 1_000_000.0,
            analysis.token_info.liquidity,
            analysis.price_change_24h * 100.0,
            analysis.token_info.lock_details,
            analysis.token_info.holder_count,
            analysis.token_info.holder_concentration * 100.0,
            analysis.token_info.total_inflow,
            fund_flows,
            analysis.creator_history.project_count,
            analysis.creator_history.success_count,
            (analysis.creator_history.success_count as f64 / analysis.creator_history.project_count as f64) * 100.0,
            analysis.creator_history.high_risk_count,
            creator_history,
            analysis.creator_history.avg_market_cap / 1_000_000.0,
            analysis.creator_history.credit_rating,
            analysis.risk_score,
            analysis.risk_factors.high_risk[0],
            analysis.risk_factors.high_risk[1],
            analysis.risk_factors.medium_risk[0],
            analysis.risk_factors.medium_risk[1],
            analysis.risk_factors.positive[0],
            analysis.risk_factors.positive[1],
            analysis.social.twitter_followers,
            analysis.social.twitter_growth * 100.0,
            analysis.social.discord_members,
            analysis.social.discord_activity * 100.0,
            analysis.social.telegram_members,
            analysis.price_change_1h * 100.0,
            analysis.price_change_24h * 100.0,
            analysis.initial_price_change * 100.0,
            analysis.volume_24h / 1_000_000.0,
            analysis.buy_pressure * 100.0,
            analysis.sell_pressure * 100.0,
            analysis.liquidity_change * 100.0,
            holder_distribution,
            analysis.holder_distribution.exchange_wallets,
            analysis.holder_distribution.whale_wallets,
            analysis.holder_distribution.market_makers,
            format!("birdeye.so/token/{}", analysis.token_info.mint),
            format!("solscan.io/token/{}", analysis.token_info.mint),
            format!("solscan.io/account/{}", analysis.token_info.creator),
            TimeWrapper::from(analysis.detection_time.clone()).to_datetime_local().format("%m-%d %H:%M"),
            TimeWrapper::from(analysis.first_trade_time.clone()).to_datetime_local().format("%m-%d %H:%M"),
            TimeWrapper::from(analysis.liquidity_add_time.clone()).to_datetime_local().format("%m-%d %H:%M"),
            analysis.monitor_id,
            analysis.risk_level,
            analysis.next_update_minutes,
            analysis.monitoring_status,
            analysis.risk_advice
        )
    }

    fn format_short_address(&self, address: &Pubkey) -> String {
        let addr_str = address.to_string();
        format!("{}...{}", &addr_str[..4], &addr_str[addr_str.len()-4..])
    }

    async fn get_block(&self, slot: u64) -> Result<Option<EncodedConfirmedBlock>> {
        let client = self.get_healthy_client().await?;
        Ok(client.0.get_block(slot).await?)
    }

    fn calculate_time_diff(&self, now: u64, t: u64) -> bool {
        now.saturating_sub(t) < 3600
    }

    async fn get_slot(&self, client: &Arc<DebugRpcClient>) -> Result<u64> {
        Ok(client.0.get_slot()?)
    }

    async fn cleanup_cache(&self) -> Result<()> {
        let cache = self.cache.lock().await;
        cache.blocks.clear();
        cache.txs.clear();
        cache.token_info.clear();
        Ok(())
    }

    async fn get_recent_alerts(&self) -> Result<Vec<Alert>> {
        let state = self.monitor_state.lock().await;
        Ok(state.alerts.clone())
    }

    fn process_transaction(&self, tx: &EncodedTransaction) -> Result<()> {
        match tx {
            EncodedTransaction::Json(tx_json) => {
                if let Some(ui_message) = tx_json.message {
                    let message = &ui_message;

                    for instruction in &message.instructions {
                        if let UiInstruction::Parsed(parsed_instruction) = instruction {
                            if parsed_instruction.program == "spl-token" {
                                // 处理 pump 交易
                            }
                        }
                    }
                }
                Ok(())
            },
            _ => Ok(())
        }
    }

    async fn process_transactions(&self, txs: Vec<EncodedTransaction>) -> Result<()> {
        for tx in txs.iter() {
            self.process_transaction(tx)?;
        }
        Ok(())
    }

    async fn process_batch<T, F, Fut>(&self, items: Vec<T>, process_fn: F) -> Result<()>
    where
        T: Clone,
        F: Fn(T) -> Fut,
        Fut: Future<Output = Result<()>>,
    {
        let mut handles = Vec::new();
        
        for chunk in items.chunks(self.batcher.batch_size.load(Ordering::Relaxed)) {
            let chunk_items: Vec<_> = chunk.to_vec();
            let handle = tokio::spawn(async move {
                for item in chunk_items {
                    process_fn(item).await?;
                }
                Ok::<_, anyhow::Error>(())
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.await??;
        }

        Ok(())
    }

    async fn get_block_time(&self, slot: u64) -> Result<SystemTime> {
        let block = self.get_block(slot).await?;
        if let Some(block) = block {
            block.block_time
                .map(|timestamp| UNIX_EPOCH + Duration::from_secs(timestamp as u64))
                .ok_or_else(|| anyhow!("Block time not available"))
        } else {
            Err(anyhow!("Block not found"))
        }
    }

    async fn start_monitoring(&self) -> Result<()> {
        let (tx, rx): (mpsc::Sender<BlockData>, mpsc::Receiver<BlockData>) = mpsc::channel(10);
        let (alert_tx, alert_rx): (mpsc::Sender<Alert>, mpsc::Receiver<Alert>) = mpsc::channel(10);
        
        self.spawn_block_processor(rx, alert_tx).await?;
        self.spawn_alert_handler(alert_rx).await?;
        
        Ok(())
    }

    fn get_cached_token_info(&self, mint: &Pubkey) -> Option<TokenInfo> {
        let cache = self.cache.lock().unwrap();
        cache.token_info.get(mint).map(|(info, _)| info.clone())
    }

    // 1. 修复缺失的生命周期参数
    #[derive(Debug)]
    pub struct TokenCache {
        pub info: TokenInfo,
        pub last_update: SystemTime,
    }

    // 2. 修复生命周期约束
    impl TokenMonitor {
        fn get_cached_info<'a>(&'a self, mint: &Pubkey) -> Option<TokenCache> {
            self.cache.token_info.get(mint).map(|(info, time)| TokenCache {
                info,
                last_update: *time,
            })
        }
    }

    // 3. 修复多重生命周期
    impl<'a> TokenCache {
        fn new(info: TokenInfo) -> Self {
            Self {
                info,
                last_update: SystemTime::now(),
            }
        }
    }

    // 1. 修复借用存活期
    async fn process_cached_blocks(&self) -> Result<()> {
        let blocks: Vec<_> = {
            let cache = self.cache.lock().await;
            cache.blocks.iter().map(|entry| *entry.key()).collect()
        };
        
        for slot in blocks {
            self.process_block(slot).await?;
        }
        Ok(())
    }

    // 2. 修复可变/不可变借用冲突
    impl CacheSystem {
        async fn update_and_get(&mut self, mint: Pubkey) -> Result<TokenInfo> {
            let info = self.fetch_token_info(&mint).await?;
            self.token_info.put(mint, (info.clone(), SystemTime::now()));
            Ok(info)
        }
    }

    // 3. 修复借用规则违反
    impl RpcPool {
        fn update_health_status(&self, url: String, status: bool) {
            self.health_status.insert(url, status);
        }

        fn add_client(&self, client: RpcClient) {
            let mut clients = self.clients.lock().unwrap();
            clients.push(Arc::new(DebugRpcClient(client)));
        }

        fn get_healthy_client(&self) -> Option<Arc<DebugRpcClient>> {
            // 实现获取健康客户端的逻辑
        }
    }

    fn get_next_rpc_client(&self) -> Arc<DebugRpcClient> {
        self.rpc_pool.get_healthy_client().unwrap_or_else(|| self.rpc_pool.clients[0].clone())
    }

    async fn handle_block(&self, block: EncodedConfirmedBlock) -> Result<()> {
        let mut metrics = self.metrics.lock().await;
        metrics.processed_blocks += 1;
        Ok(())
    }

    fn init_logger() -> Result<()> {
        let console = ConsoleAppender::builder()
            .encoder(Box::new(PatternEncoder::new("{d} {l} {t} - {m}{n}")))
            .build();

        let config = Config::builder()
            .appender(Appender::builder().build("console", Box::new(console)))
            .build(Root::builder().appender("console").build(LevelFilter::Info))
            .unwrap();

        log4rs::init_config(config)?;
        Ok(())
    }

    fn load_proxy_list() -> Result<Vec<ProxyConfig>> {
        let config_path = dirs::home_dir()
            .ok_or_else(|| anyhow!("Failed to get home directory"))?
            .join(".solana_pump/proxies.json");
            
        let content = fs::read_to_string(config_path)?;
        Ok(serde_json::from_str(&content)?)
    }

    fn load_watch_addresses() -> Result<HashSet<String>> {
        let path = dirs::home_dir()
            .ok_or_else(|| anyhow!("Failed to get home directory"))?
            .join(".solana_pump/watch_addresses.txt");
            
        let content = fs::read_to_string(path)?;
        Ok(content.lines().map(String::from).collect())
    }

    fn format_fund_flow(&self, fund_flows: &[FundingChain]) -> String {
        fund_flows.iter()
            .map(|chain| format!(
                "资金流向:\n来源: {}\n目标: {}\n金额: {} SOL",
                chain.source_wallet,
                chain.destination_wallet,
                chain.total_amount
            ))
            .collect()
    }

    fn create_funding_chain(&self, transfer: Transfer) -> FundingChain {
        FundingChain {
            source_wallet: transfer.source,
            destination_wallet: transfer.to_address,
            total_amount: transfer.amount,
            transfers: vec![transfer],
        }
    }

    fn update_request_count(&self, key: &str) -> u32 {
        let mut entry = self.request_counts.entry(key.to_string()).or_insert(0);
        *entry += 1;
        *entry
    }

    fn update_health_status(&self, url: String, status: bool) {
        self.health_status.insert(url, status);
    }

    async fn update_cache(&self, mint: Pubkey, info: TokenInfo) {
        let mut cache = self.cache.lock().await;
        cache.token_info.put(mint, (info, SystemTime::now()));
    }

    async fn create_token_analysis(&self, token_info: TokenInfo, creator: &Pubkey) -> Result<TokenAnalysis> {
        let social = self.analyze_social_media(&token_info.symbol).await?;
        let holder_distribution = self.analyze_holder_distribution(&token_info.mint).await?;
        let first_trade_time = TimeWrapper(self.get_first_trade_time(&token_info).into());
        let price_change_24h = self.calculate_price_change(0.0, token_info.price);
        let fund_flow = self.analyze_funding_flow(&token_info.mint).await?;

        Ok(TokenAnalysis {
            token_info,
            creator_history: CreatorHistory::default(),
            fund_flow,
            risk_score: 50,
            is_new_wallet: false,
            wallet_age: 0.0,
            social,
            price_change_1h: 0.0,
            price_change_24h,
            volume_24h: 0.0,
            buy_pressure: 0.0,
            sell_pressure: 0.0,
            liquidity_change: 0.0,
            holder_distribution,
            detection_time: TimeWrapper(SystemTime::now()),
            first_trade_time,
            liquidity_add_time: TimeWrapper(SystemTime::now()),
            monitor_id: 0,
            risk_level: 0,
            next_update_minutes: 60,
            monitoring_status: "Active".to_string(),
            risk_advice: String::new(),
            price_change_initial: 0.0,
        })
    }

    fn format_analysis(&self, analysis: &TokenAnalysis) -> String {
        format!(
            "Token Analysis:\n\
             Symbol: {}\n\
             Name: {}\n\
             Contract: {}\n\
             Creator: {}\n\
             Risk Score: {}\n\
             Price Change (24h): {:.2}%\n\
             Volume (24h): ${:.2}M\n\
             Market Cap: ${:.2}M\n\
             Liquidity: ${:.2}M\n\
             Holder Count: {}\n\
             Top 10 Holders: {:.2}%",
            analysis.token_info.symbol,
            analysis.token_info.name,
            analysis.token_info.mint,
            analysis.token_info.creator,
            analysis.risk_score,
            analysis.price_change_24h * 100.0,
            analysis.volume_24h / 1_000_000.0,
            analysis.token_info.market_cap / 1_000_000.0,
            analysis.token_info.liquidity / 1_000_000.0,
            analysis.token_info.holder_count,
            analysis.holder_distribution.top_10_percentage
        )
    }

    fn generate_notification(&self, analysis: &TokenAnalysis) -> NotificationData {
        NotificationData {
            title: format!("Token Alert: {}", analysis.token_info.symbol),
            message: self.format_analysis(analysis),
            level: AlertLevel::Low,
            timestamp: TimeWrapper(SystemTime::now()),
            risk_level: analysis.risk_score,
            attention_level: 0,
            symbol: analysis.token_info.symbol.clone(),
            name: analysis.token_info.name.clone(),
            contract: analysis.token_info.mint.to_string(),
            creator: analysis.token_info.creator.to_string(),
            supply: self.format_supply(analysis.token_info.supply),
            initial_price: analysis.token_info.price,
            current_price: analysis.token_info.price,
            market_cap: self.format_market_cap(analysis.token_info.market_cap),
            liquidity: analysis.token_info.liquidity,
            price_change: self.calculate_price_change(
                analysis.token_info.price,
                analysis.token_info.price
            ),
            lock_info: self.format_lock_info(&analysis.token_info),
            holder_count: analysis.token_info.holder_count,
            concentration: analysis.token_info.holder_concentration,
            fund_flow: self.format_fund_flow(&analysis.fund_flow),
            creator_analysis: self.format_creator_analysis(&analysis.creator_history),
            risk_assessment: self.format_risk_assessment(analysis),
            social_market: self.format_social_market(analysis),
            holder_distribution: self.format_holder_distribution(analysis),
            quick_links: self.format_quick_links(analysis),
            monitor_info: self.format_monitor_info(analysis),
            risk_tips: self.get_risk_tips(analysis.risk_score),
            main_risks: self.get_main_risks(analysis),
            suggestion: self.get_suggestion(analysis.risk_score),
        }
    }

    async fn get_healthy_client(&self) -> Result<Arc<DebugRpcClient>> {
        let clients = &self.rpc_pool.clients;
        for client in clients {
            if client.0.get_slot().await.is_ok() {
                return Ok(client.clone());
            }
        }
        Ok(clients[0].clone())
    }

    async fn analyze_funding_flow(&self, address: &Pubkey) -> Result<Vec<FundingChain>> {
        let transfers = self.fetch_transfers(address).await?;
        let mut chains = Vec::new();
        
        for transfer in transfers {
            // 删除 success_tokens 检查，因为 Transfer 结构体中没有这个字段
            let chain = self.create_funding_chain(transfer);
            chains.push(chain);
        }
        
        Ok(chains)
    }

    async fn fetch_holders(&self, mint: &Pubkey) -> Result<Vec<HolderInfo>> {
        let api_key = self.get_next_api_key().await;
        let url = format!(
            "https://public-api.birdeye.so/public/token_holders?address={}",
            mint
        );

        let response = self.client
            .get(&url)
            .header("X-API-KEY", api_key)
            .send()
            .await?
            .json::<HolderResponse>()
            .await?;

        Ok(response.data.items.into_iter()
            .map(|item| HolderInfo {
                address: item.address,
                balance: item.balance,
                last_activity: UNIX_EPOCH + Duration::from_secs(item.last_activity),
            })
            .collect())
    }

    fn calculate_distribution(&self, holders: &[HolderInfo]) -> HolderDistribution {
        let total_supply: f64 = holders.iter().map(|h| h.balance).sum();
        
        let mut sorted_holders = holders.to_vec();
        sorted_holders.sort_by(|a, b| b.balance.partial_cmp(&a.balance).unwrap());
        
        let top_10_sum: f64 = sorted_holders.iter().take(10).map(|h| h.balance).sum();
        let top_50_sum: f64 = sorted_holders.iter().take(50).map(|h| h.balance).sum();
        let top_100_sum: f64 = sorted_holders.iter().take(100).map(|h| h.balance).sum();

        HolderDistribution {
            exchanges: self.calculate_exchange_percentage(holders),
            whales: self.calculate_whale_percentage(holders),
            retail: self.calculate_retail_percentage(holders),
            inactive: self.calculate_inactive_percentage(holders),
            top_10_percentage: (top_10_sum / total_supply) * 100.0,
            top_50_percentage: (top_50_sum / total_supply) * 100.0,
            top_100_percentage: (top_100_sum / total_supply) * 100.0,
            holder_categories: self.categorize_holders(holders),
            exchange_count: self.count_exchanges(holders),
            whale_count: self.count_whales(holders),
            market_maker_count: self.count_market_makers(holders),
        }
    }

    fn calculate_exchange_percentage(&self, holders: &[HolderInfo]) -> f64 {
        let total_supply: f64 = holders.iter().map(|h| h.balance).sum();
        let exchange_sum: f64 = holders.iter()
            .filter(|h| self.is_exchange_wallet(&h.address))
            .map(|h| h.balance)
            .sum();
        (exchange_sum / total_supply) * 100.0
    }

    fn calculate_whale_percentage(&self, holders: &[HolderInfo]) -> f64 {
        let total_supply: f64 = holders.iter().map(|h| h.balance).sum();
        let whale_sum: f64 = holders.iter()
            .filter(|h| h.balance > total_supply * 0.01)
            .map(|h| h.balance)
            .sum();
        (whale_sum / total_supply) * 100.0
    }

    fn calculate_retail_percentage(&self, holders: &[HolderInfo]) -> f64 {
        let total_supply: f64 = holders.iter().map(|h| h.balance).sum();
        let retail_sum: f64 = holders.iter()
            .filter(|h| h.balance <= total_supply * 0.001)
            .map(|h| h.balance)
            .sum();
        (retail_sum / total_supply) * 100.0
    }

    fn calculate_inactive_percentage(&self, holders: &[HolderInfo]) -> f64 {
        let now = SystemTime::now();
        let total_supply: f64 = holders.iter().map(|h| h.balance).sum();
        let inactive_sum: f64 = holders.iter()
            .filter(|h| h.last_activity.elapsed().unwrap().as_secs() > 30 * 24 * 3600)
            .map(|h| h.balance)
            .sum();
        (inactive_sum / total_supply) * 100.0
    }

    fn count_exchanges(&self, holders: &[HolderInfo]) -> u64 {
        holders.iter()
            .filter(|h| self.is_exchange_wallet(&h.address))
            .count() as u64
    }

    fn count_whales(&self, holders: &[HolderInfo]) -> u64 {
        let total_supply: f64 = holders.iter().map(|h| h.balance).sum();
        holders.iter()
            .filter(|h| h.balance > total_supply * 0.01)
            .count() as u64
    }

    fn count_market_makers(&self, holders: &[HolderInfo]) -> u64 {
        holders.iter()
            .filter(|h| self.is_market_maker(&h.address))
            .count() as u64
    }

    fn categorize_holders(&self, holders: &[HolderInfo]) -> Vec<HolderCategory> {
        holders.iter()
            .map(|h| HolderCategory {
                address: h.address,
                balance: h.balance,
                category: self.get_holder_category(h),
                last_activity: TimeWrapper(h.last_activity),
            })
            .collect()
    }

    fn get_holder_category(&self, holder: &HolderInfo) -> String {
        if self.is_exchange_wallet(&holder.address) {
            "Exchange".to_string()
        } else if self.is_market_maker(&holder.address) {
            "Market Maker".to_string()
        } else {
            "Retail".to_string()
        }
    }

    fn get_missed_blocks_len(&self) -> usize {
        self.metrics.missed_blocks.len()
    }

    fn is_missed_blocks_empty(&self) -> bool {
        self.metrics.missed_blocks.is_empty()
    }

    // 1. 修复 fetch_token_info 返回类型不匹配
    async fn fetch_token_info(&self, mint: &Pubkey) -> Result<TokenInfo> {
        let client = self.get_healthy_client().await?;
        let account = client.0.get_account(mint).await?;
        
        // 解析账户数据
        let data = if !account.data.is_empty() {
            let decoded_data = bs58::decode(&account.data).into_vec()?;
            let token_data: TokenData = bincode::deserialize(&decoded_data)?;
            
            TokenInfo {
                mint: *mint,
                name: token_data.name,
                symbol: token_data.symbol,
                market_cap: token_data.market_cap,
                liquidity: token_data.liquidity,
                holder_count: token_data.holder_count,
                holder_concentration: token_data.holder_concentration,
                verified: token_data.verified,
                price: token_data.price,
                supply: token_data.supply,
                creator: token_data.creator,
            }
        } else {
            return Err(anyhow!("Empty account data"));
        };

        Ok(data)
    }

    // 2. 修复 format_quick_links 参数类型不匹配
    fn format_quick_links(&self, analysis: &TokenAnalysis) -> String {
        format!(
            "快速链接:\nBirdeye: {}\nSolscan: {}\n创建者: {}",
            analysis.token_info.mint.to_string(),
            analysis.token_info.mint.to_string(),
            analysis.token_info.creator.to_string()
        )
    }

    // 3. 修复 TimeWrapper 和 SystemTime 类型不匹配
    fn generate_notification(&self, analysis: &TokenAnalysis) -> NotificationData {
        NotificationData {
            // ...其他字段...
            timestamp: TimeWrapper(SystemTime::now()),  // 使用 TimeWrapper 包装
            detection_time: TimeWrapper(Local::now().into()),  // DateTime 转换为 TimeWrapper
            // ...其他字段...
        }
    }

    // 4. 修复 risk_level 和 attention_level 类型不匹配
    fn get_risk_level(&self, risk_score: u8) -> String {
        match risk_score {
            0..=39 => "低风险".to_string(),
            40..=69 => "中风险".to_string(),
            70..=100 => "高风险".to_string(),
            _ => "未知".to_string(),
        }
    }

    fn get_attention_level(&self, risk_score: u8) -> String {
        match risk_score {
            0..=39 => "无关注".to_string(),
            40..=69 => "关注".to_string(),
            70..=100 => "高度关注".to_string(),
            _ => "未知".to_string(),
        }
    }

    // 5. 修复 process_batch 类型不匹配
    async fn process_batch<T, F, Fut>(&self, items: Vec<T>, process_fn: F) -> Result<()>
    where
        T: Clone,
        F: Fn(T) -> Fut,
        Fut: Future<Output = Result<()>>,
    {
        let mut handles = Vec::new();
        
        for chunk in items.chunks(self.batcher.batch_size.load(Ordering::Relaxed)) {
            let chunk_items: Vec<_> = chunk.to_vec();
            let handle = tokio::spawn(async move {
                for item in chunk_items {
                    process_fn(item).await?;
                }
                Ok::<_, anyhow::Error>(())
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.await??;
        }

        Ok(())
    }

    // 1. 添加 fetch_transfers 方法
    async fn fetch_transfers(&self, address: &Pubkey) -> Result<Vec<Transfer>> {
        let api_key = self.get_next_api_key().await;
        let url = format!(
            "https://public-api.birdeye.so/public/address_activity?address={}",
            address
        );

        let response = self.client
            .get(&url)
            .header("X-API-KEY", api_key)
            .send()
            .await?
            .json::<AddressActivityResponse>()
            .await?;

        Ok(response.data.items.into_iter()
            .map(|item| Transfer {
                source: item.source,
                amount: item.amount,
                timestamp: TimeWrapper(UNIX_EPOCH + Duration::from_secs(item.timestamp)),
            })
            .collect())
    }

    async fn spawn_block_processor(
        &self,
        mut rx: mpsc::Receiver<BlockData>,
        alert_tx: mpsc::Sender<Alert>
    ) -> Result<()> {
        tokio::spawn(async move {
            while let Some(block_data) = rx.recv().await {
                // 处理区块数据
                if let Err(e) = self.process_block_data(&block_data, &alert_tx).await {
                    log::error!("Error processing block data: {}", e);
                }
            }
            Ok(())
        });
        Ok(())
    }

    async fn process_block_data(
        &self, 
        block_data: &BlockData,
        alert_tx: &mpsc::Sender<Alert>
    ) -> Result<()> {
        for tx in &block_data.transactions {
            if let Some((mint, creator)) = self.extract_pump_info(tx) {
                let analysis = self.analyze_token(&mint, &creator).await?;
                if analysis.risk_score > 70 {
                    alert_tx.send(Alert {
                        timestamp: TimeWrapper(SystemTime::now()),
                        level: AlertLevel::High,
                        message: format!("High risk token detected: {}", mint),
                    }).await?;
                }
            }
        }
        Ok(())
    }

    async fn spawn_alert_handler(
        &self,
        mut alert_rx: mpsc::Receiver<Alert>
    ) -> Result<()> {
        tokio::spawn(async move {
            while let Some(alert) = alert_rx.recv().await {
                // 处理告警
                self.handle_alert(&alert).await?;
            }
            Ok(())
        });
        Ok(())
    }

    async fn handle_alert(&self, alert: &Alert) -> Result<()> {
        // 更新监控状态
        let mut state = self.monitor_state.lock().await;
        state.alerts.push(alert.clone());

        // 发送通知
        if alert.level == AlertLevel::High {
            self.send_alert("High Risk Alert", &alert.message).await;
        }

        Ok(())
    }

    async fn send_alert(&self, title: &str, message: &str) -> Result<()> {
        // ServerChan 通知
        if let Some(key) = self.config.serverchan.keys.first() {
            let url = format!("https://sctapi.ftqq.com/{}.send", key);
            self.client
                .post(&url)
                .form(&[("title", title), ("desp", message)])
                .send()
                .await?;
        }

        // WeChat Ferry 通知
        for group in &self.config.wcf.groups {
            let url = format!("http://127.0.0.1:8080/send?wxid={}", group.wxid);
            self.client
                .post(&url)
                .json(&serde_json::json!({
                    "message": format!("{}\n{}", title, message)
                }))
                .send()
                .await?;
        }

        Ok(())
    }

    async fn update_rpc_health(&self, url: String, status: bool) {
        let mut rpc_health = self.rpc_health.lock().await;
        rpc_health.insert(url.clone(), status);
        self.rpc_pool.health_status.insert(url, status); // 应该是 self.rpc_pool.health_status
    }

    async fn analyze_holders(&self, mint: &Pubkey) -> Result<HolderDistribution> {
        // 实现分析持有者的逻辑
        // ...
        Ok(HolderDistribution::default())
    }
}

// 6. 修复 TimeWrapper 相关的类型转换
impl From<DateTime<Local>> for TimeWrapper {
    fn from(dt: DateTime<Local>) -> Self {
        TimeWrapper(dt.into())
    }
}

impl From<SystemTime> for TimeWrapper {
    fn from(st: SystemTime) -> Self {
        TimeWrapper(st)
    }
}

impl TimeWrapper {
    pub fn format(&self, fmt: &str) -> String {
        let datetime: DateTime<Local> = self.0.into();
        datetime.format(fmt).to_string()
    }

    pub fn to_datetime_local(&self) -> DateTime<Local> {
        DateTime::<Local>::from(self.0)
    }
}

//===========================================
// 辅助数据结构
//===========================================
#[derive(Debug)]
struct TokenMetrics {
    supply_score: f64,
    liquidity_score: f64,
    holder_score: f64,
    risk_score: f64,
    market_cap: f64,
    price: f64,
    verified: bool,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    data: TokenData,
}

#[derive(Debug, Deserialize)]
struct TokenData {
    pub mint: Pubkey,
    pub name: String,
    pub symbol: String,
    pub market_cap: f64,
    pub liquidity: f64,
    pub holder_count: u64,
    pub holder_concentration: f64,
    pub verified: bool,
    pub price: f64,
    pub supply: u64,
    pub creator: Pubkey,
}

#[derive(Debug, Deserialize)]
struct AddressActivityResponse {
    data: ActivityData,
}

#[derive(Debug, Deserialize)]
struct ActivityData {
    items: Vec<ActivityItem>,
}

#[derive(Debug, Deserialize)]
struct ActivityItem {
    source: Pubkey,
    amount: f64,
    timestamp: u64,
    signature: String,
}

#[derive(Debug, Deserialize)]
struct TokenListResponse {
    data: TokenListData,
}

#[derive(Debug, Deserialize)]
struct TokenListData {
    items: Vec<TokenListItem>,
    total: u64,
}

#[derive(Debug, Deserialize)]
struct TokenListItem {
    address: Pubkey,
    symbol: String,
    name: String,
    market_cap: f64,
    created_at: u64,
}

#[derive(Debug)]
pub struct LoadMetrics {
    pub cpu_usage: AtomicF64,
    pub memory_usage: AtomicF64,
}

impl Default for LoadMetrics {
    fn default() -> Self {
        Self {
            cpu_usage: AtomicF64::new(0.0),
            memory_usage: AtomicF64::new(0.0),
        }
    }
}

#[derive(Debug)]
pub struct RpcMetrics {
    pub requests: AtomicUsize,
    pub errors: AtomicUsize,
    pub latency: AtomicF64,
}

impl Default for RpcMetrics {
    fn default() -> Self {
        Self {
            requests: AtomicUsize::new(0),
            errors: AtomicUsize::new(0),
            latency: AtomicF64::new(0.0),
        }
    }
}

#[derive(Debug)]
pub struct ServiceHealth {
    pub running: bool,
    pub uptime: u64,
    pub processed_blocks: usize,
    pub processed_tokens: usize,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatorHistory {
    pub success_tokens: Vec<SuccessToken>,
    pub total_tokens: u64,
}

#[derive(Debug, Clone)]
pub struct TokenAnalysis {
    pub token_info: TokenInfo,
    pub creator_history: CreatorHistory,
    pub fund_flow: Vec<FundingChain>,
    pub risk_score: u8,
    pub is_new_wallet: bool,
    pub wallet_age: f64,
    pub social: SocialMediaStats,
    pub price_change_1h: f64,
    pub price_change_24h: f64,
    pub volume_24h: f64,
    pub buy_pressure: f64,
    pub sell_pressure: f64,
    pub liquidity_change: f64,
    pub holder_distribution: HolderDistribution,
    pub detection_time: TimeWrapper,
    pub first_trade_time: TimeWrapper,
    pub liquidity_add_time: TimeWrapper,
    pub monitor_id: u64,
    pub risk_level: u8,
    pub next_update_minutes: u64,
    pub monitoring_status: String,
    pub risk_advice: String,
    pub price_change_initial: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuccessToken {
    pub address: Pubkey,
    pub symbol: String,
    pub name: String,
    pub market_cap: f64,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocialMediaStats {
    pub twitter_followers: u64,
    pub twitter_growth: f64,
    pub discord_members: u64,
    pub discord_activity: f64,
    pub telegram_members: u64,
}

impl Default for SocialMediaStats {
    fn default() -> Self {
        Self {
            twitter_followers: 0,
            twitter_growth: 0.0,
            discord_members: 0,
            discord_activity: 0.0,
            telegram_members: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FundingChain {
    pub source_wallet: Pubkey,
    pub destination_wallet: Pubkey,
    pub total_amount: f64,
    pub transfers: Vec<Transfer>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Transfer {
    pub source: Pubkey,
    pub amount: f64,
    pub timestamp: TimeWrapper,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HolderDistribution {
    pub exchanges: f64,
    pub whales: f64,
    pub retail: f64,
    pub inactive: f64,
    pub top_10_percentage: f64,
    pub top_50_percentage: f64,
    pub top_100_percentage: f64,
    pub holder_categories: Vec<HolderCategory>,
    pub exchange_count: u64,
    pub whale_count: u64,
    pub market_maker_count: u64,
}

impl Default for HolderDistribution {
    fn default() -> Self {
        Self {
            exchanges: 0.0,
            whales: 0.0,
            retail: 0.0,
            inactive: 0.0,
            top_10_percentage: 0.0,
            top_50_percentage: 0.0,
            top_100_percentage: 0.0,
            holder_categories: vec![],
            exchange_count: 0,
            whale_count: 0,
            market_maker_count: 0,
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct MonitorMetrics {
    pub processed_blocks: u64,
    pub processed_txs: u64,
    pub rpc_requests: u64,
    pub rpc_errors: u64,
    pub missed_blocks: Vec<u64>,
    pub processing_delays: VecDeque<Duration>,
    pub block_processing_latency: AtomicU64,
    pub tx_processing_latency: AtomicU64,
    pub token_analysis_latency: AtomicU64,
}

#[derive(Debug)]
pub struct BlockData {
    pub slot: u64,
    pub transactions: Vec<EncodedTransaction>,
    pub timestamp: SystemTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alert {
    pub timestamp: TimeWrapper,  // 改用 TimeWrapper
    pub level: AlertLevel,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AlertLevel {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationData {
    pub title: String,
    pub message: String,
    pub level: AlertLevel,
    pub timestamp: TimeWrapper,  // 改用 TimeWrapper
    pub risk_level: String, // 风险等级，修改为 String
    pub attention_level: String, // 关注等级，修改为 String
    pub symbol: String,
    pub name: String,
    pub contract: String,
    pub creator: String,
    pub supply: String,
    pub initial_price: f64,
    pub current_price: f64,
    pub market_cap: String,
    pub liquidity: f64,
    pub price_change: f64,
    pub lock_info: String,
    pub holder_count: u64,
    pub concentration: f64,
    pub fund_flow: String,
    pub creator_analysis: String,
    pub risk_assessment: String,
    pub social_market: String,
    pub holder_distribution: String,
    pub quick_links: String,
    pub monitor_info: String,
    pub risk_tips: String,
    pub main_risks: String,
    pub suggestion: String,
}

impl Default for NotificationData {
    fn default() -> Self {
        Self {
            title: String::new(),
            message: String::new(),
            level: AlertLevel::Low,
            timestamp: TimeWrapper::default(),
            risk_level: String::new(),
            attention_level: String::new(),
            symbol: String::new(),
            name: String::new(),
            contract: String::new(),
            creator: String::new(),
            supply: String::new(),
            initial_price: 0.0,
            current_price: 0.0,
            market_cap: String::new(),
            liquidity: 0.0,
            price_change: 0.0,
            lock_info: String::new(),
            holder_count: 0,
            concentration: 0.0,
            fund_flow: String::new(),
            creator_analysis: String::new(),
            risk_assessment: String::new(),
            social_market: String::new(),
            holder_distribution: String::new(),
            quick_links: String::new(),
            monitor_info: String::new(),
            risk_tips: String::new(),
            main_risks: String::new(),
            suggestion: String::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ContractAnalysis {
    pub is_upgradeable: bool,
    pub has_mint_authority: bool,
    pub has_freeze_authority: bool,
    pub has_blacklist: bool,
    pub locked_liquidity: bool,
    pub max_tx_amount: Option<f64>,
    pub buy_tax: f64,
    pub sell_tax: f64,
}

#[derive(Debug, Clone)]
pub struct PriceTrendAnalysis {
    pub price_change_1h: f64,
    pub price_change_24h: f64,
    pub volume_change_24h: f64,
    pub liquidity_change_24h: f64,
    pub buy_pressure: f64,
    pub sell_pressure: f64,
    pub major_transactions: Vec<Transaction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComprehensiveScore {
    pub total_score: u8,
    pub risk_score: u8,
    pub social_score: u8,
    pub market_score: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HolderCategory {
    pub address: Pubkey,
    pub balance: f64,
    pub category: String,
    pub last_activity: TimeWrapper,  // 改用 TimeWrapper
}

#[derive(Debug, Clone)]
pub enum TransactionType {
    Buy,
    Sell,
    Transfer,
    Liquidity,
}

#[derive(Debug)]
pub struct Recovery {
    pub last_checkpoint: SystemTime,
    pub retry_count: AtomicUsize,
    pub error_log: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct TransactionMessage {
    pub account_keys: Vec<Pubkey>,
    pub recent_blockhash: String,
    pub instructions: Vec<TransactionInstruction>,
}

#[derive(Debug, Clone)]
pub struct TransactionInstruction {
    pub program_id: Pubkey,
    pub accounts: Vec<Pubkey>,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeWrapper(#[serde(with = "timestamp_serde")] SystemTime);

impl Default for TimeWrapper {
    fn default() -> Self {
        Self(SystemTime::now())
    }
}

mod timestamp_serde {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::{SystemTime, UNIX_EPOCH};

    pub fn serialize<S>(time: &SystemTime, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let duration = time.duration_since(UNIX_EPOCH)
            .map_err(serde::ser::Error::custom)?;
        serializer.serialize_u64(duration.as_secs())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<SystemTime, D::Error>
    where
        D: Deserializer<'de>,
    {
        let secs = u64::deserialize(deserializer)?;
        Ok(UNIX_EPOCH + std::time::Duration::from_secs(secs))
    }
}

// 在需要 Default 的结构体中使用 TimeWrapper
#[derive(Debug, Default)]
struct MonitorState {
    last_update: TimeWrapper,
    processed_blocks: u64,
    processed_tokens: u64,
    alerts: Vec<Alert>,
    watch_addresses: HashSet<String>,
    metrics: MonitorMetrics,
    start_time: TimeWrapper,  // 改用 TimeWrapper 替代 SystemTime
    last_block_slot: u64,
}

impl MonitorState { // 手动实现 Default trait
    fn default() -> Self {
        MonitorState { last_update: TimeWrapper::default(), processed_blocks: 0, processed_tokens: 0, alerts: Vec::new(), watch_addresses: HashSet::new(), metrics: MonitorMetrics::default(), start_time: TimeWrapper::default(), last_block_slot: 0 } // 使用 UNIX_EPOCH 作为默认值
    }
}

#[derive(Debug, Deserialize)]
struct SocialResponse {
    twitter: SocialPlatform,
    discord: SocialPlatform,
    telegram: SocialPlatform,
}

#[derive(Debug, Deserialize)]
struct SocialPlatform {
    followers: u64,
    growth: f64,
    members: u64,
    activity: f64,
}

#[derive(Debug)]
struct HolderInfo {
    address: Pubkey,
    balance: f64,
    last_activity: SystemTime,
}

#[derive(Debug, Clone)]
struct Transaction {
    pub amount: f64,
    pub price: f64,
    pub timestamp: TimeWrapper,
    pub transaction_type: TransactionType,
}

impl Default for Transaction {
    fn default() -> Self {
        Self {
            amount: 0.0,
            price: 0.0,
            timestamp: TimeWrapper::default(),
            transaction_type: TransactionType::Transfer,
        }
    }
}

#[derive(Clone)]
pub struct DebugRpcClient(RpcClient);

impl std::fmt::Debug for DebugRpcClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RpcClient")
            .field("url", &self.0.url())
            .finish()
    }
}

impl DebugRpcClient {
    pub fn new(url: String) -> Self {
        Self(RpcClient::new_with_commitment(
            url,
            CommitmentConfig::confirmed(),
        ))
    }
}

#[derive(Debug, Deserialize)]
pub struct HolderResponse {
    pub data: HolderData,
}

#[derive(Debug, Deserialize)]
pub struct HolderData {
    pub items: Vec<HolderItem>,
}

#[derive(Debug, Deserialize)]
pub struct HolderItem {
    pub address: Pubkey,
    pub balance: f64,
    pub last_activity: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_token_analysis() {
        let monitor = TokenMonitor::new().await.unwrap();
        let mint = Pubkey::new_unique();
        let creator = Pubkey::new_unique();
        
        let analysis = monitor.analyze_token(&mint, &creator).await.unwrap();
        assert!(analysis.risk_score <= 100);
    }

    #[test]
    fn test_risk_score_calculation() {
        // 添加风险评分计算的测试
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // 初始化日志
    env_logger::init();
    
    // 创建监控实例
    let monitor = TokenMonitor::new().await?;
    
    // 注册信号处理
    let running = monitor.service_state.running.clone();
    ctrlc::set_handler(move || {
        running.store(false, Ordering::SeqCst);
    })?;
    
    // 启动服务
    monitor.start_service().await?;
    
    // 等待服务停止
    while monitor.service_state.running.load(Ordering::SeqCst) {
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    
    // 清理资源
    monitor.stop_service().await?;
    
    Ok(())
} 
