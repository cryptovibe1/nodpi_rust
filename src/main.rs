use chrono::Local;
use rand::Rng;
use reqwest::Client;
use serde_json::Value;
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write as IoWrite};
use std::net::SocketAddr;
use std::path::Path;
#[cfg(any(windows, target_os = "linux"))]
use std::process::Command;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{lookup_host, TcpListener, TcpSocket, TcpStream};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::{Mutex, Notify};
use tokio::time::{self, Duration, Instant};

#[cfg(windows)]
use winreg::enums::*;
#[cfg(windows)]
use winreg::RegKey;

const VERSION: &str = "2.1";
const UPDATE_URL: &str = "https://gvcoder09.github.io/nodpi_site/api/v1/update_info.json";

#[derive(Clone, Copy, PartialEq, Eq)]
enum FragmentMethod {
    Random,
    Sni,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DomainMatching {
    Strict,
    Loose,
}

#[derive(Clone)]
struct Config {
    host: String,
    port: u16,
    out_host: Option<String>,
    blacklist_file: String,
    fragment_method: FragmentMethod,
    domain_matching: DomainMatching,
    log_access_file: Option<String>,
    log_error_file: Option<String>,
    no_blacklist: bool,
    auto_blacklist: bool,
    quiet: bool,
}

struct Args {
    config: Config,
    install: bool,
    uninstall: bool,
}

struct Logger {
    error: Option<Mutex<File>>,
    access: Option<Mutex<File>>,
    quiet: bool,
    error_counter: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

impl Logger {
    fn new(log_access: &Option<String>, log_error: &Option<String>, quiet: bool) -> io::Result<Self> {
        let error = if let Some(path) = log_error {
            Some(Mutex::new(open_append(path)?))
        } else {
            None
        };

        let access = if let Some(path) = log_access {
            Some(Mutex::new(open_append(path)?))
        } else {
            None
        };

        Ok(Self {
            error,
            access,
            quiet,
            error_counter: Mutex::new(None),
        })
    }

    async fn set_error_counter_callback(&self, callback: Arc<dyn Fn() + Send + Sync>) {
        let mut guard = self.error_counter.lock().await;
        *guard = Some(callback);
    }

    async fn log_error(&self, message: &str) {
        if let Some(file) = &self.error {
            let mut f = file.lock().await;
            let ts = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
            let _ = writeln!(f, "[{}][ERROR]: {}", ts, message);
        }
        let callback = { self.error_counter.lock().await.clone() };
        if let Some(cb) = callback {
            cb();
        }
    }

    async fn log_access(&self, line: &str) {
        if let Some(file) = &self.access {
            let mut f = file.lock().await;
            let _ = writeln!(f, "{}", line);
        }
    }

    fn error(&self, message: &str) {
        if !self.quiet {
            println!("{}", message);
        }
    }
}

#[derive(Clone)]
struct ConnectionInfo {
    src_ip: String,
    dst_domain: String,
    method: String,
    start_time: String,
    traffic_in: u64,
    traffic_out: u64,
}

struct StatisticsState {
    total_connections: u64,
    allowed_connections: u64,
    blocked_connections: u64,
    errors_connections: u64,
    traffic_in: u64,
    traffic_out: u64,
    last_traffic_in: u64,
    last_traffic_out: u64,
    speed_in: f64,
    speed_out: f64,
    average_speed_in: (f64, f64),
    average_speed_out: (f64, f64),
    last_time: Option<Instant>,
}

struct Statistics {
    inner: Mutex<StatisticsState>,
}

impl Statistics {
    fn new() -> Self {
        Self {
            inner: Mutex::new(StatisticsState {
                total_connections: 0,
                allowed_connections: 0,
                blocked_connections: 0,
                errors_connections: 0,
                traffic_in: 0,
                traffic_out: 0,
                last_traffic_in: 0,
                last_traffic_out: 0,
                speed_in: 0.0,
                speed_out: 0.0,
                average_speed_in: (0.0, 1.0),
                average_speed_out: (0.0, 1.0),
                last_time: None,
            }),
        }
    }

    async fn increment_total_connections(&self) {
        let mut state = self.inner.lock().await;
        state.total_connections += 1;
    }

    async fn increment_allowed_connections(&self) {
        let mut state = self.inner.lock().await;
        state.allowed_connections += 1;
    }

    async fn increment_blocked_connections(&self) {
        let mut state = self.inner.lock().await;
        state.blocked_connections += 1;
    }

    async fn increment_error_connections(&self) {
        let mut state = self.inner.lock().await;
        state.errors_connections += 1;
    }

    async fn update_traffic(&self, incoming: u64, outgoing: u64) {
        let mut state = self.inner.lock().await;
        state.traffic_in += incoming;
        state.traffic_out += outgoing;
    }

    async fn update_speeds(&self) {
        let mut state = self.inner.lock().await;
        let now = Instant::now();
        if let Some(prev) = state.last_time {
            let diff = now.duration_since(prev).as_secs_f64();
            if diff > 0.0 {
                state.speed_in = (state.traffic_in - state.last_traffic_in) as f64 * 8.0 / diff;
                state.speed_out = (state.traffic_out - state.last_traffic_out) as f64 * 8.0 / diff;

                if state.speed_in > 0.0 {
                    state.average_speed_in.0 += state.speed_in;
                    state.average_speed_in.1 += 1.0;
                }
                if state.speed_out > 0.0 {
                    state.average_speed_out.0 += state.speed_out;
                    state.average_speed_out.1 += 1.0;
                }
            }
        }

        state.last_traffic_in = state.traffic_in;
        state.last_traffic_out = state.traffic_out;
        state.last_time = Some(now);
    }

