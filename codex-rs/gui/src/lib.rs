//! Browser GUI for Codex.
//!
//! `codex gui` (installed as `cortecs-codex gui`) serves a single-page client
//! on a loopback port and bridges each browser websocket to its own
//! `codex app-server --stdio` child process. The
//! app-server owns everything that matters for safety and persistence (config,
//! auth, sandboxing, approvals, session rollouts), so GUI sessions behave like
//! TUI sessions and show up in `codex resume`.
//!
//! The app-server's own websocket listener rejects browser requests (it refuses
//! any request carrying an `Origin` header), which is why the GUI relays over
//! stdio instead. Access to the bridge is guarded by a random per-launch token
//! plus an `Origin` check, so other local web pages cannot drive it.
//!
//! Folder trust is resolved here, not in the page, using the same config code
//! as the rest of Codex: the page asks before working in a folder that has no
//! saved trust decision, like the TUI's trust prompt.

use std::net::Ipv4Addr;
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use axum::Json;
use axum::Router;
use axum::extract::RawQuery;
use axum::extract::State;
use axum::extract::WebSocketUpgrade;
use axum::extract::ws::Message;
use axum::extract::ws::WebSocket;
use axum::http::HeaderMap;
use axum::http::HeaderValue;
use axum::http::StatusCode;
use axum::http::header;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::get;
use codex_core::config::ConfigBuilder;
use codex_protocol::config_types::TrustLevel;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_cli::CliConfigOverrides;
use futures::SinkExt;
use futures::StreamExt;
use serde::Serialize;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

const INDEX_HTML: &str = include_str!("../assets/index.html");
/// Copy of `tui/assets/prompt_for_init_command.md` (kept in sync by a test), sent by `/init`.
const INIT_PROMPT: &str = include_str!("../assets/prompt_for_init_command.md");

/// How long a GUI connection's app-server gets to shut down after its stdin closes.
const CHILD_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(15);

/// [experimental] Open a browser-based GUI.
#[derive(Debug, clap::Parser)]
pub struct GuiCli {
    /// Folder to open first. Defaults to the current directory.
    #[arg(value_name = "DIR")]
    pub dir: Option<PathBuf>,

    /// Loopback port to listen on. Defaults to a free port.
    #[arg(long, default_value_t = 0)]
    pub port: u16,

    /// Print the URL instead of opening a browser.
    #[arg(long = "no-browser", default_value_t = false)]
    pub no_browser: bool,
}

struct Bridge {
    token: String,
    allowed_origins: Vec<String>,
    codex_exe: PathBuf,
    raw_overrides: Vec<String>,
    parsed_overrides: Vec<(String, toml::Value)>,
    codex_home: PathBuf,
    initial_cwd: PathBuf,
    shutdown: CancellationToken,
    sessions: TaskTracker,
}

