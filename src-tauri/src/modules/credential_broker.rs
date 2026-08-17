//! Private IPC credential broker for the unified model gateway sidecar.
//!
//! Refresh tokens and long-lived API keys never leave the Rust process. The
//! sidecar receives only a short-lived Grok access token, or a streamed
//! provider response after Rust injects the API key.

use crate::models::unified_model_gateway::UNIFIED_GATEWAY_PROTOCOL_VERSION;
use crate::modules::{account, grok_account, local_secret_blob, logger};
use rand::RngCore;
use ring::hmac;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex as TokioMutex;

const BROKER_SOCKET_NAME: &str = "credential-broker.sock";
const BROKER_PIPE_PREFIX: &str = "cockpit-ugw-broker-";
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const SESSION_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct BrokerLaunchSecrets {
    pub session_key: [u8; 32],
    pub nonce: [u8; 16],
}

impl BrokerLaunchSecrets {
    pub fn generate() -> Self {
        let mut session_key = [0u8; 32];
        let mut nonce = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut session_key);
        rand::thread_rng().fill_bytes(&mut nonce);
        Self { session_key, nonce }
    }

    pub fn encode_handshake(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(49);
        out.push(1);
        out.extend_from_slice(&self.session_key);
        out.extend_from_slice(&self.nonce);
        out
    }

    pub fn decode_handshake(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < 49 || bytes[0] != 1 {
            return Err("credential broker handshake payload is invalid".to_string());
        }
        let mut session_key = [0u8; 32];
        let mut nonce = [0u8; 16];
        session_key.copy_from_slice(&bytes[1..33]);
        nonce.copy_from_slice(&bytes[33..49]);
        Ok(Self { session_key, nonce })
    }
}