    async fn get_stats_display(&self) -> String {
        let state = self.inner.lock().await;
        let col_width = 30usize;

        let conns_stat = format!(
            "\x1b[97mTotal: \x1b[93m{}\x1b[0m",
            state.total_connections
        )
        .ljust(col_width)
            + "\x1b[97m| "
            + format!(
                "\x1b[97mMiss: \x1b[96m{}\x1b[0m",
                state.allowed_connections
            )
            .ljust(col_width)
            .as_str()
            + "\x1b[97m| "
            + format!(
                "\x1b[97mUnblock: \x1b[92m{}\x1b[0m",
                state.blocked_connections
            )
            .ljust(col_width)
            .as_str()
            + "\x1b[97m| "
            + format!(
                "\x1b[97mErrors: \x1b[91m{}\x1b[0m",
                state.errors_connections
            )
            .ljust(col_width)
            .as_str();

        let traffic_stat = format!(
            "\x1b[97mTotal: \x1b[96m{}\x1b[0m",
            format_size(state.traffic_out + state.traffic_in)
        )
        .ljust(col_width)
            + "\x1b[97m| "
            + format!(
                "\x1b[97mDL: \x1b[96m{}\x1b[0m",
                format_size(state.traffic_in)
            )
            .ljust(col_width)
            .as_str()
            + "\x1b[97m| "
            + format!(
                "\x1b[97mUL: \x1b[96m{}\x1b[0m",
                format_size(state.traffic_out)
            )
            .ljust(col_width)
            .as_str()
            + "\x1b[97m| ";

        let avg_speed_in = if state.average_speed_in.1 > 0.0 {
            state.average_speed_in.0 / state.average_speed_in.1
        } else {
            0.0
        };
        let avg_speed_out = if state.average_speed_out.1 > 0.0 {
            state.average_speed_out.0 / state.average_speed_out.1
        } else {
            0.0
        };

        let speed_stat = format!(
            "\x1b[97mDL: \x1b[96m{}\x1b[0m",
            format_speed(state.speed_in)
        )
        .ljust(col_width)
            + "\x1b[97m| "
            + format!(
                "\x1b[97mUL: \x1b[96m{}\x1b[0m",
                format_speed(state.speed_out)
            )
            .ljust(col_width)
            .as_str()
            + "\x1b[97m| "
            + format!(
                "\x1b[97mAVG DL: \x1b[96m{}\x1b[0m",
                format_speed(avg_speed_in)
            )
            .ljust(col_width)
            .as_str()
            + "\x1b[97m| "
            + format!(
                "\x1b[97mAVG UL: \x1b[96m{}\x1b[0m",
                format_speed(avg_speed_out)
            )
            .ljust(col_width)
            .as_str();

        let title = "STATISTICS";
        let top_border = format!("\x1b[92m{} {} {}\x1b[0m", "═".repeat(36), title, "═".repeat(36));
        let line_conns = format!("\x1b[92m   {:<8}:\x1b[0m {}\x1b[0m", "Conns", conns_stat);
        let line_traffic = format!("\x1b[92m   {:<8}:\x1b[0m {}\x1b[0m", "Traffic", traffic_stat);
        let line_speed = format!("\x1b[92m   {:<8}:\x1b[0m {}\x1b[0m", "Speed", speed_stat);
        let bottom_border = format!("\x1b[92m{}\x1b[0m", "═".repeat(36 * 2 + title.len() + 2));

        format!("{}\n{}\n{}\n{}\n{}", top_border, line_conns, line_traffic, line_speed, bottom_border)
    }
}

enum BlacklistManager {
    None,
    File {
        blocked: Vec<String>,
        domain_matching: DomainMatching,
    },
    Auto {
        blocked: Mutex<Vec<String>>,
        whitelist: Mutex<Vec<String>>,
        blacklist_file: String,
    },
}

impl BlacklistManager {
    async fn is_blocked(&self, domain: &str) -> bool {
        match self {
            BlacklistManager::None => true,
            BlacklistManager::File {
                blocked,
                domain_matching,
            } => is_domain_blocked(blocked, *domain_matching, domain),
            BlacklistManager::Auto { blocked, .. } => {
                let guard = blocked.lock().await;
                guard.contains(&domain.to_string())
            }
        }
    }

    async fn check_domain(&self, domain: &str) {
        if let BlacklistManager::Auto {
            blocked,
            whitelist,
            blacklist_file,
        } = self
        {
            {
                let b = blocked.lock().await;
                if b.contains(&domain.to_string()) {
                    return;
                }
            }
            {
                let w = whitelist.lock().await;
                if w.contains(&domain.to_string()) {
                    return;
                }
            }

            let client = match Client::builder()
                .danger_accept_invalid_certs(true)
                .timeout(Duration::from_secs(4))
                .user_agent("Mozilla/5.0")
                .build()
            {
                Ok(c) => c,
                Err(_) => return,
            };

            let url = format!("https://{}", domain);
            let result = client.get(url).send().await;
            match result {
                Ok(_) => {
                    let mut w = whitelist.lock().await;
                    w.push(domain.to_string());
                }
                Err(err) => {
                    if err.is_timeout() {
                        {
                            let mut b = blocked.lock().await;
                            b.push(domain.to_string());
                        }
                        let _ = append_line(blacklist_file, domain);
                    }
                }
            }
        }
    }
}

struct ConnectionHandler {
    config: Config,
    blacklist_manager: Arc<BlacklistManager>,
    statistics: Arc<Statistics>,
    logger: Arc<Logger>,
    active_connections: Arc<Mutex<HashMap<String, ConnectionInfo>>>,
    tasks: Mutex<Vec<tokio::task::JoinHandle<()>>>,
}

impl ConnectionHandler {
    fn new(
        config: Config,
        blacklist_manager: Arc<BlacklistManager>,
        statistics: Arc<Statistics>,
        logger: Arc<Logger>,
    ) -> Self {
        Self {
            config,
            blacklist_manager,
            statistics,
            logger,
            active_connections: Arc::new(Mutex::new(HashMap::new())),
            tasks: Mutex::new(Vec::new()),
        }
    }