/// Runs the GUI until Ctrl-C.
pub async fn run_gui(cli: GuiCli, config_overrides: CliConfigOverrides) -> anyhow::Result<()> {
    let codex_exe = std::env::current_exe().context("cannot locate the codex executable")?;
    let initial_cwd = match cli.dir {
        Some(dir) => std::fs::canonicalize(&dir)
            .with_context(|| format!("cannot open folder {}", dir.display()))?,
        None => std::env::current_dir()?,
    };
    anyhow::ensure!(
        initial_cwd.is_dir(),
        "{} is not a folder",
        initial_cwd.display()
    );
    let parsed_overrides = config_overrides
        .parse_overrides()
        .map_err(|err| anyhow::anyhow!("invalid -c override: {err}"))?;
    let codex_home = codex_core::config::find_codex_home()?.to_path_buf();

    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, cli.port))
        .await
        .context("cannot listen on 127.0.0.1")?;
    let port = listener.local_addr()?.port();
    let token = random_token();
    let bridge = Arc::new(Bridge {
        token: token.clone(),
        allowed_origins: vec![
            format!("http://127.0.0.1:{port}"),
            format!("http://localhost:{port}"),
        ],
        codex_exe,
        raw_overrides: config_overrides.raw_overrides,
        parsed_overrides,
        codex_home,
        initial_cwd,
        shutdown: CancellationToken::new(),
        sessions: TaskTracker::new(),
    });

    let router = Router::new()
        .route("/", get(index))
        .route("/ws", get(websocket))
        .route("/api/info", get(info))
        .route("/api/trust", get(read_trust).post(write_trust))
        .with_state(Arc::clone(&bridge));
    let server = tokio::spawn(async move { axum::serve(listener, router).await });

    let url = format!("http://127.0.0.1:{port}/?token={token}");
    #[allow(clippy::print_stdout)]
    {
        println!("Codex GUI: {url}");
        println!("Keep this terminal open; press Ctrl-C to quit.");
    }
    if !cli.no_browser
        && let Err(err) = webbrowser::open(&url)
    {
        #[allow(clippy::print_stderr)]
        {
            eprintln!("Could not open a browser ({err}); open the URL above manually.");
        }
    }

    tokio::select! {
        result = server => {
            result.context("GUI server task failed")?.context("GUI server failed")?;
        }
        _ = tokio::signal::ctrl_c() => {}
    }

    // Close every app-server's stdin so it can finish writing its sessions.
    bridge.shutdown.cancel();
    bridge.sessions.close();
    let _ = tokio::time::timeout(CHILD_SHUTDOWN_TIMEOUT, bridge.sessions.wait()).await;
    Ok(())
}