#[derive(Debug, Clone)]
pub struct BrokerEndpoint {
    pub socket_path: PathBuf,
    #[cfg(windows)]
    pub pipe_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrokerHello {
    protocol_version: u32,
    child_pid: u32,
    nonce: String,
    hmac: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
enum BrokerRequest {
    #[serde(rename = "get_grok_access_token", rename_all = "camelCase")]
    GetGrokAccessToken {
        seq: u64,
        account_id: String,
        hmac: String,
    },
    #[serde(rename = "mark_grok_account", rename_all = "camelCase")]
    MarkGrokAccount {
        seq: u64,
        account_id: String,
        status: String,
        hmac: String,
    },
    #[serde(rename = "execute_provider_request", rename_all = "camelCase")]
    ExecuteProviderRequest {
        seq: u64,
        provider_id: String,
        model_route: String,
        request: Value,
        hmac: String,
    },
}

#[derive(Debug)]
struct BrokerSession {
    child_pid: u32,
    session_key: [u8; 32],
    expected_seq: u64,
    last_seen: Instant,
}

struct BrokerInner {
    listen_path: PathBuf,
    #[cfg(windows)]
    pipe_name: String,
    pending_nonce: Option<[u8; 16]>,
    pending_key: Option<[u8; 32]>,
    pending_child_pid: Option<u32>,
    session: Option<BrokerSession>,
    stop: AtomicBool,
    generation: AtomicU64,
}

static BROKER: Mutex<Option<Arc<TokioMutex<BrokerInner>>>> = Mutex::new(None);
static SECRET_LOCKS: std::sync::LazyLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn runtime_dir() -> Result<PathBuf, String> {
    let path = account::get_data_dir()?.join("runtime");
    std::fs::create_dir_all(&path)
        .map_err(|error| format!("创建 credential broker 运行目录失败: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700));
    }
    Ok(path)
}

pub fn default_socket_path() -> Result<PathBuf, String> {
    Ok(runtime_dir()?.join(BROKER_SOCKET_NAME))
}

#[cfg(windows)]
pub fn default_pipe_name() -> String {
    let user = std::env::var("USERNAME").unwrap_or_else(|_| "user".to_string());
    format!(r"\\.\pipe\{BROKER_PIPE_PREFIX}{user}")
}

pub fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn decode_hex(value: &str) -> Result<Vec<u8>, String> {
    let value = value.trim();
    if value.len() % 2 != 0 {
        return Err("hex 长度无效".to_string());
    }
    (0..value.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&value[index..index + 2], 16).map_err(|_| "hex 内容无效".to_string())
        })
        .collect()
}

pub fn sign_payload(session_key: &[u8], payload: &[u8]) -> String {
    let key = hmac::Key::new(hmac::HMAC_SHA256, session_key);
    encode_hex(hmac::sign(&key, payload).as_ref())
}

pub fn verify_payload(session_key: &[u8], payload: &[u8], signature: &str) -> bool {
    let Ok(expected) = decode_hex(signature) else {
        return false;
    };
    let key = hmac::Key::new(hmac::HMAC_SHA256, session_key);
    hmac::verify(&key, payload, &expected).is_ok()
}

fn hello_payload(protocol_version: u32, child_pid: u32, nonce_hex: &str) -> Vec<u8> {
    format!("hello|{protocol_version}|{child_pid}|{nonce_hex}").into_bytes()
}

fn request_payload(seq: u64, kind: &str, body: &str) -> Vec<u8> {
    format!("{seq}|{kind}|{body}").into_bytes()
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn secret_lock(secret_id: &str) -> Result<Arc<tokio::sync::Mutex<()>>, String> {
    let mut locks = SECRET_LOCKS
        .lock()
        .map_err(|_| "获取 API Key 锁失败".to_string())?;
    Ok(locks
        .entry(secret_id.to_string())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone())
}

pub fn secrets_dir() -> Result<PathBuf, String> {
    let path = account::get_data_dir()?.join("unified_gateway_secrets");
    std::fs::create_dir_all(&path).map_err(|error| format!("创建统一网关密钥目录失败: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700));
    }
    Ok(path)
}

pub fn write_api_key_secret(secret_id: &str, api_key: &str) -> Result<(), String> {
    let path = secrets_dir()?.join(format!("{secret_id}.json"));
    let payload = json!({
        "id": secret_id,
        "apiKey": api_key,
        "updatedAt": now_ms(),
    });
    local_secret_blob::write_secret_file(
        &path,
        &serde_json::to_string(&payload).map_err(|error| error.to_string())?,
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

pub fn read_api_key_secret(secret_id: &str) -> Result<String, String> {
    let path = secrets_dir()?.join(format!("{secret_id}.json"));
    let raw = local_secret_blob::read_secret_file(&path)?;
    if raw.trim().is_empty() {
        return Err("API Key 不存在".to_string());
    }
    let value: Value =
        serde_json::from_str(&raw).map_err(|error| format!("解析 API Key 失败: {error}"))?;
    value
        .get("apiKey")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| "API Key 为空".to_string())
}

pub fn delete_api_key_secret(secret_id: &str) -> Result<(), String> {
    let path = secrets_dir()?.join(format!("{secret_id}.json"));
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("删除 API Key 失败: {error}")),
    }
}

pub fn redact_text(value: &str) -> String {
    let mut redacted = value.to_string();
    if let Some(index) = redacted.to_ascii_lowercase().find("bearer ") {
        let rest = &redacted[index + "bearer ".len()..];
        let token_len = rest
            .find(|ch: char| ch.is_whitespace() || ch == '"' || ch == '\'')
            .unwrap_or(rest.len());
        redacted.replace_range(index..index + "bearer ".len() + token_len, "[redacted]");
    }
    for needle in [
        "Authorization",
        "authorization",
        "refresh_token",
        "access_token",
        "api_key",
        "apiKey",
    ] {
        if redacted.contains(needle) {
            redacted = redacted.replace(needle, "[redacted]");
        }
    }
    if let Some(index) =
        redacted.find(crate::models::unified_model_gateway::UNIFIED_GATEWAY_ENDPOINT_MARKER)
    {
        let rest = &redacted[index..];
        if let Some(end) = rest.find("/v1") {
            let prefix = &redacted[..index];
            let suffix = &rest[end..];
            redacted = format!("{prefix}/_cockpit-ugw/[capability]{suffix}");
        }
    }
    redacted
}

pub fn fingerprint_secret(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    encode_hex(&digest[..8])
}

async fn handle_get_grok_token(account_id: &str) -> Result<Value, String> {
    let account = grok_account::prepare_account_for_injection(account_id).await?;
    if account.is_api_key_auth() {
        return Err("xAI API Key 账号不能作为 Grok OAuth 凭据使用".to_string());
    }
    if account.access_token.trim().is_empty() {
        return Err("Grok 账号需要重新授权".to_string());
    }
    Ok(json!({
        "type": "grok_access_token",
        "accountId": account.id,
        "accessToken": account.access_token,
        "expiresAt": account.expires_at,
    }))
}

async fn handle_mark_grok_account(account_id: &str, status: &str) -> Result<Value, String> {
    grok_account::mark_account_status(account_id, status)?;
    Ok(json!({
        "type": "ok",
        "accountId": account_id,
        "status": status,
    }))
}

async fn handle_execute_provider(
    provider_id: &str,
    model_route: &str,
    request: &Value,
) -> Result<Value, String> {
    let _ = secret_lock(provider_id)?;
    let store = crate::modules::unified_model_gateway::load_store()?;
    let provider = store
        .providers
        .iter()
        .find(|item| item.id == provider_id && item.enabled)
        .ok_or_else(|| format!("Provider {provider_id} 未启用"))?;
    if !matches!(
        provider.provider_type,
        crate::models::unified_model_gateway::UnifiedProviderType::XaiApi
            | crate::models::unified_model_gateway::UnifiedProviderType::OpenaiCompatible
    ) {
        return Err("该 Provider 不支持 API Key 执行".to_string());
    }
    let cred = store
        .credential_refs
        .iter()
        .find(|item| {
            item.enabled
                && item.kind == crate::models::unified_model_gateway::UnifiedCredentialKind::ApiKey
                && provider.credential_ref_ids.iter().any(|id| id == &item.id)
        })
        .ok_or_else(|| "Provider 没有可用的 API Key".to_string())?;
    let secret_id = cred
        .secret_id
        .as_deref()
        .ok_or_else(|| "Provider 缺少 secretId".to_string())?;
    let api_key = read_api_key_secret(secret_id)?;
    let base_url = provider
        .base_url
        .as_deref()
        .ok_or_else(|| "Provider 缺少 Base URL".to_string())?;
    let wire = provider
        .wire_api
        .as_deref()
        .unwrap_or("responses")
        .to_ascii_lowercase();
    let path = if wire.contains("chat") {
        "/v1/chat/completions"
    } else {
        "/v1/responses"
    };
    let url = format!("{}{}", base_url.trim_end_matches('/'), path);
    let mut body = request.clone();
    if let Some(object) = body.as_object_mut() {
        object.insert("model".to_string(), Value::String(model_route.to_string()));
    }
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(90))
        .build()
        .map_err(|error| format!("创建 Provider 客户端失败: {error}"))?;
    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|error| format!("Provider 请求失败: {error}"))?;
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/json")
        .to_string();
    let payload = response
        .bytes()
        .await
        .map_err(|error| format!("读取 Provider 响应失败: {error}"))?;
    Ok(json!({
        "type": "provider_response",
        "status": status,
        "contentType": content_type,
        "body": String::from_utf8_lossy(&payload),
    }))
}

