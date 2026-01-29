use chrono::Local;
use rand::Rng;
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write as IoWrite};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::{Mutex, Notify};
use tokio::time::{self, Duration, Instant};

#[cfg(windows)]
use winreg::enums::*;
#[cfg(windows)]
use winreg::RegKey;

const VERSION: &str = "1.8.2";

struct Args {
    host: String,
    port: u16,
    blacklist: String,
    no_blacklist: bool,
    log_access: Option<String>,
    log_error: Option<String>,
    quiet: bool,
    verbose: bool,
    install: bool,
    uninstall: bool,
}

struct Logger {
    error: Option<Mutex<File>>,
    access: Option<Mutex<File>>,
}

impl Logger {
    fn new(log_access: &Option<String>, log_error: &Option<String>) -> io::Result<Self> {
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

        Ok(Self { error, access })
    }

    async fn log_error(&self, message: &str) {
        if let Some(file) = &self.error {
            let mut f = file.lock().await;
            let ts = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
            let _ = writeln!(f, "[{}][ERROR]: {}", ts, message);
        }
    }

    async fn log_access(&self, line: &str) {
        if let Some(file) = &self.access {
            let mut f = file.lock().await;
            let _ = writeln!(f, "{}", line);
        }
    }
}

struct ConnectionInfo {
    src_ip: String,
    dst_domain: String,
    method: String,
    start_time: String,
    traffic_in: u64,
    traffic_out: u64,
}

struct ProxyServer {
    host: String,
    port: u16,
    blacklist: Vec<Vec<u8>>,
    no_blacklist: bool,
    quiet: bool,
    verbose: bool,
    log_error: Option<String>,
    logger: Logger,
    active_connections: Mutex<HashMap<String, ConnectionInfo>>,
    total_connections: AtomicU64,
    allowed_connections: AtomicU64,
    blocked_connections: AtomicU64,
    traffic_in: AtomicU64,
    traffic_out: AtomicU64,
}

impl ProxyServer {
    fn new(args: &Args) -> io::Result<Self> {
        let blacklist = if args.no_blacklist {
            Vec::new()
        } else {
            load_blacklist(&args.blacklist)?
        };

        Ok(Self {
            host: args.host.clone(),
            port: args.port,
            blacklist,
            no_blacklist: args.no_blacklist,
            quiet: args.quiet,
            verbose: args.verbose,
            log_error: args.log_error.clone(),
            logger: Logger::new(&args.log_access, &args.log_error)?,
            active_connections: Mutex::new(HashMap::new()),
            total_connections: AtomicU64::new(0),
            allowed_connections: AtomicU64::new(0),
            blocked_connections: AtomicU64::new(0),
            traffic_in: AtomicU64::new(0),
            traffic_out: AtomicU64::new(0),
        })
    }

    fn print(&self, msg: &str) {
        if !self.quiet {
            println!("{}", msg);
        }
    }

    fn print_banner(&self) {
        if self.quiet {
            return;
        }
        println!(
            "\n\
\033[92m`7MN.   `7MF'          `7MM\"\"\"Yb.   `7MM\"\"\"Mq. `7MMF'\n\
  MMN.    M              MM    `Yb.   MM   `MM.  MM\n\
  M YMb   M   ,pW\"Wq.    MM     `Mb   MM   ,M9   MM\n\
  M  `MN. M  6W'   `Wb   MM      MM   MMmmdM9    MM\n\
  M   `MM.M  8M     M8   MM     ,MP   MM         MM\n\
  M     YMM  YA.   ,A9   MM    ,dP'   MM         MM\n\
.JML.    YM   `Ybmd9'  .JMMmmmdP'   .JMML.     .JMML.\033[0m\n"
        );
        println!("\033[92mVersion: {}\033[0m", VERSION);
        println!(
            "\033[97mEnjoy watching! / Наслаждайтесь просмотром!\033[0m"
        );
        println!(
            "Proxy is running on {}:{}",
            self.host, self.port
        );
        println!("\n\033[92m[INFO]:\033[97m Proxy started at {}\033[0m", Local::now().format("%Y-%m-%d %H:%M:%S"));
        if !self.no_blacklist {
            println!(
                "\033[92m[INFO]:\033[97m Blacklist contains {} domains\033[0m",
                self.blacklist.len()
            );
        }
        println!("\033[92m[INFO]:\033[97m To stop the proxy, press Ctrl+C twice\033[0m");
        if let Some(path) = &self.log_error {
            println!(
                "\033[92m[INFO]:\033[97m Logging is in progress. You can see the list of errors in the file {}\033[0m",
                path
            );
        }
    }

