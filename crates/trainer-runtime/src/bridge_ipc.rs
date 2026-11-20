//! Shared authenticated loopback JSON-line IPC & LIFO rollback helpers for
//! managed loader bridge runtimes (`unity_bridge_v1` and `ue4ss_bridge_v1`).

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use nekod_manifest::RuntimeKind;
use nekod_runtime_api::{
    ProcessIdentity, RuntimeDiagnostics, RuntimeError, RuntimeSessionId, SessionLifecycleState,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const MAX_AUTH_TOKEN_LEN: usize = 128;
const MIN_AUTH_TOKEN_LEN: usize = 16;

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
            let rpc_result = if session.use_unity_explorer_http {
                send_unity_explorer_http_rpc(session.endpoint, &undo_req.op, &undo_req.params)
                    .map(|_| ())
            } else {
                send_bridge_rpc(session.endpoint, &undo_req).map(|_| ())
            };
            if let Err(err) = rpc_result {
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
        TcpStream::connect_timeout(&endpoint, Duration::from_millis(800)).map_err(|e| {
            RuntimeError::ExecutionFailed(format!("bridge connect {endpoint} failed: {e}"))
        })?;
    let _ = stream.set_read_timeout(Some(Duration::from_millis(1200)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(1200)));

    let line = serde_json::to_string(req)
        .map_err(|e| RuntimeError::InvalidPayload(format!("serialize bridge RPC failed: {e}")))?;
    writeln!(stream, "{line}")
        .and_then(|()| stream.flush())
        .map_err(|e| RuntimeError::ExecutionFailed(format!("bridge write failed: {e}")))?;

    let mut reader = BufReader::new(stream);
    let mut resp_line = String::new();
    let n = reader
        .read_line(&mut resp_line)
        .map_err(|e| RuntimeError::ExecutionFailed(format!("bridge read failed: {e}")))?;
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