    async fn handle_connection(&self, mut client: TcpStream) {
        let peer = client
            .peer_addr()
            .map(|addr| addr.to_string())
            .unwrap_or_else(|_| "unknown".to_string());

        let mut buf = vec![0u8; 1500];
        let n = match client.read(&mut buf).await {
            Ok(n) => n,
            Err(_) => return,
        };
        if n == 0 {
            let _ = client.shutdown().await;
            return;
        }

        let http_data = buf[..n].to_vec();
        let (method, host, port) = match parse_http_request(&http_data) {
            Ok(v) => v,
            Err(err) => {
                self.handle_connection_error(&mut client, &peer, err).await;
                return;
            }
        };

        if method == "CONNECT" {
            self.blacklist_manager.check_domain(&host).await;
        }

        let conn_info = ConnectionInfo {
            src_ip: peer.clone(),
            dst_domain: host.clone(),
            method: method.clone(),
            start_time: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            traffic_in: 0,
            traffic_out: 0,
        };
        {
            let mut map = self.active_connections.lock().await;
            map.insert(peer.clone(), conn_info);
        }

        let _ = self.statistics.update_traffic(0, http_data.len() as u64).await;
        self.update_conn_out(&peer, http_data.len() as u64).await;

        if method == "CONNECT" {
            self.handle_https_connection(client, host, port, &peer).await;
        } else {
            self.handle_http_connection(client, http_data, host, port, &peer).await;
        }
    }

    async fn handle_https_connection(
        &self,
        mut client: TcpStream,
        host: String,
        port: u16,
        conn_key: &str,
    ) {
        let response = b"HTTP/1.1 200 Connection Established\r\n\r\n";
        if client.write_all(response).await.is_err() {
            return;
        }
        let _ = self.statistics.update_traffic(response.len() as u64, 0).await;
        self.update_conn_in(conn_key, response.len() as u64).await;

        let dst = match connect_with_out_host(&host, port, &self.config.out_host).await {
            Ok(s) => s,
            Err(err) => {
                let _ = self.logger.log_error(&format!("{}: {}", host, err)).await;
                return;
            }
        };

        let (mut client_reader, client_writer) = client.into_split();
        let (dst_reader, mut dst_writer) = dst.into_split();

        if let Err(err) = self
            .handle_initial_tls_data(&mut client_reader, &mut dst_writer, &host, conn_key)
            .await
        {
            let _ = self.logger.log_error(&format!("{}: {}", host, err)).await;
        }

        self.setup_piping(client_reader, client_writer, dst_reader, dst_writer, conn_key)
            .await;
    }

    async fn handle_http_connection(
        &self,
        mut client: TcpStream,
        http_data: Vec<u8>,
        host: String,
        port: u16,
        conn_key: &str,
    ) {
        let mut dst = match connect_with_out_host(&host, port, &self.config.out_host).await {
            Ok(s) => s,
            Err(err) => {
                let _ = self.logger.log_error(&format!("{}: {}", host, err)).await;
                let _ = client
                    .write_all(b"HTTP/1.1 500 Internal Server Error\r\n\r\n")
                    .await;
                return;
            }
        };

        if dst.write_all(&http_data).await.is_err() {
            return;
        }

        self.statistics.increment_total_connections().await;
        self.statistics.increment_allowed_connections().await;

        let (client_reader, client_writer) = client.into_split();
        let (dst_reader, dst_writer) = dst.into_split();
        self.setup_piping(client_reader, client_writer, dst_reader, dst_writer, conn_key)
            .await;
    }

    async fn handle_initial_tls_data(
        &self,
        reader: &mut OwnedReadHalf,
        writer: &mut OwnedWriteHalf,
        host: &str,
        conn_key: &str,
    ) -> io::Result<()> {
        let mut head = [0u8; 5];
        reader.read_exact(&mut head).await?;

        let mut data = vec![0u8; 2048];
        let n = reader.read(&mut data).await?;
        data.truncate(n);

        let mut should_fragment = true;
        if !matches!(&*self.blacklist_manager, BlacklistManager::None) {
            should_fragment = self.blacklist_manager.is_blocked(host).await;
        }

        if !should_fragment {
            self.statistics.increment_total_connections().await;
            self.statistics.increment_allowed_connections().await;
            let combined = [head.to_vec(), data.clone()].concat();
            writer.write_all(&combined).await?;
            self.statistics.update_traffic(0, combined.len() as u64).await;
            self.update_conn_out(conn_key, combined.len() as u64).await;
            return Ok(());
        }

        self.statistics.increment_total_connections().await;
        self.statistics.increment_blocked_connections().await;

        let parts = match self.config.fragment_method {
            FragmentMethod::Random => fragment_random(&data),
            FragmentMethod::Sni => fragment_sni(&data),
        };

        writer.write_all(&parts).await?;
        self.statistics.update_traffic(0, parts.len() as u64).await;
        self.update_conn_out(conn_key, parts.len() as u64).await;
        Ok(())
    }

    async fn setup_piping(
        &self,
        client_reader: OwnedReadHalf,
        client_writer: OwnedWriteHalf,
        remote_reader: OwnedReadHalf,
        remote_writer: OwnedWriteHalf,
        conn_key: &str,
    ) {
        let key = conn_key.to_string();
        let handler = Arc::new(self.clone_for_pipe());
        let out_task = tokio::spawn(pipe_data(
            handler.clone(),
            client_reader,
            remote_writer,
            PipeDirection::Out,
            key.clone(),
        ));
        let in_task = tokio::spawn(pipe_data(
            handler.clone(),
            remote_reader,
            client_writer,
            PipeDirection::In,
            key.clone(),
        ));

        let mut tasks = self.tasks.lock().await;
        tasks.push(out_task);
        tasks.push(in_task);
    }