fn read_frame<R: Read>(reader: &mut R) -> Result<Vec<u8>, String> {
    let mut len_buf = [0u8; 4];
    reader
        .read_exact(&mut len_buf)
        .map_err(|error| format!("读取 broker 帧长度失败: {error}"))?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len == 0 || len > MAX_FRAME_BYTES {
        return Err("broker 帧长度非法".to_string());
    }
    let mut payload = vec![0u8; len];
    reader
        .read_exact(&mut payload)
        .map_err(|error| format!("读取 broker 帧失败: {error}"))?;
    Ok(payload)
}

fn write_frame<W: Write>(writer: &mut W, payload: &[u8]) -> Result<(), String> {
    if payload.len() > MAX_FRAME_BYTES {
        return Err("broker 响应过大".to_string());
    }
    writer
        .write_all(&(payload.len() as u32).to_le_bytes())
        .and_then(|_| writer.write_all(payload))
        .and_then(|_| writer.flush())
        .map_err(|error| format!("写入 broker 帧失败: {error}"))
}

async fn process_request(request: BrokerRequest) -> Value {
    match request {
        BrokerRequest::GetGrokAccessToken { account_id, .. } => {
            match handle_get_grok_token(&account_id).await {
                Ok(value) => value,
                Err(error) => json!({
                    "type": "error",
                    "code": classify_grok_error(&error),
                    "message": redact_text(&error),
                }),
            }
        }
        BrokerRequest::MarkGrokAccount {
            account_id, status, ..
        } => match handle_mark_grok_account(&account_id, &status).await {
            Ok(value) => value,
            Err(error) => json!({
                "type": "error",
                "code": "mark_account_failed",
                "message": redact_text(&error),
            }),
        },
        BrokerRequest::ExecuteProviderRequest {
            provider_id,
            model_route,
            request,
            ..
        } => match handle_execute_provider(&provider_id, &model_route, &request).await {
            Ok(value) => value,
            Err(error) => json!({
                "type": "error",
                "code": "provider_execute_failed",
                "message": redact_text(&error),
            }),
        },
    }
}

