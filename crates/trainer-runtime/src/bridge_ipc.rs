//! Shared authenticated loopback JSON-line IPC & LIFO rollback helpers for
//! managed loader bridge runtimes (`unity_bridge_v1` and `ue4ss_bridge_v1`).

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nekod_manifest::RuntimeKind;
use nekod_runtime_api::{
    ProcessIdentity, RuntimeDiagnostics, RuntimeError, RuntimeSessionId, SessionLifecycleState,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const MAX_AUTH_TOKEN_LEN: usize = 128;
const MIN_AUTH_TOKEN_LEN: usize = 16;
static FILE_IPC_SEQ: AtomicU64 = AtomicU64::new(1);
static FILE_IPC_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct BridgeRpcRequest {
    pub token: String,
    pub op: String,
    pub params: Value,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct BridgeRpcResponse {
    pub ok: bool,
    #[serde(default)]
    pub data: Value,
    #[serde(default)]
    pub error: Option<String>,
}

pub(crate) struct ManagedBridgeSession {
    pub endpoint: SocketAddr,
    pub auth_token: String,
    pub target_process: ProcessIdentity,
    pub runtime_kind: RuntimeKind,
    pub use_unity_explorer_http: bool,
    pub file_ipc_dir: Option<PathBuf>,
    pub state: SessionLifecycleState,
    pub rollback_stack: Vec<(String, BridgeRpcRequest)>,
    pub rollback_failures: usize,
    pub recent_events: Vec<String>,
}

pub(crate) struct BridgeSessionStore {
    next_id: AtomicU64,
    sessions: Mutex<BTreeMap<String, ManagedBridgeSession>>,
    stopped_diagnostics: Mutex<BTreeMap<String, RuntimeDiagnostics>>,
}

impl BridgeSessionStore {
    pub fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            sessions: Mutex::new(BTreeMap::new()),
            stopped_diagnostics: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn insert(&self, prefix: &str, session: ManagedBridgeSession) -> RuntimeSessionId {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let sid = RuntimeSessionId(format!("{prefix}-{id}"));
        self.sessions.lock().unwrap().insert(sid.0.clone(), session);
        sid
    }

    /// Reads `(endpoint, auth_token, use_unity_explorer_http)` without holding the session lock
    /// during blocking network I/O.
    pub fn session_connection(
        &self,
        session_id: &RuntimeSessionId,
    ) -> Result<(SocketAddr, String, bool), RuntimeError> {
        let guard = self.sessions.lock().unwrap();
        let session = guard
            .get(&session_id.0)
            .ok_or_else(|| RuntimeError::SessionNotFound(session_id.clone()))?;
        Ok((
            session.endpoint,
            session.auth_token.clone(),
            session.use_unity_explorer_http,
        ))
    }

    /// Reads `(endpoint, auth_token, file_ipc_dir)` without holding the session lock
    /// during blocking RPC I/O.
    pub fn session_transport(
        &self,
        session_id: &RuntimeSessionId,
    ) -> Result<(SocketAddr, String, Option<PathBuf>), RuntimeError> {
        let guard = self.sessions.lock().unwrap();
        let session = guard
            .get(&session_id.0)
            .ok_or_else(|| RuntimeError::SessionNotFound(session_id.clone()))?;
        Ok((
            session.endpoint,
            session.auth_token.clone(),
            session.file_ipc_dir.clone(),
        ))
    }

    pub fn with_session_mut<F, R>(
        &self,
        session_id: &RuntimeSessionId,
        f: F,
    ) -> Result<R, RuntimeError>
    where
        F: FnOnce(&mut ManagedBridgeSession) -> Result<R, RuntimeError>,
    {
        let mut guard = self.sessions.lock().unwrap();
        let session = guard
            .get_mut(&session_id.0)
            .ok_or_else(|| RuntimeError::SessionNotFound(session_id.clone()))?;
        f(session)
    }

    pub fn stop_and_rollback(
        &self,
        session_id: &RuntimeSessionId,
    ) -> Result<RuntimeDiagnostics, RuntimeError> {
        let mut session = self
            .sessions
            .lock()
            .unwrap()
            .remove(&session_id.0)
            .ok_or_else(|| RuntimeError::SessionNotFound(session_id.clone()))?;

        while let Some((fid, undo_req)) = session.rollback_stack.pop() {
            let rpc_result = if let Some(ipc_dir) = session.file_ipc_dir.as_deref() {
                send_file_ipc_rpc(ipc_dir, &undo_req).map(|_| ())
            } else if session.use_unity_explorer_http {
                send_unity_explorer_http_rpc(session.endpoint, &undo_req.op, &undo_req.params)
                    .map(|_| ())
            } else {
                send_bridge_rpc(session.endpoint, &undo_req).map(|_| ())
            };
            if let Err(err) = rpc_result {
                if is_bridge_disconnected_error(&err) {
                    let skipped = session.rollback_stack.len();
                    session.rollback_failures += 1 + skipped;
                    session.rollback_stack.clear();
                    session.recent_events.push(format!(
                        "Bridge disconnected while rolling back '{fid}' ({err}); aborted early and skipped {skipped} remaining rollback(s) because target game exited"
                    ));
                    break;
                }
                session.rollback_failures += 1;
                session
                    .recent_events
                    .push(format!("Rollback failed for '{fid}': {err}"));
            } else {
                session
                    .recent_events
                    .push(format!("Rolled back '{fid}' cleanly via bridge"));
            }
        }

        session.state = SessionLifecycleState::Stopped;
        let diag = build_diagnostics(session_id, &session);
        self.stopped_diagnostics
            .lock()
            .unwrap()
            .insert(session_id.0.clone(), diag.clone());
        Ok(diag)
    }

    pub fn diagnostics(
        &self,
        session_id: &RuntimeSessionId,
    ) -> Result<RuntimeDiagnostics, RuntimeError> {
        if let Some(session) = self.sessions.lock().unwrap().get(&session_id.0) {
            return Ok(build_diagnostics(session_id, session));
        }
        self.stopped_diagnostics
            .lock()
            .unwrap()
            .get(&session_id.0)
            .cloned()
            .ok_or_else(|| RuntimeError::SessionNotFound(session_id.clone()))
    }
}

fn is_bridge_disconnected_error(err: &RuntimeError) -> bool {
    let lower = err.to_string().to_ascii_lowercase();
    lower.contains("connect")
        || lower.contains("10061")
        || lower.contains("refused")
        || lower.contains("timed out")
        || lower.contains("file ipc timeout")
        || lower.contains("closed connection")
        || lower.contains("10054")
        || lower.contains("10053")
        || lower.contains("broken pipe")
}

fn build_diagnostics(
    session_id: &RuntimeSessionId,
    session: &ManagedBridgeSession,
) -> RuntimeDiagnostics {
    RuntimeDiagnostics {
        session_id: session_id.clone(),
        runtime_kind: session.runtime_kind,
        state: session.state,
        active_patches: session.rollback_stack.len(),
        rollback_failures: session.rollback_failures,
        rollback_conflicts: 0,
        active_freeze_features: 0,
        target_pid: Some(session.target_process.pid),
        target_process_name: Some(session.target_process.process_name.clone()),
        recent_events: session.recent_events.clone(),
    }
}

/// Validates that `host:port` resolves exclusively to a loopback IP (for SSRF protection).
pub(crate) fn validate_loopback_host_port(
    host: &str,
    port: u16,
) -> Result<SocketAddr, RuntimeError> {
    if port == 0 {
        return Err(RuntimeError::InvalidPayload(
            "bridge port must be non-zero".to_owned(),
        ));
    }
    let ip: IpAddr = match host {
        "localhost" => "127.0.0.1".parse().unwrap(),
        other => other.parse().map_err(|_| {
            RuntimeError::InvalidPayload(format!(
                "bridge endpoint requires loopback IP or 'localhost', got '{other}'"
            ))
        })?,
    };
    if !ip.is_loopback() {
        return Err(RuntimeError::InvalidPayload(format!(
            "bridge refuses non-loopback endpoint '{ip}'"
        )));
    }
    Ok(SocketAddr::new(ip, port))
}

/// Validates that `host:port` resolves exclusively to a loopback IP and `auth_token` meets
/// per-session entropy length requirements.
pub(crate) fn validate_loopback_bridge(
    host: &str,
    port: u16,
    auth_token: &str,
) -> Result<SocketAddr, RuntimeError> {
    let addr = validate_loopback_host_port(host, port)?;
    let trimmed = auth_token.trim();
    if !(MIN_AUTH_TOKEN_LEN..=MAX_AUTH_TOKEN_LEN).contains(&trimmed.len()) {
        return Err(RuntimeError::InvalidPayload(format!(
            "bridge auth_token length must be {MIN_AUTH_TOKEN_LEN}..={MAX_AUTH_TOKEN_LEN} characters"
        )));
    }
    Ok(addr)
}

/// Sends an HTTP/1.1 `POST /` JSON request compatible with `OmaruKoma/UnityExplorerMCP` (`MCPBridge.dll`).
pub(crate) fn send_unity_explorer_http_rpc(
    endpoint: SocketAddr,
    method: &str,
    params: &Value,
) -> Result<Value, RuntimeError> {
    let body = serde_json::to_string(&json!({
        "method": method,
        "params": params,
    }))
    .map_err(|e| {
        RuntimeError::InvalidPayload(format!("serialize UnityExplorerMCP RPC failed: {e}"))
    })?;

    let mut stream =
        TcpStream::connect_timeout(&endpoint, Duration::from_millis(1200)).map_err(|e| {
            RuntimeError::ExecutionFailed(format!(
                "UnityExplorerMCP bridge connect {endpoint} failed: {e}"
            ))
        })?;
    let _ = stream.set_read_timeout(Some(Duration::from_millis(5000)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(2000)));

    let request = format!(
        "POST / HTTP/1.1\r\nHost: {endpoint}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream
        .write_all(request.as_bytes())
        .and_then(|()| stream.flush())
        .map_err(|e| {
            RuntimeError::ExecutionFailed(format!("UnityExplorerMCP write failed: {e}"))
        })?;

    let mut reader = BufReader::new(stream);
    let mut header_line = String::new();
    let mut content_length: Option<usize> = None;
    let mut saw_status = false;
    loop {
        header_line.clear();
        let n = reader.read_line(&mut header_line).map_err(|e| {
            RuntimeError::ExecutionFailed(format!("UnityExplorerMCP read header failed: {e}"))
        })?;
        if n == 0 {
            break;
        }
        let trimmed = header_line.trim_end_matches(['\r', '\n']);
        if !saw_status {
            saw_status = true;
            if !trimmed.starts_with("HTTP/1.") {
                return Err(RuntimeError::ExecutionFailed(format!(
                    "invalid UnityExplorerMCP HTTP status line '{trimmed}'"
                )));
            }
            continue;
        }
        if trimmed.is_empty() {
            break;
        }
        if let Some((k, v)) = trimmed.split_once(':')
            && k.trim().eq_ignore_ascii_case("content-length")
            && let Ok(len) = v.trim().parse::<usize>()
        {
            content_length = Some(len);
        }
    }

    let raw_body = match content_length {
        Some(len) => {
            let mut buf = vec![0u8; len.min(8 * 1024 * 1024)];
            reader.read_exact(&mut buf).map_err(|e| {
                RuntimeError::ExecutionFailed(format!("UnityExplorerMCP read body failed: {e}"))
            })?;
            String::from_utf8(buf).map_err(|e| {
                RuntimeError::ExecutionFailed(format!("UnityExplorerMCP non-UTF8 body: {e}"))
            })?
        }
        None => {
            let mut s = String::new();
            reader.read_to_string(&mut s).map_err(|e| {
                RuntimeError::ExecutionFailed(format!("UnityExplorerMCP read body failed: {e}"))
            })?;
            s
        }
    };

    let parsed: Value = serde_json::from_str(&raw_body).map_err(|e| {
        RuntimeError::ExecutionFailed(format!("invalid UnityExplorerMCP response JSON: {e}"))
    })?;
    let success = parsed
        .get("Success")
        .or_else(|| parsed.get("success"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !success {
        let err_msg = parsed
            .get("Error")
            .or_else(|| parsed.get("error"))
            .and_then(Value::as_str)
            .unwrap_or("UnityExplorerMCP bridge rejected operation");
        return Err(RuntimeError::ExecutionFailed(err_msg.to_owned()));
    }

    Ok(parsed
        .get("Data")
        .or_else(|| parsed.get("data"))
        .cloned()
        .unwrap_or(Value::Null))
}

pub(crate) fn send_bridge_rpc(
    endpoint: SocketAddr,
    req: &BridgeRpcRequest,
) -> Result<BridgeRpcResponse, RuntimeError> {
    let mut stream =
        TcpStream::connect_timeout(&endpoint, Duration::from_millis(1500)).map_err(|e| {
            RuntimeError::ExecutionFailed(format!("bridge connect {endpoint} failed: {e}"))
        })?;
    let _ = stream.set_read_timeout(Some(Duration::from_millis(4500)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(2000)));

    let line = serde_json::to_string(req)
        .map_err(|e| RuntimeError::InvalidPayload(format!("serialize bridge RPC failed: {e}")))?;
    writeln!(stream, "{line}")
        .and_then(|()| stream.flush())
        .map_err(|e| RuntimeError::ExecutionFailed(format!("bridge write failed: {e}")))?;

    let mut reader = BufReader::new(stream);
    let mut resp_line = String::new();
    let n = reader.read_line(&mut resp_line).map_err(|e| {
        if matches!(
            e.kind(),
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
        ) {
            RuntimeError::ExecutionFailed(format!(
                "bridge read timed out after 4.5s (game main thread may be loading or paused; wait until gameplay resumes and retry): {e}"
            ))
        } else {
            RuntimeError::ExecutionFailed(format!("bridge read failed: {e}"))
        }
    })?;
    if n == 0 {
        return Err(RuntimeError::ExecutionFailed(
            "bridge closed connection without response".to_owned(),
        ));
    }

    let resp: BridgeRpcResponse = serde_json::from_str(&resp_line)
        .map_err(|e| RuntimeError::ExecutionFailed(format!("invalid bridge response JSON: {e}")))?;
    if !resp.ok {
        return Err(RuntimeError::ExecutionFailed(
            resp.error
                .unwrap_or_else(|| "bridge rejected operation".to_owned()),
        ));
    }
    Ok(resp)
}

pub(crate) fn send_file_ipc_rpc(
    ipc_dir: &Path,
    req: &BridgeRpcRequest,
) -> Result<BridgeRpcResponse, RuntimeError> {
    if !ipc_dir.is_dir() {
        return Err(RuntimeError::ExecutionFailed(format!(
            "UE4SS file IPC directory '{}' does not exist",
            ipc_dir.display()
        )));
    }
    let trimmed = req.token.trim();
    if !(MIN_AUTH_TOKEN_LEN..=MAX_AUTH_TOKEN_LEN).contains(&trimmed.len()) {
        return Err(RuntimeError::InvalidPayload(format!(
            "bridge auth_token length must be {MIN_AUTH_TOKEN_LEN}..={MAX_AUTH_TOKEN_LEN} characters"
        )));
    }

    let _serial = FILE_IPC_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let seq = FILE_IPC_SEQ.fetch_add(1, Ordering::Relaxed);
    let req_id = format!("{}_{}_{}", std::process::id(), nanos, seq);

    let tmp_path = ipc_dir.join("ue4ss-request.tmp");
    let req_path = ipc_dir.join("ue4ss-request.json");
    let resp_path = ipc_dir.join(format!("ue4ss-response-{req_id}.json"));

    let payload = json!({
        "id": req_id,
        "token": trimmed,
        "op": req.op,
        "params": req.params,
    });
    let serialized = serde_json::to_vec(&payload)
        .map_err(|e| RuntimeError::InvalidPayload(format!("serialize file IPC RPC failed: {e}")))?;

    fs::write(&tmp_path, &serialized).map_err(|e| {
        RuntimeError::ExecutionFailed(format!(
            "failed writing UE4SS IPC request temp '{}': {e}",
            tmp_path.display()
        ))
    })?;
    if req_path.exists() {
        let _ = fs::remove_file(&req_path);
    }
    if fs::rename(&tmp_path, &req_path).is_err() {
        fs::write(&req_path, &serialized).map_err(|e| {
            let _ = fs::remove_file(&tmp_path);
            RuntimeError::ExecutionFailed(format!(
                "failed writing UE4SS IPC request '{}': {e}",
                req_path.display()
            ))
        })?;
        let _ = fs::remove_file(&tmp_path);
    }

    let deadline = Instant::now() + Duration::from_millis(3500);
    while Instant::now() < deadline {
        if resp_path.is_file() {
            // Brief retry on Windows if rename finished right before reader opened it
            let raw = match fs::read_to_string(&resp_path) {
                Ok(s) => s,
                Err(_) => {
                    thread::sleep(Duration::from_millis(15));
                    fs::read_to_string(&resp_path).map_err(|e| {
                        RuntimeError::ExecutionFailed(format!(
                            "failed reading UE4SS IPC response '{}': {e}",
                            resp_path.display()
                        ))
                    })?
                }
            };
            let _ = fs::remove_file(&resp_path);

            let resp: BridgeRpcResponse = serde_json::from_str(&raw).map_err(|e| {
                RuntimeError::ExecutionFailed(format!("invalid UE4SS IPC response JSON: {e}"))
            })?;
            if !resp.ok {
                return Err(RuntimeError::ExecutionFailed(
                    resp.error
                        .unwrap_or_else(|| "UE4SS bridge rejected operation".to_owned()),
                ));
            }
            return Ok(resp);
        }
        thread::sleep(Duration::from_millis(25));
    }

    let _ = fs::remove_file(&req_path);
    Err(RuntimeError::ExecutionFailed(
        "UE4SS file IPC timeout after 3.5s (ensure RE-UE4SS + NekodBridge are loaded in the running UE game)"
            .to_owned(),
    ))
}