    async fn cleanup_tasks(&self) {
        loop {
            time::sleep(Duration::from_secs(60)).await;
            let mut tasks = self.tasks.lock().await;
            tasks.retain(|t| !t.is_finished());
        }
    }

    async fn handle_connection_error(&self, writer: &mut TcpStream, conn_key: &str, err: String) {
        let response = b"HTTP/1.1 500 Internal Server Error\r\n\r\n";
        let _ = writer.write_all(response).await;
        self.statistics.update_traffic(response.len() as u64, 0).await;
        self.statistics.increment_total_connections().await;
        self.statistics.increment_error_connections().await;
        self.logger.log_error(&err).await;

        let info = {
            let mut map = self.active_connections.lock().await;
            map.remove(conn_key)
        };
        if let Some(info) = info {
            let line = format!(
                "{} {} {} {} {} {}",
                info.start_time,
                info.src_ip,
                info.method,
                info.dst_domain,
                info.traffic_in,
                info.traffic_out
            );
            self.logger.log_access(&line).await;
        }
    }

    async fn update_conn_in(&self, conn_key: &str, n: u64) {
        let mut map = self.active_connections.lock().await;
        if let Some(info) = map.get_mut(conn_key) {
            info.traffic_in += n;
        }
    }

    async fn update_conn_out(&self, conn_key: &str, n: u64) {
        let mut map = self.active_connections.lock().await;
        if let Some(info) = map.get_mut(conn_key) {
            info.traffic_out += n;
        }
    }

    fn clone_for_pipe(&self) -> PipeContext {
        PipeContext {
            statistics: Arc::clone(&self.statistics),
            logger: Arc::clone(&self.logger),
            active_connections: Arc::clone(&self.active_connections),
        }
    }
}

struct PipeContext {
    statistics: Arc<Statistics>,
    logger: Arc<Logger>,
    active_connections: Arc<Mutex<HashMap<String, ConnectionInfo>>>,
}

#[derive(PartialEq, Eq)]
enum PipeDirection {
    In,
    Out,
}

async fn pipe_data(
    ctx: Arc<PipeContext>,
    mut reader: OwnedReadHalf,
    mut writer: OwnedWriteHalf,
    direction: PipeDirection,
    conn_key: String,
) {
    let mut buf = vec![0u8; 1500];
    loop {
        let n = match reader.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };
        if direction == PipeDirection::Out {
            ctx.statistics.update_traffic(0, n as u64).await;
        } else {
            ctx.statistics.update_traffic(n as u64, 0).await;
        }
        {
            let mut map = ctx.active_connections.lock().await;
            if let Some(info) = map.get_mut(&conn_key) {
                if direction == PipeDirection::Out {
                    info.traffic_out += n as u64;
                } else {
                    info.traffic_in += n as u64;
                }
            }
        }
        if writer.write_all(&buf[..n]).await.is_err() {
            break;
        }
    }
    let _ = writer.shutdown().await;

    let info = {
        let mut map = ctx.active_connections.lock().await;
        map.remove(&conn_key)
    };
    if let Some(info) = info {
        let line = format!(
            "{} {} {} {} {} {}",
            info.start_time,
            info.src_ip,
            info.method,
            info.dst_domain,
            info.traffic_in,
            info.traffic_out
        );
        ctx.logger.log_access(&line).await;
    }
}

struct ProxyServer {
    config: Config,
    blacklist_manager: Arc<BlacklistManager>,
    statistics: Arc<Statistics>,
    logger: Arc<Logger>,
    connection_handler: Arc<ConnectionHandler>,
    shutdown: Arc<Notify>,
}

impl ProxyServer {
    fn new(config: Config, blacklist_manager: Arc<BlacklistManager>, statistics: Arc<Statistics>, logger: Arc<Logger>) -> Self {
        let connection_handler = Arc::new(ConnectionHandler::new(
            config.clone(),
            Arc::clone(&blacklist_manager),
            Arc::clone(&statistics),
            Arc::clone(&logger),
        ));
        Self {
            config,
            blacklist_manager,
            statistics,
            logger,
            connection_handler,
            shutdown: Arc::new(Notify::new()),
        }
    }