fn classify_grok_error(error: &str) -> &'static str {
    let lower = error.to_ascii_lowercase();
    if lower.contains("重新授权") || lower.contains("invalid_grant") || lower.contains("reauth")
    {
        "grok_reauth_required"
    } else if lower.contains("额度") || lower.contains("quota") || lower.contains("429") {
        "grok_quota_exhausted"
    } else {
        "grok_token_failed"
    }
}

fn verify_request(session: &BrokerSession, request: &BrokerRequest) -> Result<(), String> {
    if Instant::now().duration_since(session.last_seen) > SESSION_IDLE_TIMEOUT {
        return Err("broker 会话已超时".to_string());
    }
    let (seq, kind, body, signature) = match request {
        BrokerRequest::GetGrokAccessToken {
            seq,
            account_id,
            hmac,
        } => (
            *seq,
            "get_grok_access_token",
            account_id.clone(),
            hmac.clone(),
        ),
        BrokerRequest::MarkGrokAccount {
            seq,
            account_id,
            status,
            hmac,
        } => (
            *seq,
            "mark_grok_account",
            format!("{account_id}|{status}"),
            hmac.clone(),
        ),
        BrokerRequest::ExecuteProviderRequest {
            seq,
            provider_id,
            model_route,
            hmac,
            ..
        } => (
            *seq,
            "execute_provider",
            format!("{provider_id}|{model_route}"),
            hmac.clone(),
        ),
    };
    if seq != session.expected_seq {
        return Err("broker 请求序号无效".to_string());
    }
    if !verify_payload(
        &session.session_key,
        &request_payload(seq, kind, &body),
        &signature,
    ) {
        return Err("broker 请求签名无效".to_string());
    }
    Ok(())
}