fn random_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 24];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn query_param(query: Option<&str>, name: &str) -> Option<String> {
    url::form_urlencoded::parse(query?.as_bytes())
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

impl Bridge {
    /// Requires the launch token, and a same-origin `Origin` header when one is
    /// present or `require_origin` is set (websocket upgrades and writes).
    fn authorize(
        &self,
        query: Option<&str>,
        headers: &HeaderMap,
        require_origin: bool,
    ) -> Result<(), Response> {
        let token_ok = query_param(query, "token")
            .is_some_and(|token| constant_time_eq(token.as_bytes(), self.token.as_bytes()));
        if !token_ok {
            return Err((StatusCode::FORBIDDEN, "missing or wrong token").into_response());
        }
        match headers.get(header::ORIGIN).map(HeaderValue::to_str) {
            Some(Ok(origin)) if self.allowed_origins.iter().any(|allowed| allowed == origin) => {
                Ok(())
            }
            None if !require_origin => Ok(()),
            _ => Err((StatusCode::FORBIDDEN, "cross-origin request refused").into_response()),
        }
    }
}

async fn index(
    State(bridge): State<Arc<Bridge>>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Response {
    if let Err(response) = bridge.authorize(query.as_deref(), &headers, false) {
        return response;
    }
    let ws_sources = bridge
        .allowed_origins
        .iter()
        .map(|origin| origin.replacen("http://", "ws://", 1))
        .collect::<Vec<_>>()
        .join(" ");
    let csp = format!(
        "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; \
         img-src 'self' data:; connect-src 'self' {ws_sources}; base-uri 'none'; \
         form-action 'none'; frame-ancestors 'none'"
    );
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8".to_string()),
            (header::CONTENT_SECURITY_POLICY, csp),
            // The URL carries the token; never leak it to linked sites.
            (header::REFERRER_POLICY, "no-referrer".to_string()),
            (header::CACHE_CONTROL, "no-store".to_string()),
        ],
        INDEX_HTML,
    )
        .into_response()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InfoResponse {
    cwd: String,
    home: Option<String>,
    version: &'static str,
    init_prompt: &'static str,
}

async fn info(
    State(bridge): State<Arc<Bridge>>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Response {
    if let Err(response) = bridge.authorize(query.as_deref(), &headers, false) {
        return response;
    }
    Json(InfoResponse {
        cwd: bridge.initial_cwd.to_string_lossy().into_owned(),
        home: std::env::var_os("HOME").map(|home| home.to_string_lossy().into_owned()),
        version: env!("CARGO_PKG_VERSION"),
        init_prompt: INIT_PROMPT,
    })
    .into_response()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TrustInfo {
    /// Canonical folder the session would use.
    cwd: String,
    /// Saved decision: `"trusted"`, `"untrusted"`, or null when none was made.
    trust_level: Option<String>,
    /// Folder a "trust" decision is saved for (the repository root inside Git).
    trust_target: String,
    /// `config` overrides for `thread/start` that open the folder restricted
    /// for one session without saving trust (the TUI's "Look without trusting").
    look_only_config: serde_json::Value,
}

async fn read_trust(
    State(bridge): State<Arc<Bridge>>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Response {
    if let Err(response) = bridge.authorize(query.as_deref(), &headers, false) {
        return response;
    }
    let Some(cwd) = query_param(query.as_deref(), "cwd") else {
        return (StatusCode::BAD_REQUEST, "missing cwd").into_response();
    };
    match trust_info(&bridge, Path::new(&cwd)).await {
        Ok(info) => Json(info).into_response(),
        Err(err) => (StatusCode::BAD_REQUEST, format!("{err:#}")).into_response(),
    }
}

async fn write_trust(
    State(bridge): State<Arc<Bridge>>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Response {
    if let Err(response) = bridge.authorize(query.as_deref(), &headers, true) {
        return response;
    }
    let Some(cwd) = query_param(query.as_deref(), "cwd") else {
        return (StatusCode::BAD_REQUEST, "missing cwd").into_response();
    };
    let result = async {
        let info = trust_info(&bridge, Path::new(&cwd)).await?;
        let codex_home = bridge.codex_home.clone();
        let target = PathBuf::from(&info.trust_target);
        tokio::task::spawn_blocking(move || {
            codex_core::config::set_project_trust_level(&codex_home, &target, TrustLevel::Trusted)
        })
        .await??;
        trust_info(&bridge, Path::new(&cwd)).await
    }
    .await;
    match result {
        Ok(info) => Json(info).into_response(),
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{err:#}")).into_response(),
    }
}

async fn trust_info(bridge: &Bridge, cwd: &Path) -> anyhow::Result<TrustInfo> {
    let cwd =
        std::fs::canonicalize(cwd).with_context(|| format!("cannot open {}", cwd.display()))?;
    anyhow::ensure!(cwd.is_dir(), "{} is not a folder", cwd.display());
    let config = ConfigBuilder::default()
        .codex_home(bridge.codex_home.clone())
        .cli_overrides(bridge.parsed_overrides.clone())
        .fallback_cwd(Some(cwd.clone()))
        .build()
        .await
        .context("cannot load the configuration for this folder")?;
    let trust_level = config
        .active_project
        .trust_level
        .map(|level| level.to_string());
    let absolute_cwd = AbsolutePathBuf::from_absolute_path(&cwd)?;
    let trust_target = codex_git_utils::resolve_root_git_project_for_trust(
        codex_exec_server::LOCAL_FS.as_ref(),
        &absolute_cwd,
    )
    .await
    .map(|root| root.to_path_buf())
    .unwrap_or_else(|| cwd.clone());
    let (key, value) = codex_config::loader::session_untrusted_projects_override(&cwd);
    let mut look_only_config = serde_json::Map::new();
    look_only_config.insert(key, serde_json::to_value(value)?);
    Ok(TrustInfo {
        cwd: cwd.to_string_lossy().into_owned(),
        trust_level,
        trust_target: trust_target.to_string_lossy().into_owned(),
        look_only_config: serde_json::Value::Object(look_only_config),
    })
}

async fn websocket(
    State(bridge): State<Arc<Bridge>>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    if let Err(response) = bridge.authorize(query.as_deref(), &headers, true) {
        return response;
    }
    upgrade.on_upgrade(move |socket| {
        let tracker = bridge.sessions.clone();
        tracker.track_future(async move {
            if let Err(err) = relay(&bridge, socket).await {
                #[allow(clippy::print_stderr)]
                {
                    eprintln!("GUI connection ended: {err:#}");
                }
            }
        })
    })
}

/// Relays one browser connection to a dedicated `codex app-server --stdio`.
async fn relay(bridge: &Bridge, socket: WebSocket) -> anyhow::Result<()> {
    let mut command = Command::new(&bridge.codex_exe);
    // Unload a session as soon as the page leaves it, so its writer lock is
    // released and the TUI can resume it right away (the default waits 60 s).
    // User overrides come later and win.
    command.arg("-c").arg("thread_unload_delay_secs=0");
    for raw in &bridge.raw_overrides {
        command.arg("-c").arg(raw);
    }
    command
        .arg("app-server")
        .arg("--stdio")
        .current_dir(&bridge.initial_cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    // Keep Ctrl-C in the terminal away from the child: it is shut down through
    // stdin EOF instead, so it can finish writing session files.
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn().context("cannot start codex app-server")?;
    let mut stdin = child.stdin.take().context("app-server stdin missing")?;
    let stdout = child.stdout.take().context("app-server stdout missing")?;
    let mut lines = BufReader::new(stdout).lines();
    let (mut ws_tx, mut ws_rx) = socket.split();

    let result: anyhow::Result<()> = async {
        loop {
            tokio::select! {
                _ = bridge.shutdown.cancelled() => {
                    let _ = ws_tx.send(Message::Close(None)).await;
                    return Ok(());
                }
                line = lines.next_line() => match line? {
                    Some(line) => {
                        if ws_tx.send(Message::Text(line.into())).await.is_err() {
                            return Ok(());
                        }
                    }
                    None => {
                        let _ = ws_tx.send(Message::Close(None)).await;
                        anyhow::bail!("codex app-server exited");
                    }
                },
                message = ws_rx.next() => match message {
                    Some(Ok(Message::Text(text))) => {
                        // Re-encode so a message is always exactly one stdio line.
                        let value: serde_json::Value = match serde_json::from_str(text.as_str()) {
                            Ok(value) => value,
                            Err(_) => continue,
                        };
                        let mut line = serde_json::to_vec(&value)?;
                        line.push(b'\n');
                        stdin.write_all(&line).await?;
                        stdin.flush().await?;
                    }
                    Some(Ok(Message::Close(_)) | Err(_)) | None => return Ok(()),
                    Some(Ok(_)) => {}
                },
            }
        }
    }
    .await;

    // EOF makes the app-server shut down gracefully and release its session locks.
    drop(stdin);
    if tokio::time::timeout(CHILD_SHUTDOWN_TIMEOUT, child.wait())
        .await
        .is_err()
    {
        let _ = child.kill().await;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn query_param_decodes_values() {
        assert_eq!(
            query_param(Some("token=abc&cwd=%2Fhome%2Fa%20b"), "cwd"),
            Some("/home/a b".to_string())
        );
        assert_eq!(query_param(Some("token=abc"), "cwd"), None);
        assert_eq!(query_param(None, "cwd"), None);
    }

    /// `/init` must send the same prompt as the TUI. Skipped where the TUI sources
    /// are not available (for example in sandboxed Bazel test runs).
    #[test]
    fn init_prompt_matches_tui() {
        let tui_prompt =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../tui/assets/prompt_for_init_command.md");
        if let Ok(tui_prompt) = std::fs::read_to_string(tui_prompt) {
            assert_eq!(INIT_PROMPT, tui_prompt);
        }
    }

    #[test]
    fn constant_time_eq_compares_contents() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }
}