    async fn check_for_updates(&self) -> Option<String> {
        if self.config.quiet {
            return None;
        }
        let client = Client::builder()
            .timeout(Duration::from_secs(3))
            .user_agent("Mozilla/5.0")
            .build()
            .ok()?;

        let response = client.get(UPDATE_URL).send().await.ok()?;
        if response.status() != 200 {
            return None;
        }
        let json: Value = response.json().await.ok()?;
        let latest = json
            .get("nodpi")
            .and_then(|v| v.get("latest_version"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !latest.is_empty() && latest != VERSION {
            return Some(format!("\x1b[93m[UPDATE]: Available new version: v{} \x1b[97m", latest));
        }
        None
    }

    async fn print_banner(&self) {
        if self.config.quiet {
            return;
        }

        let update_message = match time::timeout(Duration::from_secs(2), self.check_for_updates()).await {
            Ok(msg) => msg,
            Err(_) => None,
        };

        print!("\x1b]0;NoDPI\x07");

        #[cfg(windows)]
        {
            let _ = Command::new("cmd").args(["/C", "mode con: lines=33"]).status();
        }

        let console_width = 80;

        let disclaimer = "DISCLAIMER. The developer and/or supplier of this software shall not be liable for any loss or damage, including but not limited to direct, indirect, incidental, punitive or consequential damages arising out of the use of or inability to use this software, even if the developer or supplier has been advised of the possibility of such damages. The developer and/or supplier of this software shall not be liable for any legal consequences arising out of the use of this software. This includes, but is not limited to, violation of laws, rules or regulations, as well as any claims or suits arising out of the use of this software. The user is solely responsible for compliance with all applicable laws and regulations when using this software.";
        let wrapped = wrap_text(disclaimer, 70);
        let left_padding = if console_width > 76 { (console_width - 76) / 2 } else { 0 };

        println!("\n\n\n");
        println!("\x1b[91m{}╔{}╗\x1b[0m", " ".repeat(left_padding), "═".repeat(72));
        for line in wrapped {
            println!(
                "\x1b[91m{}║ {} ║\x1b[0m",
                " ".repeat(left_padding),
                format!("{:<70}", line)
            );
        }
        println!("\x1b[91m{}╚{}╝\x1b[0m", " ".repeat(left_padding), "═".repeat(72));

        time::sleep(Duration::from_secs(1)).await;

        print!("\x1b[2J\x1b[H");

        println!(
            "\n\x1b[92m  ██████   █████          ██████████   ███████████  █████
  ░░██████ ░░███          ░░███░░░░███ ░░███░░░░░███░░███
   ░███░███ ░███   ██████  ░███   ░░███ ░███    ░███ ░███
   ░███░░███░███  ███░░███ ░███    ░███ ░██████████  ░███
   ░███ ░░██████ ░███ ░███ ░███    ░███ ░███░░░░░░   ░███
   ░███  ░░█████ ░███ ░███ ░███    ███  ░███         ░███
   █████  ░░█████░░██████  ██████████   █████        █████
  ░░░░░    ░░░░░  ░░░░░░  ░░░░░░░░░░   ░░░░░        ░░░░░\x1b[0m\n"
        );

        println!("\x1b[92mVersion: {}\x1b[0m", VERSION);
        println!("\x1b[97m{}\x1b[0m", "Enjoy watching! / Наслаждайтесь просмотром!");
        println!("");

        if let Some(msg) = update_message {
            println!("{}", msg);
        }

        println!(
            "\x1b[92m[INFO]:\x1b[97m Proxy is running on {}:{} at {}",
            self.config.host,
            self.config.port,
            Local::now().format("%H:%M on %Y-%m-%d")
        );
        println!(
            "\x1b[92m[INFO]:\x1b[97m The selected fragmentation method: {}",
            match self.config.fragment_method {
                FragmentMethod::Random => "random",
                FragmentMethod::Sni => "sni",
            }
        );

        println!("");
        if matches!(&*self.blacklist_manager, BlacklistManager::None) {
            println!("\x1b[92m[INFO]:\x1b[97m Blacklist is disabled. All domains will be subject to unblocking.");
        } else if matches!(&*self.blacklist_manager, BlacklistManager::Auto { .. }) {
            println!("\x1b[92m[INFO]:\x1b[97m Auto-blacklist is enabled");
        } else {
            let count = match &*self.blacklist_manager {
                BlacklistManager::File { blocked, .. } => blocked.len(),
                _ => 0,
            };
            println!(
                "\x1b[92m[INFO]:\x1b[97m Blacklist contains {} domains",
                count
            );
            println!(
                "\x1b[92m[INFO]:\x1b[97m Path to blacklist: '{}'",
                std::path::Path::new(&self.config.blacklist_file).display()
            );
        }

        println!("");
        if let Some(path) = &self.config.log_error_file {
            println!(
                "\x1b[92m[INFO]:\x1b[97m Error logging is enabled. Path to error log: '{}'",
                path
            );
        } else {
            println!("\x1b[92m[INFO]:\x1b[97m Error logging is disabled");
        }

        if let Some(path) = &self.config.log_access_file {
            println!(
                "\x1b[92m[INFO]:\x1b[97m Access logging is enabled. Path to access log: '{}'",
                path
            );
        } else {
            println!("\x1b[92m[INFO]:\x1b[97m Access logging is disabled");
        }

        println!("");
        println!("\x1b[92m[INFO]:\x1b[97m To stop the proxy, press Ctrl+C twice");
        println!("");
    }


    async fn run(&self) -> io::Result<()> {
        if !self.config.quiet {
            self.print_banner().await;
        }

        let addr = format!("{}:{}", self.config.host, self.config.port);
        let listener = match TcpListener::bind(&addr).await {
            Ok(l) => l,
            Err(_) => {
                self.logger.error(&format!(
                    "\x1b[91m[ERROR]: Failed to start proxy on this address ({}:{}). It looks like the port is already in use\x1b[0m",
                    self.config.host, self.config.port
                ));
                return Err(io::Error::new(io::ErrorKind::AddrInUse, "bind failed"));
            }
        };

        let handler = Arc::clone(&self.connection_handler);
        tokio::spawn(async move {
            handler.cleanup_tasks().await;
        });

        if !self.config.quiet {
            let stats = self.statistics.clone();
            let config = self.config.clone();
            tokio::spawn(async move {
                if !config.quiet {
                    loop {
                        time::sleep(Duration::from_secs(1)).await;
                        stats.update_speeds().await;
                        if !config.quiet {
                            let display = stats.get_stats_display().await;
                            println!("{}", display);
                            print!("\x1b[5A");
                        }
                    }
                }
            });
        }

        loop {
            tokio::select! {
                _ = self.shutdown.notified() => break,
                res = listener.accept() => {
                    match res {
                        Ok((conn, _)) => {
                            let handler = Arc::clone(&self.connection_handler);
                            tokio::spawn(async move {
                                handler.handle_connection(conn).await;
                            });
                        }
                        Err(_) => continue,
                    }
                }
            }
        }

        Ok(())
    }

}

fn parse_http_request(data: &[u8]) -> Result<(String, String, u16), String> {
    let headers: Vec<&[u8]> = data.split(|b| *b == b'\n').collect();
    if headers.is_empty() {
        return Err("Missing request line".to_string());
    }
    let first_line = headers[0];
    let parts: Vec<&[u8]> = first_line.split(|b| *b == b' ').collect();
    if parts.len() < 2 {
        return Err("Invalid request line".to_string());
    }
    let method = String::from_utf8_lossy(parts[0]).to_string();
    let url = String::from_utf8_lossy(parts[1]).to_string();

    if method == "CONNECT" {
        let hp: Vec<&str> = url.split(':').collect();
        let host = hp.get(0).unwrap_or(&"").to_string();
        let port = hp.get(1).and_then(|p| p.parse::<u16>().ok()).unwrap_or(443);
        return Ok((method, host, port));
    }

    let mut host_header = None;
    for line in headers.iter().skip(1) {
        if line.starts_with(b"Host:") {
            host_header = Some(line);
            break;
        }
    }
    let host_header = host_header.ok_or_else(|| "Missing Host header".to_string())?;
    let host_value = String::from_utf8_lossy(&host_header[5..]).trim().to_string();
    let hp: Vec<&str> = host_value.split(':').collect();
    let host = hp.get(0).unwrap_or(&"").to_string();
    let port = hp.get(1).and_then(|p| p.parse::<u16>().ok()).unwrap_or(80);

    Ok((method, host, port))
}

fn extract_sni_position(data: &[u8]) -> Option<(usize, usize)> {
    let mut i = 0usize;
    while i + 8 < data.len() {
        if data[i] == 0
            && data[i + 1] == 0
            && data[i + 2] == 0
            && data[i + 4] == 0
            && data[i + 6] == 0
            && data[i + 7] == 0
        {
            let ext_len = data[i + 3] as usize;
            let server_name_list_len = data[i + 5] as usize;
            let server_name_len = data[i + 8] as usize;
            if ext_len >= server_name_list_len
                && server_name_list_len >= server_name_len
                && ext_len - server_name_list_len == 2
                && server_name_list_len - server_name_len == 3
            {
                let start = i + 9;
                let end = start + server_name_len;
                if end <= data.len() {
                    return Some((start, end));
                }
            }
        }
        i += 1;
    }
    None
}

fn fragment_random(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() * 2);
    let mut data = data.to_vec();
    if let Some(pos) = data.iter().position(|b| *b == 0) {
        out.extend_from_slice(&[0x16, 0x03, 0x04]);
        out.extend_from_slice(&int_to_2_bytes(pos + 1));
        out.extend_from_slice(&data[..pos + 1]);
        data = data[pos + 1..].to_vec();
    }
    let mut rng = rand::thread_rng();
    while !data.is_empty() {
        let chunk = rng.gen_range(1..=data.len());
        out.extend_from_slice(&[0x16, 0x03, 0x04]);
        out.extend_from_slice(&int_to_2_bytes(chunk));
        out.extend_from_slice(&data[..chunk]);
        data = data[chunk..].to_vec();
    }
    out
}

fn fragment_sni(data: &[u8]) -> Vec<u8> {
    let mut parts: Vec<Vec<u8>> = Vec::new();
    if let Some((start, end)) = extract_sni_position(data) {
        let part_start = &data[..start];
        let sni_data = &data[start..end];
        let part_end = &data[end..];
        let middle = (sni_data.len() + 1) / 2;

        parts.push(
            [
                vec![0x16, 0x03, 0x04],
                int_to_2_bytes(part_start.len()).to_vec(),
                part_start.to_vec(),
            ]
            .concat(),
        );
        parts.push(
            [
                vec![0x16, 0x03, 0x04],
                int_to_2_bytes(sni_data[..middle].len()).to_vec(),
                sni_data[..middle].to_vec(),
            ]
            .concat(),
        );
        parts.push(
            [
                vec![0x16, 0x03, 0x04],
                int_to_2_bytes(sni_data[middle..].len()).to_vec(),
                sni_data[middle..].to_vec(),
            ]
            .concat(),
        );
        parts.push(
            [
                vec![0x16, 0x03, 0x04],
                int_to_2_bytes(part_end.len()).to_vec(),
                part_end.to_vec(),
            ]
            .concat(),
        );
    }

    parts.concat()
}

fn is_domain_blocked(blocked: &[String], domain_matching: DomainMatching, domain: &str) -> bool {
    let domain = domain.replace("www.", "").to_lowercase();
    if domain_matching == DomainMatching::Loose {
        for b in blocked {
            if domain.contains(b) {
                return true;
            }
        }
    }

    if blocked.contains(&domain) {
        return true;
    }

    let parts: Vec<&str> = domain.split('.').collect();
    for i in 1..parts.len() {
        let parent = parts[i..].join(".");
        if blocked.contains(&parent) {
            return true;
        }
    }

    false
}

async fn connect_with_out_host(host: &str, port: u16, out_host: &Option<String>) -> io::Result<TcpStream> {
    let mut addrs = lookup_host((host, port)).await?;
    let out_addr = if let Some(out) = out_host {
        let mut out_iter = lookup_host((out.as_str(), 0)).await?;
        out_iter.next()
    } else {
        None
    };

    while let Some(addr) = addrs.next() {
        if let Some(local) = out_addr {
            if addr.is_ipv4() != local.is_ipv4() {
                continue;
            }
            let socket = if addr.is_ipv4() {
                TcpSocket::new_v4()?
            } else {
                TcpSocket::new_v6()?
            };
            let bind_addr = SocketAddr::new(local.ip(), 0);
            socket.bind(bind_addr)?;
            if let Ok(stream) = socket.connect(addr).await {
                return Ok(stream);
            }
        } else if let Ok(stream) = TcpStream::connect(addr).await {
            return Ok(stream);
        }
    }

    Err(io::Error::new(
        io::ErrorKind::Other,
        "Unable to connect to remote host",
    ))
}

fn format_size(size: u64) -> String {
    let units = ["B", "KB", "MB", "GB"];
    let mut unit = 0;
    let mut size_float = size as f64;
    while size_float >= 1024.0 && unit < units.len() - 1 {
        size_float /= 1024.0;
        unit += 1;
    }
    format!("{:.1} {}", size_float, units[unit])
}

fn format_speed(speed_bps: f64) -> String {
    if speed_bps <= 0.0 {
        return "0 b/s".to_string();
    }
    let units = ["b/s", "Kb/s", "Mb/s", "Gb/s"];
    let mut unit = 0;
    let mut speed = speed_bps;
    while speed >= 1000.0 && unit < units.len() - 1 {
        speed /= 1000.0;
        unit += 1;
    }
    format!("{:.0} {}", speed, units[unit])
}

fn int_to_2_bytes(n: usize) -> [u8; 2] {
    [(n >> 8) as u8, (n & 0xff) as u8]
}

fn open_append(path: &str) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

fn append_line(path: &str, line: &str) -> io::Result<()> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{}", line)?;
    Ok(())
}

fn load_blacklist(path: &str) -> io::Result<Vec<String>> {
    if !Path::new(path).exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("File {} not found", path),
        ));
    }
    let data = fs::read_to_string(path)?;
    let mut blocked = Vec::new();
    for line in data.lines() {
        let line = line.trim();
        if line.len() < 2 || line.starts_with('#') {
            continue;
        }
        blocked.push(line.to_lowercase().replace("www.", ""));
    }
    Ok(blocked)
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if current.len() + word.len() + 1 > width {
            lines.push(current.trim_end().to_string());
            current.clear();
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

fn parse_args() -> Result<Args, String> {
    let mut host = "127.0.0.1".to_string();
    let mut port: u16 = 8881;
    let mut out_host: Option<String> = None;
    let mut blacklist = "blacklist.txt".to_string();
    let mut fragment_method = FragmentMethod::Random;
    let mut domain_matching = DomainMatching::Strict;
    let mut log_access: Option<String> = None;
    let mut log_error: Option<String> = None;
    let mut no_blacklist = false;
    let mut auto_blacklist = false;
    let mut quiet = false;
    let mut install = false;
    let mut uninstall = false;
    let mut blacklist_set = false;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        let (key, inline_value) = if let Some((k, v)) = arg.split_once('=') {
            (k, Some(v))
        } else {
            (arg.as_str(), None)
        };

        match key {
            "--host" => {
                if let Some(v) = take_value(&args, &mut i, inline_value) {
                    host = v;
                }
            }
            "--port" => {
                if let Some(v) = take_value(&args, &mut i, inline_value) {
                    if let Ok(p) = v.parse::<u16>() {
                        port = p;
                    }
                }
            }
            "--out-host" | "--out_host" => {
                if let Some(v) = take_value(&args, &mut i, inline_value) {
                    out_host = Some(v);
                }
            }
            "--blacklist" => {
                if let Some(v) = take_value(&args, &mut i, inline_value) {
                    blacklist = v;
                    blacklist_set = true;
                }
            }
            "--no-blacklist" | "--no_blacklist" => {
                no_blacklist = true;
            }
            "--autoblacklist" => {
                auto_blacklist = true;
            }
            "--fragment-method" | "--fragment_method" => {
                if let Some(v) = take_value(&args, &mut i, inline_value) {
                    fragment_method = match v.as_str() {
                        "sni" => FragmentMethod::Sni,
                        _ => FragmentMethod::Random,
                    };
                }
            }
            "--domain-matching" | "--domain_matching" => {
                if let Some(v) = take_value(&args, &mut i, inline_value) {
                    domain_matching = match v.as_str() {
                        "loose" => DomainMatching::Loose,
                        _ => DomainMatching::Strict,
                    };
                }
            }
            "--log-access" | "--log_access" => {
                if let Some(v) = take_value(&args, &mut i, inline_value) {
                    log_access = Some(v);
                }
            }
            "--log-error" | "--log_error" => {
                if let Some(v) = take_value(&args, &mut i, inline_value) {
                    log_error = Some(v);
                }
            }
            "-q" | "--quiet" => {
                quiet = true;
            }
            "--install" => {
                install = true;
            }
            "--uninstall" => {
                uninstall = true;
            }
            _ => {}
        }

        i += 1;
    }

    let conflicts = [no_blacklist, auto_blacklist, blacklist_set]
        .iter()
        .filter(|v| **v)
        .count();
    if conflicts > 1 {
        return Err("error: blacklist flags are mutually exclusive".to_string());
    }
    if install && uninstall {
        return Err("error: argument --install: not allowed with --uninstall".to_string());
    }

    Ok(Args {
        config: Config {
            host,
            port,
            out_host,
            blacklist_file: blacklist,
            fragment_method,
            domain_matching,
            log_access_file: log_access,
            log_error_file: log_error,
            no_blacklist,
            auto_blacklist,
            quiet,
        },
        install,
        uninstall,
    })
}