#[cfg(unix)]
async fn serve_unix_connection(
    inner: Arc<TokioMutex<BrokerInner>>,
    mut stream: tokio::net::UnixStream,
    peer_pid: Option<u32>,
) {
    let mut len_buf = [0u8; 4];
    if timeout_read(&mut stream, &mut len_buf).await.is_err() {
        return;
    }
    let len = u32::from_le_bytes(len_buf) as usize;
    if len == 0 || len > MAX_FRAME_BYTES {
        return;
    }
    let mut payload = vec![0u8; len];
    if timeout_read(&mut stream, &mut payload).await.is_err() {
        return;
    }
    let hello: BrokerHello = match serde_json::from_slice(&payload) {
        Ok(value) => value,
        Err(_) => return,
    };
    let mut guard = inner.lock().await;
    if hello.protocol_version != UNIFIED_GATEWAY_PROTOCOL_VERSION {
        let _ = write_async_frame(
            &mut stream,
            &json!({"type":"error","code":"protocol_mismatch","message":"unsupported broker protocol"}),
        )
        .await;
        return;
    }
    let Some(expected_nonce) = guard.pending_nonce.take() else {
        return;
    };
    let Some(session_key) = guard.pending_key.take() else {
        return;
    };
    let expected_pid = guard.pending_child_pid.filter(|pid| *pid != 0);
    if expected_pid.is_some_and(|pid| pid != hello.child_pid)
        || peer_pid.is_some_and(|pid| pid != hello.child_pid)
    {
        logger::log_warn("[UnifiedGateway] credential broker 拒绝不匹配的 sidecar PID");
        return;
    }
    let nonce = match decode_hex(&hello.nonce) {
        Ok(value) => value,
        Err(_) => return,
    };
    if nonce.as_slice() != expected_nonce.as_slice() {
        return;
    }
    if !verify_payload(
        &session_key,
        &hello_payload(hello.protocol_version, hello.child_pid, &hello.nonce),
        &hello.hmac,
    ) {
        return;
    }
    guard.session = Some(BrokerSession {
        child_pid: hello.child_pid,
        session_key,
        expected_seq: 1,
        last_seen: Instant::now(),
    });
    drop(guard);
    let _ = write_async_frame(&mut stream, &json!({"type":"hello_ok"})).await;

    loop {
        let mut len_buf = [0u8; 4];
        if timeout_read_long(&mut stream, &mut len_buf).await.is_err() {
            break;
        }
        let len = u32::from_le_bytes(len_buf) as usize;
        if len == 0 || len > MAX_FRAME_BYTES {
            break;
        }
        let mut payload = vec![0u8; len];
        if timeout_read_long(&mut stream, &mut payload).await.is_err() {
            break;
        }
        let request: BrokerRequest = match serde_json::from_slice(&payload) {
            Ok(value) => value,
            Err(error) => {
                let _ = write_async_frame(
                    &mut stream,
                    &json!({"type":"error","code":"bad_request","message": redact_text(&error.to_string())}),
                )
                .await;
                continue;
            }
        };
        {
            let mut guard = inner.lock().await;
            let Some(session) = guard.session.as_mut() else {
                break;
            };
            if let Err(error) = verify_request(session, &request) {
                let _ = write_async_frame(
                    &mut stream,
                    &json!({"type":"error","code":"auth_failed","message": error}),
                )
                .await;
                guard.session = None;
                break;
            }
            session.expected_seq += 1;
            session.last_seen = Instant::now();
        }
        let response = process_request(request).await;
        if write_async_frame(&mut stream, &response).await.is_err() {
            break;
        }
    }
}

