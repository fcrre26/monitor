#![allow(unused_imports)]

extern crate solana_client;
extern crate solana_sdk;
extern crate solana_transaction_status;
extern crate tokio;
extern crate log4rs;
extern crate dirs;
extern crate serde_json;
extern crate chrono;
extern crate env_logger;
extern crate colored;

use {
    anyhow::Result,
    log,
    reqwest,
    serde::{Deserialize, Serialize},
    solana_client::rpc_client::RpcClient,
    solana_sdk::{commitment_config::CommitmentConfig, pubkey::Pubkey},
    solana_transaction_status::{
        EncodedConfirmedBlock,
        EncodedTransaction,
        UiTransactionEncoding,
    },
    std::{
        collections::{HashMap, HashSet},
        sync::Arc,
        time::{Duration, SystemTime, Instant},
        fs::File,
        io::Read,
        process,
        path::Path,
    },
    tokio::{
        sync::{mpsc, Mutex},
        time,
    },
    log4rs::{
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
    },
    colored::*,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Config {
    api_keys: Vec<String>,
    serverchan: ServerChanConfig,
    wcf: WeChatFerryConfig,
    proxy: ProxyConfig,
    rpc_nodes: HashMap<String, RpcNodeConfig>,
}

impl Config {
    fn builder() -> ConfigBuilder {
        ConfigBuilder::default()
    }
}

#[derive(Default)]
struct ConfigBuilder {
    appenders: Vec<Appender>,
    root: Option<Root>,
}

impl ConfigBuilder {
    fn appender(mut self, appender: Appender) -> Self {
        self.appenders.push(appender);
        self
    }

    fn build(self, root: Root) -> Result<Config> {
        // 实现构建逻辑
        Ok(Config {
            api_keys: Vec::new(),
            serverchan: ServerChanConfig { keys: Vec::new() },
            wcf: WeChatFerryConfig { groups: Vec::new() },
            proxy: ProxyConfig {
                enabled: false,
                ip: String::new(),
                port: 0,
                username: String::new(),
                password: String::new(),
            },
            rpc_nodes: HashMap::new(),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ServerChanConfig {
    keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WeChatFerryConfig {
    groups: Vec<WeChatGroup>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WeChatGroup {
    name: String,
    wxid: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProxyConfig {
    enabled: bool,
    ip: String,
    port: u16,
    username: String,
    password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RpcNodeConfig {
    weight: f64,
    fails: u64,
    last_used: u64,
}

#[derive(Debug)]
struct Metrics {
    processed_blocks: u64,
    processed_txs: u64,
    missed_blocks: HashSet<u64>,
    processing_delays: Vec<Duration>,
    last_process_time: Instant,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            processed_blocks: 0,
            processed_txs: 0,
            missed_blocks: HashSet::new(),
            processing_delays: Vec::new(),
            last_process_time: Instant::now(),
        }
    }
}

struct TokenMonitor {
    config: Config,
    rpc_clients: Vec<Arc<RpcClient>>,
    metrics: Arc<Mutex<Metrics>>,
    pump_program: Pubkey,
    client: reqwest::Client,
    current_api_key: Arc<Mutex<usize>>,
    request_counts: HashMap<String, u32>,
    last_reset: HashMap<String, SystemTime>,
    cache: Arc<Mutex<Cache>>,
    watch_addresses: HashSet<String>,
    monitor_state: Arc<Mutex<MonitorState>>,
    proxy_pool: Arc<Mutex<ProxyPool>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct MonitorState {
    last_slot: u64,
    processed_mints: HashSet<String>,
    start_time: SystemTime,
}

impl Default for MonitorState {
    fn default() -> Self {
        Self {
            last_slot: 0,
            processed_mints: HashSet::new(),
            start_time: SystemTime::now(),
        }
    }
}

#[derive(Default)]
struct Cache {
    token_info: HashMap<Pubkey, (TokenInfo, SystemTime)>,
    creator_history: HashMap<Pubkey, (CreatorHistory, SystemTime)>,
    fund_flow: HashMap<Pubkey, (Vec<FundingChain>, SystemTime)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TokenInfo {
    mint: Pubkey,
    name: String,
    symbol: String,
    market_cap: f64,
    liquidity: f64,
    holder_count: u64,
    holder_concentration: f64,
    verified: bool,
    price: f64,
    supply: u64,
    creator: Pubkey,
}

impl From<TokenResponse> for TokenInfo {
    fn from(response: TokenResponse) -> Self {
        response.data
    }
}

#[derive(Debug, Clone)]
struct CreatorHistory {
    success_tokens: Vec<SuccessToken>,
    total_tokens: u64,
}

#[derive(Debug, Clone)]
struct SuccessToken {
    address: Pubkey,
    symbol: String,
    name: String,
    market_cap: f64,
    created_at: u64,
}

#[derive(Debug, Clone)]
struct FundingChain {
    transfers: Vec<Transfer>,
    total_amount: f64,
    risk_score: u8,
}

#[derive(Debug, Clone)]
struct Transfer {
    source: Pubkey,
    amount: f64,
    timestamp: u64,
    tx_id: String,
    success_tokens: Option<Vec<SuccessToken>>,
}

#[derive(Debug)]
struct TokenAnalysis {
    token_info: TokenInfo,
    creator_history: CreatorHistory,
    fund_flow: Vec<FundingChain>,
    risk_score: u8,
    is_new_wallet: bool,
    wallet_age: f64,
}

#[derive(Debug, Default)]
struct ProxyPool {
    proxies: Vec<ProxyConfig>,
    current_index: usize,
    last_check: SystemTime,
}

impl ProxyPool {
    async fn get_next_proxy(&mut self) -> Option<reqwest::Proxy> {
        if self.proxies.is_empty() {
            return None;
        }

        let proxy = &self.proxies[self.current_index];
        self.current_index = (self.current_index + 1) % self.proxies.len();

        Some(reqwest::Proxy::http(&format!(
            "http://{}:{}@{}:{}",
            proxy.username,
            proxy.password,
            proxy.ip,
            proxy.port
        )).unwrap())
    }

    async fn check_proxies(&mut self) {
        let client = reqwest::Client::new();
        let mut valid_proxies = Vec::new();

        for proxy in &self.proxies {
            let proxy_url = format!(
                "http://{}:{}@{}:{}",
                proxy.username,
                proxy.password,
                proxy.ip,
                proxy.port
            );

            let proxy = match reqwest::Proxy::http(&proxy_url) {
                Ok(p) => p,
                Err(_) => continue,
            };

            let test_client = match client.clone()
                .proxy(proxy)
                .build() {
                Ok(c) => c,
                Err(_) => continue,
            };

            match test_client.get("https://api.mainnet-beta.solana.com")
                .timeout(Duration::from_secs(5))
                .send()
                .await {
                Ok(_) => valid_proxies.push(proxy.clone()),
                Err(_) => log::warn!("代理不可用: {}", proxy_url),
            }
        }

        self.proxies = valid_proxies;
        self.current_index = 0;
    }
}

impl TokenMonitor {
    const PARALLEL_REQUESTS: usize = 20;
    const BLOCK_BATCH_SIZE: usize = 100;
    const WORKER_THREADS: usize = 20;

    fn get_proxy(&self) -> Option<reqwest::Proxy> {
        if !self.config.proxy.enabled {
            return None;
        }

        let proxy_url = format!(
            "http://{}:{}@{}:{}",
            self.config.proxy.username,
            self.config.proxy.password,
            self.config.proxy.ip,
            self.config.proxy.port
        );

        Some(reqwest::Proxy::http(&proxy_url).unwrap())
    }

    async fn new() -> Result<Self> {
        // 初始化日志
        Self::init_logger()?;
        
        log::info!("Starting Solana Token Monitor...");
        
        let config = Self::load_config()?;
        
        let proxy_pool = if config.proxy.enabled {
            let proxies = Self::load_proxy_list()?;
            Arc::new(Mutex::new(ProxyPool::new(proxies)))
        } else {
            Arc::new(Mutex::new(ProxyPool::default()))
        };

        let mut client_builder = reqwest::Client::builder();
        if let Some(proxy) = Self::get_proxy(&config.proxy) {
            client_builder = client_builder.proxy(proxy);
        }
        let client = client_builder.build()?;

        let mut monitor = Self {
            config: config.clone(),
            rpc_clients: Vec::new(),
            metrics: Arc::new(Mutex::new(Metrics::default())),
            pump_program: "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ35MKDfgCcMKJ".parse()?,
            client,
            current_api_key: Arc::new(Mutex::new(0)),
            request_counts: HashMap::new(),
            last_reset: HashMap::new(),
            cache: Arc::new(Mutex::new(Cache::default())),
            watch_addresses: HashSet::new(),
            monitor_state: Arc::new(Mutex::new(MonitorState::default())),
            proxy_pool,
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
            rpc_clients: monitor.rpc_clients,
            metrics: monitor.metrics,
            pump_program: monitor.pump_program,
            client: monitor.client,
            current_api_key: monitor.current_api_key,
            request_counts: monitor.request_counts,
            last_reset: monitor.last_reset,
            cache: monitor.cache,
            watch_addresses,
            monitor_state: Arc::new(Mutex::new(monitor_state)),
            proxy_pool: monitor.proxy_pool,
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
            self.rpc_clients.push(Arc::new(RpcClient::new_with_commitment(
                url.to_string(),
                CommitmentConfig::confirmed(),
            )));
        }
    }

    async fn collect_metrics(metrics: Arc<Mutex<Metrics>>) {
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

    fn load_config() -> Result<Config> {
        let config_path = dirs::home_dir()
            .ok_or_else(|| anyhow!("Cannot find home directory"))?
            .join(".solana_pump.cfg");

        let config_str = fs::read_to_string(config_path)?;
        Ok(serde_json::from_str(&config_str)?)
    }

    async fn start(&mut self) -> Result<()> {
        log::info!("Starting Solana token monitor...");
        
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
            metrics.processing_delays.push(start_time.elapsed());
            
            time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn get_current_slot(&self) -> Result<u64> {
        for client in &self.rpc_clients {
            match client.get_slot().await {
                Ok(slot) => return Ok(slot),
                Err(e) => log::warn!("RPC client error: {}", e),
            }
        }
        Err(anyhow!("All RPC clients failed"))
    }

    async fn process_block(
        &self,
        slot: u64,
        token_tx: &mpsc::Sender<(Pubkey, Pubkey)>,
    ) -> Result<()> {
        let block = match self.get_block(slot).await {
            Ok(Some(block)) => block,
            Ok(None) => {
                let mut metrics = self.metrics.lock().await;
                metrics.missed_blocks.insert(slot);
                return Ok(());
            }
            Err(e) => {
                log::error!("Failed to get block {}: {}", slot, e);
                return Err(e.into());
            }
        };

        for tx in block.transactions {
            if let Some((mint, creator)) = self.extract_pump_info(&tx) {
                token_tx.send((mint, creator)).await?;
                let mut metrics = self.metrics.lock().await;
                metrics.processed_txs += 1;
            }
        }

        let mut metrics = self.metrics.lock().await;
        metrics.processed_blocks += 1;
        Ok(())
    }

    async fn get_block(&self, slot: u64) -> Result<Option<EncodedConfirmedBlock>> {
        for client in &self.rpc_clients {
            match client.get_block_with_encoding(
                slot,
                solana_transaction_status::UiTransactionEncoding::Json,
            ).await {
                Ok(block) => return Ok(Some(block)),
                Err(e) => log::warn!("Failed to get block from RPC: {}", e),
            }
        }
        Ok(None)
    }

    fn extract_pump_info(&self, tx: &EncodedTransaction) -> Option<(Pubkey, Pubkey)> {
        let message = &tx.message;
        
        if !message.account_keys.contains(&self.pump_program) {
            return None;
        }
        
        Some((
            message.account_keys[4],
            message.account_keys[0],
        ))
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

    async fn analyze_token(&self, mint: &Pubkey, creator: &Pubkey) -> Result<TokenAnalysis> {
        let (token_info, creator_history, fund_flow) = tokio::join!(
            self.fetch_token_info(mint),
            self.analyze_creator_history(creator),
            self.trace_fund_flow(creator)
        );

        let token_info = token_info?;
        let creator_history = creator_history?;
        let fund_flow = fund_flow?;
        
        let risk_score = self.calculate_risk_score(&token_info, &creator_history, &fund_flow);
        
        let wallet_age = self.calculate_wallet_age(creator).await?;
        
        Ok(TokenAnalysis {
            token_info,
            creator_history,
            fund_flow,
            risk_score,
            is_new_wallet: wallet_age < 1.0,
            wallet_age,
        })
    }

    async fn fetch_token_info(&self, mint: &Pubkey) -> Result<TokenInfo> {
        if let Some(info) = self.cache.lock().await.token_info.get(mint) {
            if info.1.elapsed()? < Duration::from_secs(300) {
                return Ok(info.0.clone());
            }
        }

        let api_key = self.get_next_api_key().await;
        
        let response = self.client
            .get(&format!(
                "https://public-api.birdeye.so/public/token?address={}",
                mint
            ))
            .header("X-API-KEY", api_key)
            .send()
            .await?;
            
        let data: TokenResponse = response.json().await?;
        let info = TokenInfo::from(data);
        
        self.cache.lock().await.token_info.insert(*mint, (info.clone(), SystemTime::now()));
        
        Ok(info)
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
        const MAX_DEPTH: u8 = 5;
        let mut visited = HashSet::new();
        self.trace_fund_flow_recursive(address, &mut visited, 0, MAX_DEPTH).await
    }

    async fn trace_fund_flow_recursive(
        &self,
        address: &Pubkey,
        visited: &mut HashSet<Pubkey>,
        depth: u8,
        max_depth: u8,
    ) -> Result<Vec<FundingChain>> {
        if depth >= max_depth || visited.contains(address) {
            return Ok(Vec::new());
        }

        visited.insert(*address);
        let transfers = self.get_address_transfers(address).await?;
        let mut chains = Vec::new();

        for transfer in transfers {
            if transfer.amount < 1.0 {
                continue;
            }

            let source = transfer.source;
            if visited.contains(&source) {
                continue;
            }

            let success_tokens = self.check_address_success_tokens(&source).await?;
            let mut chain = FundingChain {
                transfers: vec![Transfer {
                    source,
                    amount: transfer.amount,
                    timestamp: transfer.timestamp,
                    tx_id: transfer.signature,
                    success_tokens: if success_tokens.is_empty() {
                        None
                    } else {
                        Some(success_tokens)
                    },
                }],
                total_amount: transfer.amount,
                risk_score: 0,
            };

            let sub_chains = self.trace_fund_flow_recursive(&source, visited, depth + 1, max_depth).await?;
            
            for mut sub_chain in sub_chains {
                sub_chain.transfers.extend(chain.transfers.clone());
                sub_chain.total_amount += chain.total_amount;
                chains.push(sub_chain);
            }

            if chain.transfers.iter().any(|t| t.success_tokens.is_some()) {
                chains.push(chain);
            }
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
                tx_id: tx.signature,
                success_tokens: None,
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
        let client = &self.rpc_clients[0];
        
        let signatures = client
            .get_signatures_for_address(address)
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
            if let Err(e) = self.send_server_chan(key, &message).await {
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

    async fn send_server_chan(&self, key: &str, message: &str) -> Result<()> {
        let res = self.client
            .post(&format!("https://sctapi.ftqq.com/{}.send", key))
            .form(&[
                ("title", "Solana新代币提醒"),
                ("desp", &format!("{}\n\n**合约地址(点击复制)**\n```\n{}\n```", 
                    message, 
                    analysis.token_info.mint)),
            ])
            .send()
            .await?;
            
        if !res.status().is_success() {
            return Err(anyhow!("ServerChan push failed: {}", res.text().await?));
        }
        
        Ok(())
    }

    async fn send_wechat(&self, group: &WeChatGroup, message: &str) -> Result<()> {
        log::info!("Sending WeChat message to {}: {}", group.name, message);
        Ok(())
    }

    fn format_message(&self, analysis: &TokenAnalysis) -> String {
        let mut msg = vec![
            "┏━━━━━━━━━━━━━━━━━━━━━ 🔔 发现新代币 (UTC+8) ━━━━━━━━━━━━━━━━━━━━━┓".to_string(),
            "".to_string(),
            "📋 合约信息".to_string(),
            format!("┣━ CA: {}", analysis.token_info.mint),
            format!("┣━ 创建者: {}", analysis.token_info.creator),
            format!(
                "┗━ 钱包状态: {} | 钱包年龄: {:.1f} 天",
                if analysis.is_new_wallet { "🆕 新钱包" } else { "📅 老钱包" },
                analysis.wallet_age
            ),
        ];

        self.add_token_info(&mut msg, &analysis.token_info);
        
        if !analysis.fund_flow.is_empty() {
            self.add_fund_flow_info(&mut msg, &analysis.fund_flow);
        }

        if !analysis.creator_history.success_tokens.is_empty() {
            self.add_creator_history(&mut msg, &analysis.creator_history);
        }

        self.add_risk_assessment(&mut msg, analysis);
        self.add_quick_links(&mut msg, &analysis.token_info);

        msg.join("\n")
    }

    fn add_token_info(&self, msg: &mut Vec<String>, token_info: &TokenInfo) {
        msg.extend_from_slice(&[
            "┏━━━━━━━━━━━━━━━━━━━━━ 💰 代币数据 ━━━━━━━━━━━━━━━━━━━━━┓".to_string(),
            format!(
                "┃ 代币名称: {:<15} | 代币符号: {:<8} | 认证状态: {} ┃",
                token_info.name,
                token_info.symbol,
                if token_info.verified { "✅ 已认证" } else { "❌ 未认证" }
            ),
            format!(
                "┃ 初始市值: ${:<12} | 代币供应量: {:<8} | 单价: ${} ┃",
                self.format_number(token_info.market_cap),
                self.format_number(token_info.supply as f64),
                token_info.price
            ),
            format!(
                "┃ 流动性: {:.2} SOL{} | 持有人数: {:<8} | 前10持有比: {:.2}% ┃",
                self.format_number(token_info.liquidity),
                " ".repeat(8),
                token_info.holder_count,
                self.format_number(token_info.holder_concentration)
            ),
        ]);
    }

    fn add_fund_flow_info(&self, msg: &mut Vec<String>, fund_flow: &[FundingChain]) {
        let total_transfer: f64 = fund_flow.iter()
            .map(|chain| chain.total_amount)
            .sum();
            
        msg.push(format!("💸 资金追踪 (总流入: {:.2} SOL)", total_transfer));
        
        for (i, chain) in fund_flow.iter().enumerate() {
            msg.push(format!("┣━ 资金链#{} ({:.2} SOL) - 上游资金追踪", i + 1, chain.total_amount));
            
            for (j, transfer) in chain.transfers.iter().enumerate() {
                let wallet_level = (b'E' - j as u8) as char;
                let time_str = self.format_timestamp(transfer.timestamp);
                msg.push(format!(
                    "┃   ⬆️ {:.2} SOL ({}) | 来自钱包{}: {}",
                    transfer.amount,
                    time_str,
                    wallet_level,
                    transfer.source
                ));
                
                if let Some(ref tokens) = transfer.success_tokens {
                    let token_info: Vec<String> = tokens.iter()
                        .map(|t| format!("{}(${:.2}M)", t.symbol, t.market_cap / 1_000_000.0))
                        .collect();
                    msg.push(format!("┃   └─ 创建者历史: {}", token_info.join(" ")));
                } else {
                    msg.push("┃   └─ 仅用于转账".to_string());
                }
            }
        }
    }

    fn add_creator_history(&self, msg: &mut Vec<String>, history: &CreatorHistory) {
        let active_tokens = history.success_tokens.len();
        let success_rate = active_tokens as f64 / history.total_tokens as f64;
        
        let best_token = history.success_tokens.iter()
            .max_by_key(|t| (t.market_cap * 1000.0) as u64)
            .unwrap();
        let avg_market_cap = history.success_tokens.iter()
            .map(|t| t.market_cap)
            .sum::<f64>() / active_tokens as f64;
        let latest_token = history.success_tokens.iter()
            .max_by_key(|t| t.created_at)
            .unwrap();
        
        msg.extend_from_slice(&[
            "┏━━━━━━━━━━━━━━━━━━━━━ 📜 创建者历史 ━━━━━━━━━━━━━━━━━━━━━┓".to_string(),
            format!(
                "┃ 历史代币: {}个 | 成功项目: {}个 | 成功率: {:.1}%{}┃",
                history.total_tokens,
                active_tokens,
                success_rate * 100.0,
                " ".repeat(20)
            ),
            format!(
                "┃ 最佳业绩: {}(${:.1}M) | 平均市值: ${:.1}M | 最近: {}(${:.1}M) ┃",
                best_token.symbol,
                best_token.market_cap / 1_000_000.0,
                avg_market_cap / 1_000_000.0,
                latest_token.symbol,
                latest_token.market_cap / 1_000_000.0
            ),
            "┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛".to_string(),
        ]);
    }

    fn add_risk_assessment(&self, msg: &mut Vec<String>, analysis: &TokenAnalysis) {
        msg.extend_from_slice(&[
            "".to_string(),
            "🎯 风险评估".to_string(),
            format!(
                "┣━ 风险评分: {}/100 | 风险等级: {}",
                analysis.risk_score,
                if analysis.risk_score >= 70 { "高" }
                else if analysis.risk_score >= 40 { "中" }
                else { "低" }
            ),
        ]);
    }

    fn add_quick_links(&self, msg: &mut Vec<String>, token_info: &TokenInfo) {
        msg.extend_from_slice(&[
            "".to_string(),
            "🔗 快速链接".to_string(),
            format!("┣━ Birdeye: https://birdeye.so/token/{}", token_info.mint),
            format!("┣━ Solscan: https://solscan.io/token/{}", token_info.mint),
            format!("┗━ 创建者: https://solscan.io/account/{}", token_info.creator),
            "".to_string(),
            format!("⏰ 发现时间: {} (UTC+8)",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S")),
        ]);
    }

    fn format_number(&self, number: f64) -> String {
        if number >= 1_000_000_000.0 {
            format!("{:.2}B", number / 1_000_000_000.0)
        } else if number >= 1_000_000.0 {
            format!("{:.2}M", number / 1_000_000.0)
        } else if number >= 1_000.0 {
            format!("{:.2}K", number / 1_000.0)
        } else {
            format!("{:.2}", number)
        }
    }

    fn format_timestamp(&self, timestamp: u64) -> String {
        let datetime = chrono::DateTime::from_timestamp(timestamp as i64, 0)
            .unwrap()
            .with_timezone(&chrono::FixedOffset::east(8));
        datetime.format("%m-%d %H:%M").to_string()
    }

    async fn should_notify(&self, analysis: &TokenAnalysis) -> bool {
        if analysis.risk_score >= 80 {
            return false;
        }
        
        if analysis.token_info.liquidity < 5.0 {
            return false;
        }
        
        if !analysis.creator_history.success_tokens.is_empty() {
            return true;
        }
        
        let has_successful_source = analysis.fund_flow.iter()
            .any(|chain| chain.transfers.iter()
                .any(|t| t.success_tokens.is_some()));
                
        if has_successful_source {
            return true;
        }
        
        false
    }

    async fn test_fund_tracking(&self, creator: &Pubkey) -> Result<()> {
        println!("\n测试资金追踪功能...");
        println!("追踪地址: {}", creator);
        
        let funding_chains = self.trace_fund_flow(creator).await?;
        
        if !funding_chains.is_empty() {
            println!("\n发现 {} 条资金链:", funding_chains.len());
            for (i, chain) in funding_chains.iter().enumerate() {
                println!("\n链路 {}:", i + 1);
                println!("总转账金额: {:.2} SOL", chain.total_amount);
                println!("链路深度: {} 层", chain.transfers.len());
                
                for (j, transfer) in chain.transfers.iter().enumerate() {
                    let time_str = self.format_timestamp(transfer.timestamp);
                    println!(
                        "  [{}/{}] {} | {:.2} SOL",
                        j + 1,
                        chain.transfers.len(),
                        time_str,
                        transfer.amount
                    );
                    println!("      {} ->", transfer.source);
                    
                    if let Some(ref tokens) = transfer.success_tokens {
                        for token in tokens {
                            println!(
                                "      历史代币: {} (${:.2}M)",
                                token.symbol,
                                token.market_cap / 1_000_000.0
                            );
                        }
                    }
                }
            }
        } else {
            println!("未发现资金链");
        }
        
        Ok(())
    }

    async fn test_token_info(&self, mint: &Pubkey) -> Result<()> {
        println!("\n测试代币信息获取...");
        println!("获取代币信息: {}", mint);
        
        let token_info = self.fetch_token_info(mint).await?;
        
        println!("\n代币详情:");
        println!("名称: {}", token_info.name);
        println!("符号: {}", token_info.symbol);
        println!("市值: ${}", self.format_number(token_info.market_cap));
        println!("流动性: {:.2} SOL", token_info.liquidity);
        println!("持有人数量: {}", token_info.holder_count);
        println!("持有人集中度: {:.2}%", token_info.holder_concentration);
        
        Ok(())
    }

    async fn show_menu(&mut self) -> Result<()> {
        println!("\n=== Solana Token Monitor ===");
        println!("1. 开始监控");
        println!("2. 测试资金追踪");
        println!("3. 测试代币信息");
        println!("4. 扫描RPC节点");
        println!("5. 管理监控地址");
        println!("6. 管理代理");
        println!("7. 生成配置文件");
        println!("8. 管理日志");
        println!("9. 测试Server酱通知");
        println!("10. 退出");
        println!("请选择功能 (1-10): ");

        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;

        match input.trim() {
            "1" => {
                println!("开始监控...");
                self.start().await?;
            }
            "2" => {
                println!("请输入创建者地址: ");
                let mut address = String::new();
                std::io::stdin().read_line(&mut address)?;
                let pubkey: Pubkey = address.trim().parse()?;
                self.test_fund_tracking(&pubkey).await?;
            }
            "3" => {
                println!("请输入代币地址: ");
                let mut address = String::new();
                std::io::stdin().read_line(&mut address)?;
                let pubkey: Pubkey = address.trim().parse()?;
                self.test_token_info(&pubkey).await?;
            }
            "4" => {
                println!("开始扫描RPC节点...");
                self.scan_rpc_nodes().await?;
            }
            "5" => {
                self.manage_watch_addresses().await?;
            }
            "6" => {
                self.manage_proxies().await?;
            }
            "7" => {
                Self::generate_config_files()?;
                println!("配置文件生成完成");
            }
            "8" => {
                self.manage_logs().await?;
            }
            "9" => {
                self.test_serverchan();
            }
            "10" => {
                println!("退出程序");
                self.save_monitor_state().await?;
                std::process::exit(0);
            }
            _ => {
                println!("无效选项，请重新选择");
            }
        }

        Ok(())
    }

    async fn scan_rpc_nodes(&self) -> Result<()> {
        println!("\n开始扫描 Solana RPC 节点...");
        
        let nodes = vec![
            "https://api.mainnet-beta.solana.com",
            "https://api.metaplex.solana.com",
            "https://solana-api.projectserum.com",
            "https://ssc-dao.genesysgo.net",
            "https://rpc.ankr.com/solana",
            "https://mainnet.rpcpool.com",
            "https://api.mainnet.rpcpool.com",
        ];

        for node in nodes {
            match RpcClient::new_with_commitment(
                node.to_string(),
                CommitmentConfig::confirmed(),
            ).get_slot().await {
                Ok(slot) => {
                    println!("✅ {} - 当前区块: {}", node, slot);
                }
                Err(e) => {
                    println!("❌ {} - 错误: {}", node, e);
                }
            }
        }

        Ok(())
    }

    fn load_watch_addresses() -> Result<HashSet<String>> {
        let watch_file = Path::new("watch_addresses.json");

        if !watch_file.exists() {
            return Ok(HashSet::new());
        }

        let content = fs::read_to_string(watch_file)?;
        Ok(serde_json::from_str(&content)?)
    }

    fn load_monitor_state() -> Result<MonitorState> {
        let state_file = dirs::home_dir()
            .ok_or_else(|| anyhow!("Cannot find home directory"))?
            .join(".solana_pump/monitor_state.json");

        if !state_file.exists() {
            return Ok(MonitorState::default());
        }

        let content = fs::read_to_string(state_file)?;
        Ok(serde_json::from_str(&content)?)
    }

    async fn save_monitor_state(&self) -> Result<()> {
        let state_file = dirs::home_dir()
            .ok_or_else(|| anyhow!("Cannot find home directory"))?
            .join(".solana_pump/monitor_state.json");

        let state = self.monitor_state.lock().await;
        let content = serde_json::to_string_pretty(&*state)?;
        fs::write(state_file, content)?;
        
        Ok(())
    }

    async fn manage_watch_addresses(&mut self) -> Result<()> {
        loop {
            println!("\n=== 监控地址管理 ===");
            println!("1. 查看当前地址");
            println!("2. 添加地址");
            println!("3. 删除地址");
            println!("4. 导入地址列表");
            println!("5. 导出地址列表");
            println!("6. 查看地址详情");
            println!("7. 返回主菜单");
            println!("请选择功能 (1-7): ");

            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;

            match input.trim() {
                "1" => {
                    println!("\n当前监控地址 (共 {} 个):", self.watch_addresses.len());
                    for addr in &self.watch_addresses {
                        println!("{}", addr);
                    }
                }
                "2" => {
                    println!("请输入要添加的地址: ");
                    let mut address = String::new();
                    std::io::stdin().read_line(&mut address)?;
                    let address = address.trim();
                    
                    match Pubkey::from_str(address) {
                        Ok(_) => {
                            self.watch_addresses.insert(address.to_string());
                            self.save_watch_addresses()?;
                            println!("✅ 地址添加成功");
                        }
                        Err(_) => println!("❌ 无效的Solana地址格式"),
                    }
                }
                "3" => {
                    println!("请输入要删除的地址: ");
                    let mut address = String::new();
                    std::io::stdin().read_line(&mut address)?;
                    let address = address.trim();
                    
                    if self.watch_addresses.remove(address) {
                        self.save_watch_addresses()?;
                        println!("✅ 地址删除成功");
                    } else {
                        println!("❌ 地址不存在");
                    }
                }
                "4" => {
                    println!("请输入地址列表文件路径: ");
                    let mut path = String::new();
                    std::io::stdin().read_line(&mut path)?;
                    let path = path.trim();
                    
                    match fs::read_to_string(path) {
                        Ok(content) => {
                            let mut count = 0;
                            for line in content.lines() {
                                let address = line.trim();
                                if !address.is_empty() {
                                    if let Ok(_) = Pubkey::from_str(address) {
                                        self.watch_addresses.insert(address.to_string());
                                        count += 1;
                                    }
                                }
                            }
                            self.save_watch_addresses()?;
                            println!("✅ 成功导入 {} 个地址", count);
                        }
                        Err(e) => println!("❌ 读取文件失败: {}", e),
                    }
                }
                "5" => {
                    let export_path = Path::new("exported_addresses.txt");
                    let content = self.watch_addresses.iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join("\n");
                        
                    fs::write(&export_path, content)?;
                    println!("✅ 地址已导出到: {}", export_path.display());
                }
                "6" => {
                    println!("请输入要查看的地址: ");
                    let mut address = String::new();
                    std::io::stdin().read_line(&mut address)?;
                    let address = address.trim();
                    
                    if let Ok(pubkey) = Pubkey::from_str(address) {
                        match self.analyze_creator_history(&pubkey).await {
                            Ok(history) => {
                                println!("\n地址详情:");
                                println!("历史发行代币: {} 个", history.total_tokens);
                                println!("成功项目: {} 个", history.success_tokens.len());
                                
                                if !history.success_tokens.is_empty() {
                                    println!("\n成功项目列表:");
                                    for token in &history.success_tokens {
                                        println!("- {} ({}) - 市值: ${:.2}M",
                                            token.name,
                                            token.symbol,
                                            token.market_cap / 1_000_000.0
                                        );
                                    }
                                }
                            }
                            Err(e) => println!("❌ 获取地址信息失败: {}", e),
                        }
                    } else {
                        println!("❌ 无效的Solana地址格式");
                    }
                }
                "7" => break,
                _ => println!("无效选项"),
            }
        }
        Ok(())
    }

    fn save_watch_addresses(&self) -> Result<()> {
        let watch_file = Path::new("watch_addresses.json");
        let content = serde_json::to_string_pretty(&self.watch_addresses)?;
        fs::write(watch_file, content)?;
        Ok(())
    }

    async fn manage_proxies(&mut self) -> Result<()> {
        println!("\n=== 代理管理 ===");
        println!("1. 查看当前代理");
        println!("2. 添加代理");
        println!("3. 删除代理");
        println!("4. 测试代理");
        println!("5. 返回主菜单");

        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;

        match input.trim() {
            "1" => {
                let pool = self.proxy_pool.lock().await;
                println!("\n当前代理列表:");
                for (i, proxy) in pool.proxies.iter().enumerate() {
                    println!("{}. {}:{}", i + 1, proxy.ip, proxy.port);
                }
            }
            "2" => {
                println!("请输入代理信息 (格式: ip:port:username:password):");
                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;
                let parts: Vec<&str> = input.trim().split(':').collect();
                if parts.len() == 4 {
                    let proxy = ProxyConfig {
                        enabled: true,
                        ip: parts[0].to_string(),
                        port: parts[1].parse()?,
                        username: parts[2].to_string(),
                        password: parts[3].to_string(),
                    };
                    let mut pool = self.proxy_pool.lock().await;
                    pool.proxies.push(proxy);
                    println!("代理添加成功");
                } else {
                    println!("格式错误");
                }
            }
            "3" => {
                println!("请输入要删除的代理序号:");
                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;
                if let Ok(index) = input.trim().parse::<usize>() {
                    let mut pool = self.proxy_pool.lock().await;
                    if index > 0 && index <= pool.proxies.len() {
                        pool.proxies.remove(index - 1);
                        println!("代理删除成功");
                    } else {
                        println!("无效的序号");
                    }
                }
            }
            "4" => {
                println!("开始测试代理...");
                let mut pool = self.proxy_pool.lock().await;
                pool.check_proxies().await;
                println!("代理测试完成");
            }
            "5" => return Ok(()),
            _ => println!("无效选项"),
        }

        Ok(())
    }

    fn load_proxy_list() -> Result<Vec<ProxyConfig>> {
        let proxy_file = dirs::home_dir()
            .ok_or_else(|| anyhow!("Cannot find home directory"))?
            .join(".solana_pump/proxies.json");

        if !proxy_file.exists() {
            return Ok(Vec::new());
        }

        let content = fs::read_to_string(proxy_file)?;
        Ok(serde_json::from_str(&content)?)
    }

    fn generate_config_files() -> Result<()> {
        let home = dirs::home_dir()
            .ok_or_else(|| anyhow!("Cannot find home directory"))?;
        
        let config_dir = home.join(".solana_pump");
        fs::create_dir_all(&config_dir)?;

        let config = Config {
            api_keys: vec!["your_api_key_here".to_string()],
            serverchan: ServerChanConfig {
                keys: vec!["your_serverchan_key_here".to_string()],
            },
            wcf: WeChatFerryConfig {
                groups: vec![WeChatGroup {
                    name: "test_group".to_string(),
                    wxid: "test_wxid".to_string(),
                }],
            },
            proxy: ProxyConfig {
                enabled: false,
                ip: "127.0.0.1".to_string(),
                port: 1080,
                username: "user".to_string(),
                password: "pass".to_string(),
            },
            rpc_nodes: HashMap::new(),
        };

        fs::write(
            config_dir.join("config.json"),
            serde_json::to_string_pretty(&config)?,
        )?;

        let proxy_list = vec![ProxyConfig {
            enabled: true,
            ip: "proxy.example.com".to_string(),
            port: 1080,
            username: "user".to_string(),
            password: "pass".to_string(),
        }];

        fs::write(
            config_dir.join("proxies.json"),
            serde_json::to_string_pretty(&proxy_list)?,
        )?;

        let watch_addresses = HashSet::new();
        fs::write(
            config_dir.join("watch_addresses.json"),
            serde_json::to_string_pretty(&watch_addresses)?,
        )?;

        println!("配置文件已生成在: {}", config_dir.display());
        Ok(())
    }

    async fn manage_logs(&self) -> Result<()> {
        println!("\n=== 日志管理 ===");
        println!("1. 查看日志文件");
        println!("2. 清理旧日志");
        println!("3. 设置日志级别");
        println!("4. 返回主菜单");
        println!("请选择功能 (1-4): ");

        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;

        match input.trim() {
            "1" => {
                let log_dir = dirs::home_dir()?.join(".solana_pump/logs");
                println!("\n日志文件列表:");
                for entry in fs::read_dir(log_dir)? {
                    let entry = entry?;
                    println!("{}", entry.file_name().to_string_lossy());
                }
            }
            "2" => {
                let log_dir = dirs::home_dir()?.join(".solana_pump/logs");
                let mut count = 0;
                for entry in fs::read_dir(log_dir)? {
                    let entry = entry?;
                    let path = entry.path();
                    if path.is_file() && path.to_string_lossy().contains(".log.") {
                        fs::remove_file(path)?;
                        count += 1;
                    }
                }
                println!("已清理 {} 个旧日志文件", count);
            }
            "3" => {
                println!("请选择日志级别 (1: ERROR, 2: WARN, 3: INFO, 4: DEBUG):");
                let mut level = String::new();
                std::io::stdin().read_line(&mut level)?;
                
                let level_filter = match level.trim() {
                    "1" => log::LevelFilter::Error,
                    "2" => log::LevelFilter::Warn,
                    "3" => log::LevelFilter::Info,
                    "4" => log::LevelFilter::Debug,
                    _ => {
                        println!("无效的日志级别");
                        return Ok(());
                    }
                };

                // 重新配置日志级别
                let log_dir = dirs::home_dir()?.join(".solana_pump/logs");
                let config = Config::builder()
                    .appender(
                        Appender::builder().build(
                            "rolling",
                            Box::new(
                                RollingFileAppender::builder()
                                    .encoder(Box::new(PatternEncoder::new(
                                        "{d(%Y-%m-%d %H:%M:%S)} {l} [{T}] {m}{n}"
                                    )))
                                    .build(
                                        log_dir.join("solana_pump.log"),
                                        Box::new(CompoundPolicy::new(
                                            Box::new(SizeTrigger::new(10 * 1024 * 1024)),
                                            Box::new(
                                                FixedWindowRoller::builder()
                                                    .build(
                                                        log_dir.join("solana_pump.{}.log").to_str().unwrap(),
                                                        5,
                                                    )?
                                            ),
                                        )),
                                    )?
                            )
                        )
                    )
                    .build(Root::builder().appender("rolling").build(level_filter))?;

                log4rs::init_config(config)?;
                println!("日志级别已更新");
            }
            "4" => return Ok(()),
            _ => println!("无效选项"),
        }

        Ok(())
    }

    fn test_serverchan(&self) {
        println!("\n{}", ">>> 测试Server酱通知...".yellow());
        
        // 模拟一个发现的代币数据
        let mock_token = TokenInfo {
            mint: "DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263".parse().unwrap(),
            name: "Solana Monkey Business".to_string(),
            symbol: "SMB".to_string(),
            market_cap: 15_000_000.0,
            liquidity: 2500.0,
            holder_count: 5823,
            holder_concentration: 35.8,
            verified: true,
            price: 0.00145,
            supply: 5000_000_000,
            creator: "7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU".parse().unwrap(),
        };

        // 模拟创建者历史
        let mock_creator_history = CreatorHistory {
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
        };

        // 模拟资金流动
        let mock_fund_flow = vec![
            FundingChain {
                transfers: vec![
                    Transfer {
                        source: "DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263".parse().unwrap(),
                        amount: 1250.5,
                        timestamp: 1711008000, // 2024-03-21 12:00:00
                        tx_id: "5KtPn1LGuxhFqnXGKxgVPJ6eXrec8LD6ENxgfvzewZFwRBpfnyaQYKCYXgYjkKxVGvnkxhQp".to_string(),
                        success_tokens: Some(vec![
                            SuccessToken {
                                address: "7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU".parse().unwrap(),
                                symbol: "SAMO".to_string(),
                                name: "Samoyedcoin".to_string(),
                                market_cap: 25_000_000.0,
                                created_at: 1640995200,
                            }
                        ]),
                    }
                ],
                total_amount: 1250.5,
                risk_score: 25,
            }
        ];

        let mock_analysis = TokenAnalysis {
            token_info: mock_token,
            creator_history: mock_creator_history,
            fund_flow: mock_fund_flow,
            risk_score: 35,
            is_new_wallet: false,
            wallet_age: 245.5,
        };

        // 生成通知消息
        let message = self.format_message(&mock_analysis);
        
        // 添加模拟测试标记
        let test_message = format!(
            "[⚠️ 这是一条模拟测试消息]\n测试时间: {}\n--------------------------------\n\n{}",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
            message
        );
        
        for key in &self.config.serverchan.keys {
            println!("\n{} Server酱密钥: {}...{}", ">>>".yellow(), &key[..8], &key[key.len()-8..]);
            
            println!("{}", "模拟请求内容:".blue());
            println!("URL: https://sctapi.ftqq.com/{}.send", key);
            println!("参数:");
            println!("  - title: [模拟测试] Solana新代币提醒");
            println!("  - desp: {}", test_message);
            
            println!("\n{}", "模拟响应:".blue());
            println!("{{");
            println!("    \"code\": 0,");
            println!("    \"message\": \"\",");
            println!("    \"data\": {{");
            println!("        \"pushid\": \"mock-xxxxx\",");
            println!("        \"readkey\": \"mock-xxxxx\",");
            println!("        \"error\": \"SUCCESS\",");
            println!("        \"errno\": 0");
            println!("    }}");
            println!("}}");
            
            println!("\n{}", "✓ 模拟发送成功".green());
        }
        
        if self.config.serverchan.keys.is_empty() {
            println!("{}", "没有配置Server酱密钥".yellow());
        }
    }
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    data: TokenData,
}

#[derive(Debug, Deserialize)]
struct TokenData {
    mint: Pubkey,
    name: String,
    symbol: String,
    market_cap: f64,
    liquidity: f64,
    holder_count: u64,
    holder_concentration: f64,
    verified: bool,
    price: f64,
    supply: u64,
    creator: Pubkey,
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

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    let mut monitor = TokenMonitor::new().await?;
    loop {
        if let Err(e) = monitor.show_menu().await {
            log::error!("菜单错误: {}", e);
        }
    }
} 