fn take_value(args: &[String], i: &mut usize, inline: Option<&str>) -> Option<String> {
    if let Some(v) = inline {
        return Some(v.to_string());
    }
    if let Some(next) = args.get(*i + 1) {
        if !next.starts_with('-') {
            *i += 1;
            return Some(next.clone());
        }
    }
    None
}

fn manage_autostart(install: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        let app_name = "NoDPIProxy";
        let exe_path = std::env::current_exe()
            .map_err(|e| format!("[ERROR]: Autostart operation failed: {}", e))?;
        let exe_dir = exe_path
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "".to_string());
        let command = format!(
            "\"{}\" --blacklist \"{}/blacklist.txt\"",
            exe_path.to_string_lossy(),
            exe_dir
        );

        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let path = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
        let key = hkcu
            .open_subkey_with_flags(path, KEY_WRITE)
            .or_else(|_| hkcu.create_subkey(path).map(|(k, _)| k))
            .map_err(|e| format!("[ERROR]: Autostart operation failed: {}", e))?;

        if install {
            key.set_value(app_name, &command)
                .map_err(|e| format!("[ERROR]: Autostart operation failed: {}", e))?;
            println!("\x1b[92m[INFO]:\x1b[97m Added to autostart: {}", exe_path.to_string_lossy());
        } else {
            match key.delete_value(app_name) {
                Ok(()) => println!("\x1b[92m[INFO]:\x1b[97m Removed from autostart"),
                Err(_) => println!("\x1b[91m[ERROR]: Not found in autostart\x1b[0m"),
            }
        }

        return Ok(());
    }

    #[cfg(target_os = "linux")]
    {
        let app_name = "NoDPIProxy";
        let exec_path = std::env::current_exe()
            .map_err(|e| format!("[ERROR]: Autostart operation failed: {}", e))?;
        let service_name = format!("{}.service", app_name.to_lowercase());

        let home_dir = std::env::var("HOME")
            .map_err(|_| "[ERROR]: Cannot resolve home directory".to_string())?;
        let user_service_dir = Path::new(&home_dir).join(".config/systemd/user");
        let service_file = user_service_dir.join(&service_name);
        let blacklist_path = format!("{}/blacklist.txt", exec_path.parent().unwrap().display());

        if install {
            fs::create_dir_all(&user_service_dir)
                .map_err(|e| format!("[ERROR]: Autostart operation failed: {}", e))?;
            let service_content = format!(
                "[Unit]\nDescription=NoDPIProxy Service\nAfter=network.target graphical-session.target\nWants=network.target\n\n[Service]\nType=simple\nExecStart={} --blacklist \"{}\" --quiet\nRestart=on-failure\nRestartSec=5\nEnvironment=DISPLAY=:0\nEnvironment=XAUTHORITY=%h/.Xauthority\n\n[Install]\nWantedBy=default.target\n",
                exec_path.display(),
                blacklist_path
            );
            fs::write(&service_file, service_content)
                .map_err(|e| format!("[ERROR]: Autostart operation failed: {}", e))?;
            Command::new("systemctl").args(["--user", "daemon-reload"]).status().ok();
            Command::new("systemctl").args(["--user", "enable", &service_name]).status().ok();
            Command::new("systemctl").args(["--user", "start", &service_name]).status().ok();
            println!("\x1b[92m[INFO]:\x1b[97m Service installed and started: {}", service_name);
            println!("\x1b[93m[NOTE]:\x1b[97m Service will auto-start on login");
        } else {
            Command::new("systemctl").args(["--user", "stop", &service_name]).status().ok();
            Command::new("systemctl").args(["--user", "disable", &service_name]).status().ok();
            if service_file.exists() {
                let _ = fs::remove_file(service_file);
            }
            Command::new("systemctl").args(["--user", "daemon-reload"]).status().ok();
            println!("\x1b[92m[INFO]:\x1b[97m Service removed from autostart");
        }

        return Ok(());
    }

    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = install;
        Err("Autostart only available on Windows/Linux".to_string())
    }
}