#[cfg(unix)]
async fn timeout_read(stream: &mut tokio::net::UnixStream, buf: &mut [u8]) -> Result<(), String> {
    tokio::time::timeout(HANDSHAKE_TIMEOUT, stream.read_exact(buf))
        .await
        .map_err(|_| "broker 握手超时".to_string())?
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[cfg(unix)]
async fn timeout_read_long(
    stream: &mut tokio::net::UnixStream,
    buf: &mut [u8],
) -> Result<(), String> {
    tokio::time::timeout(REQUEST_TIMEOUT, stream.read_exact(buf))
        .await
        .map_err(|_| "broker 读取超时".to_string())?
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[cfg(unix)]
async fn write_async_frame(
    stream: &mut tokio::net::UnixStream,
    value: &Value,
) -> Result<(), String> {
    let payload = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    stream
        .write_all(&(payload.len() as u32).to_le_bytes())
        .await
        .map_err(|error| error.to_string())?;
    stream
        .write_all(&payload)
        .await
        .map_err(|error| error.to_string())?;
    stream.flush().await.map_err(|error| error.to_string())
}

#[cfg(unix)]
fn peer_pid(stream: &std::os::unix::net::UnixStream) -> Option<u32> {
    use std::os::unix::io::AsRawFd;
    let fd = stream.as_raw_fd();
    #[cfg(target_os = "macos")]
    {
        let mut pid: libc::pid_t = 0;
        let mut len = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
        let rc = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_LOCAL,
                libc::LOCAL_PEERPID,
                &mut pid as *mut _ as *mut libc::c_void,
                &mut len,
            )
        };
        if rc == 0 && pid > 0 {
            return Some(pid as u32);
        }
        None
    }
    #[cfg(target_os = "linux")]
    {
        let mut cred = libc::ucred {
            pid: 0,
            uid: 0,
            gid: 0,
        };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        let rc = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                &mut cred as *mut _ as *mut libc::c_void,
                &mut len,
            )
        };
        if rc == 0 && cred.pid > 0 {
            return Some(cred.pid as u32);
        }
        None
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = fd;
        None
    }
}