    async fn run(self: Arc<Self>, shutdown: Arc<Notify>) -> io::Result<()> {
        let addr = format!("{}:{}", self.host, self.port);
        let listener = TcpListener::bind(&addr).await?;

        self.print_banner();

        if !self.quiet {
            let stats_server = Arc::clone(&self);
            let stats_shutdown = Arc::clone(&shutdown);
            tokio::spawn(async move {
                stats_server.display_stats(stats_shutdown).await;
            });
        }

        loop {
            tokio::select! {
                _ = shutdown.notified() => {
                    self.print("\n\n\033[92m[INFO]:\033[97m Shutting down proxy...\033[0m");
                    break;
                }
                res = listener.accept() => {
                    match res {
                        Ok((conn, _)) => {
                            let server = Arc::clone(&self);
                            let shutdown_clone = Arc::clone(&shutdown);
                            tokio::spawn(async move {
                                let _ = server.handle_connection(conn, shutdown_clone).await;
                            });
                        }
                        Err(err) => {
                            if self.verbose {
                                eprintln!("Accept error: {}", err);
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    async fn handle_connection(
        &self,
        mut client: TcpStream,
        shutdown: Arc<Notify>,
    ) -> io::Result<()> {
        let peer = client
            .peer_addr()
            .map(|addr| addr.to_string())
            .unwrap_or_else(|_| "unknown".to_string());

        let mut buf = vec![0u8; 1500];
        let n = client.read(&mut buf).await?;
        if n == 0 {
            return Ok(());
        }

        let http_data = buf[..n].to_vec();
        let req = String::from_utf8_lossy(&http_data);
        let lines: Vec<&str> = req.split("\r\n").collect();
        if lines.is_empty() {
            return Ok(());
        }

        let parts: Vec<&str> = lines[0].split(' ').collect();
        if parts.len() < 2 {
            return Ok(());
        }
        let method = parts[0].to_string();
        let target = parts[1].to_string();

        let (host, port) = if method == "CONNECT" {
            let hp: Vec<&str> = target.split(':').collect();
            let host = hp.get(0).unwrap_or(&"").to_string();
            let port = hp.get(1).unwrap_or(&"443").to_string();
            (host, port)
        } else {
            let mut host = String::new();
            let mut port = String::from("80");
            for line in &lines[1..] {
                if let Some(rest) = line.strip_prefix("Host:") {
                    let hp: Vec<&str> = rest.trim().split(':').collect();
                    host = hp.get(0).unwrap_or(&"").to_string();
                    if hp.len() == 2 {
                        port = hp[1].to_string();
                    }
                    break;
                }
            }
            (host, port)
        };

        if host.is_empty() {
            self.logger
                .log_error("Missing Host header")
                .await;
            let _ = client
                .write_all(b"HTTP/1.1 500 Internal Server Error\r\n\r\n")
                .await;
            return Ok(());
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

        if method == "CONNECT" {
            let _ = client
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .await;
            let mut dst = match TcpStream::connect(format!("{}:{}", host, port)).await {
                Ok(s) => s,
                Err(err) => {
                    self.logger
                        .log_error(&format!("{}: {}", host, err))
                        .await;
                    return Ok(());
                }
            };
            let (mut client_reader, mut client_writer) = client.into_split();
            let (mut dst_reader, mut dst_writer) = dst.into_split();

            if let Err(err) = self
                .fragment_data(&mut client_reader, &mut dst_writer)
                .await
            {
                self.logger
                    .log_error(&format!("{}: {}", host, err))
                    .await;
            }

            self.total_connections.fetch_add(1, Ordering::Relaxed);

            let out_task = self.pipe(
                client_reader,
                dst_writer,
                PipeDirection::Out,
                peer.clone(),
                Arc::clone(&shutdown),
            );
            let in_task = self.pipe(
                dst_reader,
                client_writer,
                PipeDirection::In,
                peer.clone(),
                Arc::clone(&shutdown),
            );
            let _ = tokio::join!(out_task, in_task);
        } else {
            let mut dst = match TcpStream::connect(format!("{}:{}", host, port)).await {
                Ok(s) => s,
                Err(err) => {
                    self.logger
                        .log_error(&format!("{}: {}", host, err))
                        .await;
                    let _ = client
                        .write_all(b"HTTP/1.1 500 Internal Server Error\r\n\r\n")
                        .await;
                    return Ok(());
                }
            };
            let (mut client_reader, mut client_writer) = client.into_split();
            let (mut dst_reader, mut dst_writer) = dst.into_split();

            if let Err(err) = dst_writer.write_all(&http_data).await {
                self.logger
                    .log_error(&format!("{}: {}", host, err))
                    .await;
                return Ok(());
            }

            self.allowed_connections.fetch_add(1, Ordering::Relaxed);
            self.total_connections.fetch_add(1, Ordering::Relaxed);

            let out_task = self.pipe(
                client_reader,
                dst_writer,
                PipeDirection::Out,
                peer.clone(),
                Arc::clone(&shutdown),
            );
            let in_task = self.pipe(
                dst_reader,
                client_writer,
                PipeDirection::In,
                peer.clone(),
                Arc::clone(&shutdown),
            );
            let _ = tokio::join!(out_task, in_task);
        }

        let conn_info = {
            let mut map = self.active_connections.lock().await;
            map.remove(&peer)
        };

        if let Some(info) = conn_info {
            let line = format!(
                "{} {} {} {}",
                info.start_time, info.src_ip, info.method, info.dst_domain
            );
            self.logger.log_access(&line).await;
        }

        Ok(())
    }

    async fn fragment_data(
        &self,
        reader: &mut OwnedReadHalf,
        writer: &mut OwnedWriteHalf,
    ) -> io::Result<()> {
        let mut head = [0u8; 5];
        reader.read_exact(&mut head).await?;

        let mut data = vec![0u8; 2048];
        let n = reader.read(&mut data).await?;
        data.truncate(n);

        let mut blocked = true;
        if !self.no_blacklist {
            let mut contains_blocked = false;
            for site in &self.blacklist {
                if data.windows(site.len()).any(|w| w == site.as_slice()) {
                    contains_blocked = true;
                    break;
                }
            }
            if !contains_blocked {
                blocked = false;
            }
        }

        if !blocked {
            self.allowed_connections.fetch_add(1, Ordering::Relaxed);
            writer.write_all(&head).await?;
            writer.write_all(&data).await?;
            return Ok(());
        }

        self.blocked_connections.fetch_add(1, Ordering::Relaxed);

        let mut out = Vec::with_capacity(data.len() * 2);
        if let Some(pos) = data.iter().position(|b| *b == 0) {
            out.extend_from_slice(&[0x16, 0x03, 0x04]);
            out.extend_from_slice(&int_to_2_bytes(pos + 1));
            out.extend_from_slice(&data[..pos + 1]);
            data = data[pos + 1..].to_vec();
        }

        {
            let mut rng = rand::thread_rng();
            while !data.is_empty() {
                let chunk = rng.gen_range(1..=data.len());
                out.extend_from_slice(&[0x16, 0x03, 0x04]);
                out.extend_from_slice(&int_to_2_bytes(chunk));
                out.extend_from_slice(&data[..chunk]);
                data = data[chunk..].to_vec();
            }
        }

        writer.write_all(&out).await?;
        Ok(())
    }

    async fn pipe(
        &self,
        mut reader: OwnedReadHalf,
        mut writer: OwnedWriteHalf,
        direction: PipeDirection,
        conn_key: String,
        shutdown: Arc<Notify>,
    ) {
        let mut buf = vec![0u8; 1500];
        loop {
            tokio::select! {
                _ = shutdown.notified() => {
                    break;
                }
                res = reader.read(&mut buf) => {
                    let n = match res {
                        Ok(0) => break,
                        Ok(n) => n,
                        Err(err) => {
                            if self.verbose {
                                eprintln!("Pipe error: {}", err);
                            }
                            break;
                        }
                    };
                    if direction == PipeDirection::Out {
                        self.traffic_out.fetch_add(n as u64, Ordering::Relaxed);
                    } else {
                        self.traffic_in.fetch_add(n as u64, Ordering::Relaxed);
                    }
                    {
                        let mut map = self.active_connections.lock().await;
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
            }
        }
        let _ = writer.shutdown().await;
    }

    async fn display_stats(self: Arc<Self>, shutdown: Arc<Notify>) {
        let mut last_time: Option<Instant> = None;
        let mut last_in = 0u64;
        let mut last_out = 0u64;
        let mut ticker = time::interval(Duration::from_secs(1));

        loop {
            tokio::select! {
                _ = shutdown.notified() => break,
                _ = ticker.tick() => {
                    let now = Instant::now();
                    let curr_in = self.traffic_in.load(Ordering::Relaxed);
                    let curr_out = self.traffic_out.load(Ordering::Relaxed);

                    let mut speed_in = 0.0;
                    let mut speed_out = 0.0;
                    if let Some(prev) = last_time {
                        let diff = now.duration_since(prev).as_secs_f64();
                        if diff > 0.0 {
                            speed_in = (curr_in - last_in) as f64 * 8.0 / diff;
                            speed_out = (curr_out - last_out) as f64 * 8.0 / diff;
                        }
                    }

                    last_time = Some(now);
                    last_in = curr_in;
                    last_out = curr_out;

                    let stats = format!(
                        "\r\033[92m[STATS]:\033[0m \
\033[97mConns: \033[93m{}\033[0m | \
\033[97mMiss: \033[92m{}\033[0m | \
\033[97mUnblock: \033[91m{}\033[0m | \
\033[97mDL: \033[96m{}\033[0m | \
\033[97mUL: \033[96m{}\033[0m | \
\033[97mSpeed DL: \033[96m{}\033[0m | \
\033[97mSpeed UL: \033[96m{}\033[0m",
                        self.total_connections.load(Ordering::Relaxed),
                        self.allowed_connections.load(Ordering::Relaxed),
                        self.blocked_connections.load(Ordering::Relaxed),
                        format_size(curr_in as f64),
                        format_size(curr_out as f64),
                        format_speed(speed_in),
                        format_speed(speed_out),
                    );
                    print!("\u{001b}[2K{}", stats);
                    let _ = io::stdout().flush();
                }
            }
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum PipeDirection {
    In,
    Out,
}

fn format_size(mut size: f64) -> String {
    let units = ["B", "KB", "MB", "GB"];
    let mut unit = 0;
    while size >= 1024.0 && unit < units.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    format!("{:.1} {}", size, units[unit])
}

fn format_speed(mut speed: f64) -> String {
    let units = ["bps", "Kbps", "Mbps", "Gbps"];
    let mut unit = 0;
    while speed >= 1000.0 && unit < units.len() - 1 {
        speed /= 1000.0;
        unit += 1;
    }
    format!("{:.1} {}", speed, units[unit])
}

fn int_to_2_bytes(n: usize) -> [u8; 2] {
    [(n >> 8) as u8, (n & 0xff) as u8]
}

fn open_append(path: &str) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

fn load_blacklist(path: &str) -> io::Result<Vec<Vec<u8>>> {
    if !std::path::Path::new(path).exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("File {} not found", path),
        ));
    }
    let data = fs::read(path)?;
    let mut blocked = Vec::new();
    for line in data.split(|b| *b == b'\n') {
        if !line.is_empty() {
            blocked.push(line.to_vec());
        }
    }
    Ok(blocked)
}

fn parse_args() -> Result<Args, String> {
    let mut host = "127.0.0.1".to_string();
    let mut port: u16 = 8881;
    let mut blacklist = "blacklist.txt".to_string();
    let mut no_blacklist = false;
    let mut log_access = None;
    let mut log_error = None;
    let mut quiet = false;
    let mut verbose = false;
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
            "--blacklist" => {
                if let Some(v) = take_value(&args, &mut i, inline_value) {
                    blacklist = v;
                    blacklist_set = true;
                }
            }
            "--no_blacklist" => {
                if let Some(v) = take_bool_value(&args, &mut i, inline_value) {
                    no_blacklist = v;
                } else {
                    no_blacklist = true;
                }
            }
            "--log_access" => {
                if let Some(v) = take_value(&args, &mut i, inline_value) {
                    log_access = Some(v);
                }
            }
            "--log_error" => {
                if let Some(v) = take_value(&args, &mut i, inline_value) {
                    log_error = Some(v);
                }
            }
            "-q" | "--quiet" => {
                quiet = true;
            }
            "-v" | "--verbose" => {
                verbose = true;
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

    if blacklist_set && no_blacklist {
        return Err("error: argument --no_blacklist: not allowed with --blacklist".to_string());
    }
    if install && uninstall {
        return Err("error: argument --install: not allowed with --uninstall".to_string());
    }

    Ok(Args {
        host,
        port,
        blacklist,
        no_blacklist,
        log_access,
        log_error,
        quiet,
        verbose,
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

fn take_bool_value(args: &[String], i: &mut usize, inline: Option<&str>) -> Option<bool> {
    if let Some(v) = inline {
        return v.parse::<bool>().ok();
    }
    if let Some(next) = args.get(*i + 1) {
        if matches!(next.as_str(), "true" | "false") {
            *i += 1;
            return next.parse::<bool>().ok();
        }
    }
    None
}

fn manage_autostart(install: bool) -> Result<(), String> {
    #[cfg(not(windows))]
    {
        let _ = install;
        return Err("Autostart only available on Windows".to_string());
    }

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
            println!("\033[92m[INFO]:\033[97m Added to autostart: {}", exe_path.to_string_lossy());
        } else {
            match key.delete_value(app_name) {
                Ok(()) => println!("\033[92m[INFO]:\033[97m Removed from autostart"),
                Err(_) => println!("\033[91m[ERROR]: Not found in autostart\033[0m"),
            }
        }

        Ok(())
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
            eprintln!("\033[91m[ERROR]:\033[97m {}", err);
            std::process::exit(1);
        }
        return;
    }

    let server = match ProxyServer::new(&args) {
        Ok(s) => Arc::new(s),
        Err(err) => {
            eprintln!("\033[91m[ERROR]:\033[97m {}\033[0m", err);
            return;
        }
    };

    let shutdown = Arc::new(Notify::new());
    let shutdown_signal = Arc::clone(&shutdown);
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        shutdown_signal.notify_waiters();
    });

    if let Err(err) = server.run(shutdown).await {
        eprintln!("Proxy stopped: {}", err);
    }
    println!("\nProxy exited gracefully");
}