#[tokio::main]
async fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(msg) => {
            eprintln!("{}", msg);
            return;
        }
    };

    if args.install || args.uninstall {
        if let Err(err) = manage_autostart(args.install) {
            eprintln!("\x1b[91m[ERROR]:\x1b[97m {}", err);
            std::process::exit(1);
        }
        return;
    }

    let logger = match Logger::new(&args.config.log_access_file, &args.config.log_error_file, args.config.quiet) {
        Ok(l) => Arc::new(l),
        Err(err) => {
            eprintln!("\x1b[91m[ERROR]:\x1b[97m {}\x1b[0m", err);
            return;
        }
    };

    let statistics = Arc::new(Statistics::new());
    let stats_clone = Arc::clone(&statistics);
    logger
        .set_error_counter_callback(Arc::new(move || {
            let stats = Arc::clone(&stats_clone);
            tokio::spawn(async move {
                stats.increment_error_connections().await;
            });
        }))
        .await;

    let blacklist_manager = match create_blacklist_manager(&args.config) {
        Ok(m) => Arc::new(m),
        Err(err) => {
            logger.error(&format!("\x1b[91m[ERROR]: {}\x1b[0m", err));
            return;
        }
    };

    let proxy = ProxyServer::new(args.config, blacklist_manager, statistics, logger);

    let shutdown = proxy.shutdown.clone();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        shutdown.notify_waiters();
        #[cfg(windows)]
        {
            let _ = Command::new("cmd").args(["/C", "mode con: lines=3000"]).status();
        }
    });

    if let Err(err) = proxy.run().await {
        eprintln!("Proxy stopped: {}", err);
    }
}

fn create_blacklist_manager(config: &Config) -> io::Result<BlacklistManager> {
    if config.no_blacklist {
        return Ok(BlacklistManager::None);
    }
    if config.auto_blacklist {
        return Ok(BlacklistManager::Auto {
            blocked: Mutex::new(Vec::new()),
            whitelist: Mutex::new(Vec::new()),
            blacklist_file: config.blacklist_file.clone(),
        });
    }
    let blocked = load_blacklist(&config.blacklist_file)?;
    Ok(BlacklistManager::File {
        blocked,
        domain_matching: config.domain_matching,
    })
}

trait LeftJustify {
    fn ljust(&self, width: usize) -> String;
}

impl LeftJustify for String {
    fn ljust(&self, width: usize) -> String {
        let mut s = self.clone();
        if s.len() < width {
            s.push_str(&" ".repeat(width - s.len()));
        }
        s
    }
}

impl LeftJustify for &str {
    fn ljust(&self, width: usize) -> String {
        let mut s = self.to_string();
        if s.len() < width {
            s.push_str(&" ".repeat(width - s.len()));
        }
        s
    }
}