pub async fn start_broker(
    secrets: BrokerLaunchSecrets,
    child_pid: u32,
) -> Result<BrokerEndpoint, String> {
    stop_broker().await;
    let socket_path = default_socket_path()?;
    if socket_path.exists() {
        let _ = std::fs::remove_file(&socket_path);
    }
    #[cfg(unix)]
    {
        use std::os::unix::net::UnixListener as StdUnixListener;
        let listener = StdUnixListener::bind(&socket_path)
            .map_err(|error| format!("绑定 credential broker socket 失败: {error}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600));
        }
        listener
            .set_nonblocking(true)
            .map_err(|error| format!("设置 broker socket 非阻塞失败: {error}"))?;
        let tokio_listener = tokio::net::UnixListener::from_std(listener)
            .map_err(|error| format!("接管 broker socket 失败: {error}"))?;
        let inner = Arc::new(TokioMutex::new(BrokerInner {
            listen_path: socket_path.clone(),
            pending_nonce: Some(secrets.nonce),
            pending_key: Some(secrets.session_key),
            pending_child_pid: Some(child_pid),
            session: None,
            stop: AtomicBool::new(false),
            generation: AtomicU64::new(1),
        }));
        {
            let mut slot = BROKER
                .lock()
                .map_err(|_| "获取 broker 锁失败".to_string())?;
            *slot = Some(inner.clone());
        }
        tauri::async_runtime::spawn(async move {
            loop {
                if inner.lock().await.stop.load(Ordering::SeqCst) {
                    break;
                }
                match tokio_listener.accept().await {
                    Ok((stream, _)) => {
                        let peer = {
                            let std_stream = stream.into_std().ok();
                            let pid = std_stream.as_ref().and_then(peer_pid);
                            match std_stream {
                                Some(std_stream) => {
                                    let _ = std_stream.set_nonblocking(true);
                                    match tokio::net::UnixStream::from_std(std_stream) {
                                        Ok(stream) => Some((stream, pid)),
                                        Err(_) => None,
                                    }
                                }
                                None => None,
                            }
                        };
                        if let Some((stream, pid)) = peer {
                            let inner = inner.clone();
                            tauri::async_runtime::spawn(async move {
                                serve_unix_connection(inner, stream, pid).await;
                            });
                        }
                    }
                    Err(_) => {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                }
            }
            let _ = std::fs::remove_file(&inner.lock().await.listen_path);
        });
        return Ok(BrokerEndpoint { socket_path });
    }
    #[cfg(windows)]
    {
        let _ = child_pid;
        let pipe_name = default_pipe_name();
        let inner = Arc::new(TokioMutex::new(BrokerInner {
            listen_path: socket_path.clone(),
            pipe_name: pipe_name.clone(),
            pending_nonce: Some(secrets.nonce),
            pending_key: Some(secrets.session_key),
            pending_child_pid: Some(child_pid),
            session: None,
            stop: AtomicBool::new(false),
            generation: AtomicU64::new(1),
        }));
        {
            let mut slot = BROKER
                .lock()
                .map_err(|_| "获取 broker 锁失败".to_string())?;
            *slot = Some(inner.clone());
        }
        tauri::async_runtime::spawn(async move {
            serve_windows_pipe(inner).await;
        });
        return Ok(BrokerEndpoint {
            socket_path,
            pipe_name,
        });
    }
}

#[cfg(windows)]
async fn serve_windows_pipe(inner: Arc<TokioMutex<BrokerInner>>) {
    use tokio::net::windows::named_pipe::ServerOptions;
    loop {
        if inner.lock().await.stop.load(Ordering::SeqCst) {
            break;
        }
        let pipe_name = inner.lock().await.pipe_name.clone();
        let server = match ServerOptions::new()
            .first_pipe_instance(true)
            .create(&pipe_name)
        {
            Ok(server) => server,
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(200)).await;
                continue;
            }
        };
        if server.connect().await.is_err() {
            continue;
        }
        // Windows named-pipe ACL is the current user by default for this process.
        let mut stream = server;
        let mut len_buf = [0u8; 4];
        if tokio::time::timeout(HANDSHAKE_TIMEOUT, stream.read_exact(&mut len_buf))
            .await
            .is_err()
        {
            continue;
        }
        let Ok(()) = stream.read_exact(&mut len_buf).await else {
            continue;
        };
        let len = u32::from_le_bytes(len_buf) as usize;
        if len == 0 || len > MAX_FRAME_BYTES {
            continue;
        }
        let mut payload = vec![0u8; len];
        if stream.read_exact(&mut payload).await.is_err() {
            continue;
        }
        let Ok(hello) = serde_json::from_slice::<BrokerHello>(&payload) else {
            continue;
        };
        let mut guard = inner.lock().await;
        let Some(expected_nonce) = guard.pending_nonce.take() else {
            continue;
        };
        let Some(session_key) = guard.pending_key.take() else {
            continue;
        };
        let nonce = decode_hex(&hello.nonce).unwrap_or_default();
        if nonce.as_slice() != expected_nonce.as_slice()
            || !verify_payload(
                &session_key,
                &hello_payload(hello.protocol_version, hello.child_pid, &hello.nonce),
                &hello.hmac,
            )
        {
            continue;
        }
        guard.session = Some(BrokerSession {
            child_pid: hello.child_pid,
            session_key,
            expected_seq: 1,
            last_seen: Instant::now(),
        });
        drop(guard);
        let ok = serde_json::to_vec(&json!({"type":"hello_ok"})).unwrap_or_default();
        let _ = stream.write_all(&(ok.len() as u32).to_le_bytes()).await;
        let _ = stream.write_all(&ok).await;
        loop {
            let mut len_buf = [0u8; 4];
            if tokio::time::timeout(REQUEST_TIMEOUT, stream.read_exact(&mut len_buf))
                .await
                .is_err()
            {
                break;
            }
            if stream.read_exact(&mut len_buf).await.is_err() {
                break;
            }
            let len = u32::from_le_bytes(len_buf) as usize;
            if len == 0 || len > MAX_FRAME_BYTES {
                break;
            }
            let mut payload = vec![0u8; len];
            if stream.read_exact(&mut payload).await.is_err() {
                break;
            }
            let Ok(request) = serde_json::from_slice::<BrokerRequest>(&payload) else {
                break;
            };
            {
                let mut guard = inner.lock().await;
                let Some(session) = guard.session.as_mut() else {
                    break;
                };
                if verify_request(session, &request).is_err() {
                    guard.session = None;
                    break;
                }
                session.expected_seq += 1;
                session.last_seen = Instant::now();
            }
            let response = process_request(request).await;
            let payload = serde_json::to_vec(&response).unwrap_or_default();
            if stream
                .write_all(&(payload.len() as u32).to_le_bytes())
                .await
                .is_err()
                || stream.write_all(&payload).await.is_err()
            {
                break;
            }
        }
    }
}

pub async fn stop_broker() {
    let inner = {
        let mut slot = match BROKER.lock() {
            Ok(slot) => slot,
            Err(_) => return,
        };
        slot.take()
    };
    if let Some(inner) = inner {
        let mut guard = inner.lock().await;
        guard.stop.store(true, Ordering::SeqCst);
        guard.session = None;
        guard.pending_key = None;
        guard.pending_nonce = None;
        let path = guard.listen_path.clone();
        drop(guard);
        let _ = std::fs::remove_file(path);
    }
}

pub fn is_broker_configured() -> bool {
    BROKER.lock().ok().is_some_and(|slot| slot.is_some())
}

pub async fn set_expected_child_pid(child_pid: u32) {
    let inner = {
        let slot = match BROKER.lock() {
            Ok(slot) => slot,
            Err(_) => return,
        };
        slot.clone()
    };
    if let Some(inner) = inner {
        inner.lock().await.pending_child_pid = Some(child_pid);
    }
}

pub fn hmac_for_request(session_key: &[u8], seq: u64, kind: &str, body: &str) -> String {
    sign_payload(session_key, &request_payload(seq, kind, body))
}

pub fn hmac_for_hello(
    session_key: &[u8],
    protocol_version: u32,
    child_pid: u32,
    nonce_hex: &str,
) -> String {
    sign_payload(
        session_key,
        &hello_payload(protocol_version, child_pid, nonce_hex),
    )
}

/// In-process broker used by unit tests. Does not bind a socket.
pub async fn get_grok_access_token_for_tests(account_id: &str) -> Result<Value, String> {
    handle_get_grok_token(account_id).await
}

pub fn write_frame_for_tests(payload: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    write_frame(&mut out, payload)?;
    Ok(out)
}

pub fn read_frame_for_tests(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut cursor = std::io::Cursor::new(bytes);
    read_frame(&mut cursor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handshake_round_trip_preserves_secrets() {
        let secrets = BrokerLaunchSecrets::generate();
        let encoded = secrets.encode_handshake();
        let decoded = BrokerLaunchSecrets::decode_handshake(&encoded).unwrap();
        assert_eq!(secrets.session_key, decoded.session_key);
        assert_eq!(secrets.nonce, decoded.nonce);
    }

    #[test]
    fn hmac_rejects_tampered_payload() {
        let secrets = BrokerLaunchSecrets::generate();
        let sig = hmac_for_request(&secrets.session_key, 1, "get_grok_access_token", "acc-1");
        assert!(verify_payload(
            &secrets.session_key,
            &request_payload(1, "get_grok_access_token", "acc-1"),
            &sig
        ));
        assert!(!verify_payload(
            &secrets.session_key,
            &request_payload(2, "get_grok_access_token", "acc-1"),
            &sig
        ));
    }

    #[test]
    fn redacts_capability_path_and_tokens() {
        let raw =
            "Authorization Bearer secret http://127.0.0.1:1457/_cockpit-ugw/abc123/v1/responses";
        let redacted = redact_text(raw);
        assert!(!redacted.contains("secret"));
        assert!(!redacted.contains("abc123"));
        assert!(redacted.contains("[capability]"));
    }

    #[test]
    fn frame_round_trip() {
        let framed = write_frame_for_tests(b"{\"ok\":true}").unwrap();
        assert_eq!(read_frame_for_tests(&framed).unwrap(), b"{\"ok\":true}");
    }
}
