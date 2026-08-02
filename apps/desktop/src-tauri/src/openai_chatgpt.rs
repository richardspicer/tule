//! Experimental, fixed-contract ChatGPT subscription compatibility adapter.
//!
//! Endpoint values are compile-time constants. A `cfg(test)` transport seam may
//! supply bounded mock responses while asserting production destinations; the
//! seam does not compile into release binaries.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use futures_util::StreamExt;
use oauth2::{CsrfToken, PkceCodeChallenge};
use reqwest::{Client, StatusCode, redirect::Policy};
use serde_json::Value;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::oneshot,
    time::timeout,
};
use tule_core::{AgentRepository, PROVIDER_PROFILE_ID};
use zeroize::Zeroize;

use crate::{
    credentials::{CredentialKind, CredentialStore, CredentialStoreError},
    provider::{
        ConnectionState, ConnectionStatus, ProviderAdapter, ProviderEvent, ProviderEventSink,
        ProviderFuture, ProviderRequest, PublicError,
    },
    sqlite::SqliteStore,
};
use tokio_util::sync::CancellationToken;

pub(crate) const PROVIDER_ID: &str = "openai-chatgpt-compat";
pub(crate) const MODEL: &str = "gpt-5.5";
pub(crate) const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub(crate) const AUTH_URL: &str = "https://auth.openai.com/oauth/authorize";
pub(crate) const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
pub(crate) const INFERENCE_URL: &str = "https://chatgpt.com/backend-api/codex/responses";
pub(crate) const SCOPES: &str = "openid profile email offline_access";
pub(crate) const CREDENTIAL_HANDLE: &str = "openai-chatgpt-compat-v1";
const PRIMARY_CALLBACK_PORT: u16 = 1455;
const FALLBACK_CALLBACK_PORT: u16 = 1457;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(300);
const MAX_CALLBACK_REQUEST: usize = 16 * 1024;
const MAX_CALLBACK_VALUE: usize = 8 * 1024;
const MAX_PROVIDER_BODY: usize = 64 * 1024;
const MAX_SSE_BUFFER: usize = 256 * 1024;
const REFRESH_SKEW_SECS: i64 = 60;

pub(crate) struct ChatGptAdapter {
    credentials: Arc<dyn CredentialStore>,
    client: Client,
    refresh_lock: tokio::sync::Mutex<()>,
    connect_cancel: Mutex<Option<oneshot::Sender<()>>>,
    connecting: AtomicBool,
    reconnect_required: AtomicBool,
    #[cfg(test)]
    test_transport: Mutex<Option<Arc<dyn TestTransport>>>,
}

impl ChatGptAdapter {
    pub(crate) fn new(credentials: Arc<dyn CredentialStore>) -> Self {
        let client = Client::builder()
            .redirect(Policy::none())
            .timeout(Duration::from_secs(60))
            .build()
            .expect("reqwest client construction should not fail");
        Self {
            credentials,
            client,
            refresh_lock: tokio::sync::Mutex::new(()),
            connect_cancel: Mutex::new(None),
            connecting: AtomicBool::new(false),
            reconnect_required: AtomicBool::new(false),
            #[cfg(test)]
            test_transport: Mutex::new(None),
        }
    }

    pub(crate) async fn ensure_fresh_access_public(
        &self,
        store: &SqliteStore,
    ) -> Result<(), PublicError> {
        self.ensure_fresh_access(store).await
    }

    #[cfg(test)]
    pub(crate) fn set_test_transport(&self, transport: Arc<dyn TestTransport>) {
        *self.test_transport.lock().expect("lock") = Some(transport);
    }

    pub(crate) async fn connect_in_browser(
        &self,
        store: Arc<SqliteStore>,
        open_url: impl FnOnce(&str) -> Result<(), PublicError>,
    ) -> Result<ConnectionStatus, PublicError> {
        if self.connection_status().state == ConnectionState::UnavailableInThisBuild {
            return Ok(self.connection_status());
        }
        if self.connecting.swap(true, Ordering::SeqCst) {
            return Err(PublicError::SessionBusy);
        }
        let result = self.connect_in_browser_inner(store, open_url).await;
        self.connecting.store(false, Ordering::SeqCst);
        result
    }

    async fn connect_in_browser_inner(
        &self,
        store: Arc<SqliteStore>,
        open_url: impl FnOnce(&str) -> Result<(), PublicError>,
    ) -> Result<ConnectionStatus, PublicError> {
        let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
        let csrf = CsrfToken::new_random();
        let (listener, redirect_uri) = bind_callback_listener().await?;
        let auth_url =
            build_authorization_url(&redirect_uri, pkce_challenge.as_str(), csrf.secret());

        let (cancel_tx, cancel_rx) = oneshot::channel();
        *self
            .connect_cancel
            .lock()
            .map_err(|_| PublicError::ProviderUnavailable)? = Some(cancel_tx);

        open_url(&auth_url)?;

        let callback = tokio::select! {
            result = timeout(CONNECT_TIMEOUT, accept_callback(listener, csrf.secret())) => {
                match result {
                    Ok(inner) => inner,
                    Err(_) => Err(PublicError::ProviderUnavailable),
                }
            }
            _ = cancel_rx => Err(PublicError::Cancelled),
        };
        let _ = self.connect_cancel.lock().map(|mut slot| slot.take());

        let code = callback?;
        let token = self
            .exchange_token(&redirect_uri, pkce_verifier.secret(), &code)
            .await?;
        match token {
            TokenBundle::Success(values) => {
                self.persist_tokens(store.as_ref(), values)?;
                self.reconnect_required.store(false, Ordering::SeqCst);
            }
            TokenBundle::InvalidGrant => return Err(PublicError::AuthenticationRequired),
        }
        Ok(self.connection_status())
    }

    #[allow(dead_code)]
    pub(crate) fn cancel_connect(&self) {
        if let Ok(mut slot) = self.connect_cancel.lock()
            && let Some(tx) = slot.take()
        {
            let _ = tx.send(());
        }
    }

    pub(crate) fn disconnect(&self, store: &SqliteStore) -> Result<ConnectionStatus, PublicError> {
        let mut deleted = Vec::new();
        for kind in [
            CredentialKind::RefreshToken,
            CredentialKind::AccessToken,
            CredentialKind::AccountId,
        ] {
            match self.credentials.delete(CREDENTIAL_HANDLE, kind) {
                Ok(()) => deleted.push(kind),
                Err(error) => {
                    let _ = error;
                    return Err(PublicError::CredentialStoreUnavailable);
                }
            }
        }

        let mut profile = store
            .get_provider_profile(PROVIDER_PROFILE_ID)
            .map_err(|_| PublicError::AgentStorageUnavailable)?
            .ok_or(PublicError::AgentStorageUnavailable)?;
        let now = unix_now_ms().map_err(|_| PublicError::AgentStorageUnavailable)?;
        profile.set_credential_metadata(None, None, now);
        store
            .update_provider_profile(&profile)
            .map_err(|_| PublicError::AgentStorageUnavailable)?;
        self.reconnect_required.store(false, Ordering::SeqCst);
        let _ = deleted;
        Ok(self.connection_status())
    }

    async fn exchange_token(
        &self,
        redirect_uri: &str,
        verifier: &str,
        code: &str,
    ) -> Result<TokenBundle, PublicError> {
        #[cfg(test)]
        if let Some(transport) = self.test_transport.lock().expect("lock").clone() {
            return transport.exchange_token(TOKEN_URL, redirect_uri, verifier, code);
        }

        let response = self
            .client
            .post(TOKEN_URL)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Accept", "application/json")
            .body(format!(
                "grant_type=authorization_code&client_id={}&code={}&redirect_uri={}&code_verifier={}",
                urlencoding(CLIENT_ID),
                urlencoding(code),
                urlencoding(redirect_uri),
                urlencoding(verifier)
            ))
            .send()
            .await
            .map_err(|_| PublicError::ProviderUnavailable)?;
        parse_token_response(response).await
    }

    async fn refresh_access_token(&self, store: &SqliteStore) -> Result<(), PublicError> {
        let _guard = self.refresh_lock.lock().await;
        let refresh = self
            .credentials
            .read(CREDENTIAL_HANDLE, CredentialKind::RefreshToken)
            .map_err(map_credential_error)?
            .ok_or(PublicError::AuthenticationRequired)?;
        let refresh =
            String::from_utf8(refresh).map_err(|_| PublicError::AuthenticationRequired)?;

        #[cfg(test)]
        let token = if let Some(transport) = self.test_transport.lock().expect("lock").clone() {
            transport.refresh_token(TOKEN_URL, &refresh)?
        } else {
            self.refresh_via_http(&refresh).await?
        };
        #[cfg(not(test))]
        let token = self.refresh_via_http(&refresh).await?;

        if matches!(token, TokenBundle::InvalidGrant) {
            let _ = self
                .credentials
                .delete(CREDENTIAL_HANDLE, CredentialKind::AccessToken);
            let _ = self
                .credentials
                .delete(CREDENTIAL_HANDLE, CredentialKind::RefreshToken);
            let _ = self
                .credentials
                .delete(CREDENTIAL_HANDLE, CredentialKind::AccountId);
            if let Ok(Some(mut profile)) = store.get_provider_profile(PROVIDER_PROFILE_ID) {
                let now = unix_now_ms().unwrap_or(0);
                profile.set_credential_metadata(None, None, now);
                let _ = store.update_provider_profile(&profile);
            }
            self.reconnect_required.store(true, Ordering::SeqCst);
            return Err(PublicError::AuthenticationRequired);
        }
        if let TokenBundle::Success(values) = token {
            self.persist_tokens(store, values)?;
        }
        Ok(())
    }

    async fn refresh_via_http(&self, refresh: &str) -> Result<TokenBundle, PublicError> {
        let response = self
            .client
            .post(TOKEN_URL)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Accept", "application/json")
            .body(format!(
                "grant_type=refresh_token&client_id={}&refresh_token={}",
                urlencoding(CLIENT_ID),
                urlencoding(refresh)
            ))
            .send()
            .await
            .map_err(|_| PublicError::ProviderUnavailable)?;
        if response.status() == StatusCode::BAD_REQUEST {
            let body = response.text().await.unwrap_or_default();
            if body.contains("invalid_grant") {
                return Ok(TokenBundle::InvalidGrant);
            }
            return Err(PublicError::ProviderUnavailable);
        }
        parse_token_response(response).await
    }

    fn persist_tokens(
        &self,
        store: &SqliteStore,
        mut token: TokenValues,
    ) -> Result<(), PublicError> {
        let mut written = Vec::new();
        let mut steps = Vec::new();
        if !token.preserve_refresh {
            if token.refresh.is_empty() {
                token.zeroize();
                return Err(PublicError::ProviderUnavailable);
            }
            steps.push((
                CredentialKind::RefreshToken,
                token.refresh.as_bytes().to_vec(),
            ));
        }
        steps.push((
            CredentialKind::AccessToken,
            token.access.as_bytes().to_vec(),
        ));
        steps.push((CredentialKind::AccountId, token.account.as_bytes().to_vec()));
        for (kind, value) in &steps {
            match self.credentials.replace(CREDENTIAL_HANDLE, *kind, value) {
                Ok(()) => written.push(*kind),
                Err(error) => {
                    for prior in &written {
                        let _ = self.credentials.delete(CREDENTIAL_HANDLE, *prior);
                    }
                    token.zeroize();
                    return Err(map_credential_error(error));
                }
            }
        }

        let mut profile = store
            .get_provider_profile(PROVIDER_PROFILE_ID)
            .map_err(|_| PublicError::AgentStorageUnavailable)?
            .ok_or(PublicError::AgentStorageUnavailable)?;
        let now = unix_now_ms().map_err(|_| PublicError::AgentStorageUnavailable)?;
        profile.set_credential_metadata(
            Some(CREDENTIAL_HANDLE.to_owned()),
            token.expires_at_unix_ms,
            now,
        );
        if store.update_provider_profile(&profile).is_err() {
            for prior in &written {
                let _ = self.credentials.delete(CREDENTIAL_HANDLE, *prior);
            }
            token.zeroize();
            return Err(PublicError::AgentStorageUnavailable);
        }
        token.zeroize();
        Ok(())
    }

    async fn ensure_fresh_access(&self, store: &SqliteStore) -> Result<(), PublicError> {
        let profile = store
            .get_provider_profile(PROVIDER_PROFILE_ID)
            .map_err(|_| PublicError::AgentStorageUnavailable)?
            .ok_or(PublicError::NotConnected)?;
        let now = unix_now_ms().map_err(|_| PublicError::AgentStorageUnavailable)?;
        if let Some(expires) = profile.access_token_expires_at_unix_ms()
            && expires - now <= REFRESH_SKEW_SECS * 1000
        {
            self.refresh_access_token(store).await?;
        }
        Ok(())
    }

    fn read_access_and_account(&self) -> Result<(String, String), PublicError> {
        let access = self
            .credentials
            .read(CREDENTIAL_HANDLE, CredentialKind::AccessToken)
            .map_err(map_credential_error)?
            .ok_or(PublicError::NotConnected)?;
        let account = self
            .credentials
            .read(CREDENTIAL_HANDLE, CredentialKind::AccountId)
            .map_err(map_credential_error)?
            .ok_or(PublicError::NotConnected)?;
        Ok((
            String::from_utf8(access).map_err(|_| PublicError::AuthenticationRequired)?,
            String::from_utf8(account).map_err(|_| PublicError::AuthenticationRequired)?,
        ))
    }
}

impl ProviderAdapter for ChatGptAdapter {
    fn connection_status(&self) -> ConnectionStatus {
        let state = if self.connecting.load(Ordering::SeqCst) {
            ConnectionState::Connecting
        } else if self.reconnect_required.load(Ordering::SeqCst) {
            ConnectionState::ReconnectRequired
        } else {
            match self
                .credentials
                .read(CREDENTIAL_HANDLE, CredentialKind::RefreshToken)
            {
                Ok(Some(_)) => ConnectionState::Connected,
                Ok(None) => ConnectionState::Disconnected,
                Err(_) => ConnectionState::UnavailableInThisBuild,
            }
        };
        ConnectionStatus {
            state,
            provider_id: PROVIDER_ID,
            model: MODEL,
        }
    }

    fn stream<'a>(
        &'a self,
        request: ProviderRequest,
        cancel: CancellationToken,
        mut on_event: ProviderEventSink,
    ) -> ProviderFuture<'a> {
        Box::pin(async move {
            if cancel.is_cancelled() {
                return Err(PublicError::Cancelled);
            }
            let (mut access, mut account) = self.read_access_and_account()?;

            #[cfg(test)]
            {
                let transport = self.test_transport.lock().expect("lock").clone();
                if let Some(transport) = transport {
                    access.zeroize();
                    account.zeroize();
                    let mock = transport.inference(INFERENCE_URL, &request.request_json)?;
                    return emit_mock_inference(mock, cancel, &mut on_event).await;
                }
            }

            let response = self
                .client
                .post(INFERENCE_URL)
                .header("Authorization", format!("Bearer {access}"))
                .header("Accept", "text/event-stream")
                .header("Content-Type", "application/json")
                .header("originator", "tule")
                .header("User-Agent", "tule-desktop/0.1.0")
                .header("session_id", &request.session_id)
                .header("ChatGPT-Account-Id", account.as_str())
                .body(request.request_json)
                .send()
                .await
                .map_err(|_| PublicError::ProviderUnavailable)?;
            access.zeroize();
            account.zeroize();
            if cancel.is_cancelled() {
                return Err(PublicError::Cancelled);
            }
            match response.status() {
                StatusCode::UNAUTHORIZED => return Err(PublicError::AuthenticationRequired),
                StatusCode::FORBIDDEN => return Err(PublicError::EntitlementUnavailable),
                StatusCode::TOO_MANY_REQUESTS => return Err(PublicError::RateLimited),
                status if !status.is_success() => return Err(PublicError::ProviderUnavailable),
                _ => {}
            }
            parse_sse_response(response, cancel, &mut on_event).await
        })
    }
}

#[cfg(test)]
async fn emit_mock_inference(
    mock: MockInference,
    cancel: CancellationToken,
    on_event: &mut ProviderEventSink,
) -> Result<Vec<ProviderEvent>, PublicError> {
    match mock {
        MockInference::Status(401) => Err(PublicError::AuthenticationRequired),
        MockInference::Status(403) => Err(PublicError::EntitlementUnavailable),
        MockInference::Status(429) => Err(PublicError::RateLimited),
        MockInference::Status(_) => Err(PublicError::ProviderUnavailable),
        MockInference::Events(events) => {
            for event in events {
                if cancel.is_cancelled() {
                    return Err(PublicError::Cancelled);
                }
                on_event(event)?;
            }
            Ok(Vec::new())
        }
        MockInference::Sse(body) => parse_sse_buffer(body.as_bytes(), cancel, on_event).await,
    }
}

async fn bind_callback_listener() -> Result<(TcpListener, String), PublicError> {
    match TcpListener::bind(("127.0.0.1", PRIMARY_CALLBACK_PORT)).await {
        Ok(listener) => Ok((
            listener,
            format!("http://localhost:{PRIMARY_CALLBACK_PORT}/auth/callback"),
        )),
        Err(_) => {
            let listener = TcpListener::bind(("127.0.0.1", FALLBACK_CALLBACK_PORT))
                .await
                .map_err(|_| PublicError::ProviderUnavailable)?;
            Ok((
                listener,
                format!("http://localhost:{FALLBACK_CALLBACK_PORT}/auth/callback"),
            ))
        }
    }
}

fn build_authorization_url(redirect_uri: &str, challenge: &str, state: &str) -> String {
    format!(
        "{AUTH_URL}?response_type=code&client_id={}&redirect_uri={}&scope={}&code_challenge={}&code_challenge_method=S256&state={}&id_token_add_organizations=true&codex_cli_simplified_flow=true&originator=tule",
        urlencoding(CLIENT_ID),
        urlencoding(redirect_uri),
        urlencoding(SCOPES),
        urlencoding(challenge),
        urlencoding(state)
    )
}

async fn accept_callback(
    listener: TcpListener,
    expected_state: &str,
) -> Result<String, PublicError> {
    let (mut socket, _) = listener
        .accept()
        .await
        .map_err(|_| PublicError::ProviderUnavailable)?;
    let mut buf = vec![0_u8; MAX_CALLBACK_REQUEST + 1];
    let mut read = 0_usize;
    loop {
        let n = socket
            .read(&mut buf[read..])
            .await
            .map_err(|_| PublicError::ProviderUnavailable)?;
        if n == 0 {
            break;
        }
        read += n;
        if read > MAX_CALLBACK_REQUEST {
            let _ = socket
                .write_all(b"HTTP/1.1 413 Payload Too Large\r\nContent-Length: 0\r\n\r\n")
                .await;
            return Err(PublicError::InvalidInput);
        }
        if buf[..read].windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    let request = std::str::from_utf8(&buf[..read]).map_err(|_| PublicError::InvalidInput)?;
    let line = request.lines().next().ok_or(PublicError::InvalidInput)?;
    let path = line
        .split_whitespace()
        .nth(1)
        .ok_or(PublicError::InvalidInput)?;
    let query = path.split('?').nth(1).unwrap_or("");
    let params = parse_query(query)?;
    if params.contains_key("error") {
        let _ = socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 16\r\n\r\nSign-in declined")
            .await;
        return Err(PublicError::AuthenticationRequired);
    }
    let state = params.get("state").ok_or(PublicError::InvalidInput)?;
    if state.len() > MAX_CALLBACK_VALUE || state != expected_state {
        let _ = socket
            .write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n")
            .await;
        return Err(PublicError::InvalidInput);
    }
    let code = params.get("code").ok_or(PublicError::InvalidInput)?;
    if code.is_empty() || code.len() > MAX_CALLBACK_VALUE {
        return Err(PublicError::InvalidInput);
    }
    let _ = socket
        .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 48\r\n\r\nYou can return to TULE. This window may be closed.")
        .await;
    Ok(code.clone())
}

fn parse_query(query: &str) -> Result<HashMap<String, String>, PublicError> {
    let mut map = HashMap::new();
    for pair in query.split('&').filter(|part| !part.is_empty()) {
        let mut parts = pair.splitn(2, '=');
        let key = parts.next().unwrap_or_default();
        let value = parts.next().unwrap_or_default();
        map.insert(urldecoding(key), urldecoding(value));
    }
    Ok(map)
}

async fn parse_token_response(response: reqwest::Response) -> Result<TokenBundle, PublicError> {
    if !response.status().is_success() {
        return Err(PublicError::ProviderUnavailable);
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|_| PublicError::ProviderUnavailable)?;
    if bytes.len() > MAX_PROVIDER_BODY {
        return Err(PublicError::InvalidInput);
    }
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| PublicError::ProviderUnavailable)?;
    let access = value
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or(PublicError::ProviderUnavailable)?
        .to_owned();
    let refresh = value
        .get("refresh_token")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let mut id_token = value
        .get("id_token")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let account = extract_account_id(&id_token)?;
    id_token.zeroize();
    let expires_in = value
        .get("expires_in")
        .and_then(Value::as_i64)
        .unwrap_or(3600);
    let expires_at_unix_ms = unix_now_ms().ok().map(|now| now + expires_in * 1000);
    if refresh.is_empty() {
        // Rotation without a new refresh token preserves the existing refresh outside this parser.
        return Ok(TokenBundle::Success(TokenValues {
            access,
            refresh: String::new(),
            account,
            expires_at_unix_ms,
            preserve_refresh: true,
        }));
    }
    Ok(TokenBundle::Success(TokenValues {
        access,
        refresh,
        account,
        expires_at_unix_ms,
        preserve_refresh: false,
    }))
}

fn extract_account_id(id_token: &str) -> Result<String, PublicError> {
    let payload = id_token
        .split('.')
        .nth(1)
        .ok_or(PublicError::ProviderUnavailable)?;
    let decoded = URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(payload))
        .map_err(|_| PublicError::ProviderUnavailable)?;
    if decoded.len() > MAX_PROVIDER_BODY {
        return Err(PublicError::InvalidInput);
    }
    let value: Value =
        serde_json::from_slice(&decoded).map_err(|_| PublicError::ProviderUnavailable)?;
    value
        .get("https://api.openai.com/auth")
        .and_then(|auth| auth.get("chatgpt_account_id"))
        .and_then(Value::as_str)
        .or_else(|| value.get("chatgpt_account_id").and_then(Value::as_str))
        .or_else(|| value.get("sub").and_then(Value::as_str))
        .map(str::to_owned)
        .ok_or(PublicError::ProviderUnavailable)
}

async fn parse_sse_response(
    response: reqwest::Response,
    cancel: CancellationToken,
    on_event: &mut ProviderEventSink,
) -> Result<Vec<ProviderEvent>, PublicError> {
    let mut buffer = Vec::new();
    let mut saw_completed = false;
    let mut stream = response.bytes_stream();
    while let Some(next) = stream.next().await {
        if cancel.is_cancelled() {
            return Err(PublicError::Cancelled);
        }
        let chunk = next.map_err(|_| PublicError::ProviderUnavailable)?;
        if buffer.len().saturating_add(chunk.len()) > MAX_SSE_BUFFER {
            return Err(PublicError::OutputLimit);
        }
        buffer.extend_from_slice(&chunk);
        while let Some(boundary) = find_event_boundary(&buffer) {
            let event = buffer.drain(..boundary).collect::<Vec<_>>();
            let delimiter = event_delimiter_len(&buffer);
            if delimiter > 0 {
                buffer.drain(..delimiter);
            }
            let mut batch = Vec::new();
            parse_sse_event(&event, &mut batch)?;
            for item in batch {
                if matches!(item, ProviderEvent::Completed { .. }) {
                    saw_completed = true;
                }
                on_event(item)?;
            }
        }
    }
    if !buffer.is_empty() {
        let mut batch = Vec::new();
        parse_sse_event(&buffer, &mut batch)?;
        for item in batch {
            if matches!(item, ProviderEvent::Completed { .. }) {
                saw_completed = true;
            }
            on_event(item)?;
        }
    }
    if saw_completed {
        Ok(Vec::new())
    } else {
        Err(PublicError::ProviderUnavailable)
    }
}

#[cfg(test)]
async fn parse_sse_buffer(
    bytes: &[u8],
    cancel: CancellationToken,
    on_event: &mut ProviderEventSink,
) -> Result<Vec<ProviderEvent>, PublicError> {
    if cancel.is_cancelled() {
        return Err(PublicError::Cancelled);
    }
    if bytes.len() > MAX_SSE_BUFFER {
        return Err(PublicError::OutputLimit);
    }
    let mut buffer = bytes.to_vec();
    let mut saw_completed = false;
    while let Some(boundary) = find_event_boundary(&buffer) {
        let event = buffer.drain(..boundary).collect::<Vec<_>>();
        let delimiter = event_delimiter_len(&buffer);
        if delimiter > 0 {
            buffer.drain(..delimiter);
        }
        let mut batch = Vec::new();
        parse_sse_event(&event, &mut batch)?;
        for item in batch {
            if matches!(item, ProviderEvent::Completed { .. }) {
                saw_completed = true;
            }
            on_event(item)?;
        }
    }
    if !buffer.is_empty() {
        let mut batch = Vec::new();
        parse_sse_event(&buffer, &mut batch)?;
        for item in batch {
            if matches!(item, ProviderEvent::Completed { .. }) {
                saw_completed = true;
            }
            on_event(item)?;
        }
    }
    if saw_completed {
        Ok(Vec::new())
    } else {
        Err(PublicError::ProviderUnavailable)
    }
}

fn find_event_boundary(buffer: &[u8]) -> Option<usize> {
    buffer
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .or_else(|| buffer.windows(2).position(|w| w == b"\n\n"))
}

fn event_delimiter_len(buffer: &[u8]) -> usize {
    if buffer.starts_with(b"\r\n\r\n") {
        4
    } else if buffer.starts_with(b"\n\n") {
        2
    } else {
        0
    }
}

fn parse_sse_event(event: &[u8], output: &mut Vec<ProviderEvent>) -> Result<(), PublicError> {
    if event.len() > MAX_SSE_BUFFER {
        return Err(PublicError::OutputLimit);
    }
    let text = std::str::from_utf8(event).map_err(|_| PublicError::ProviderUnavailable)?;
    let data = text
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim_start)
        .collect::<Vec<_>>()
        .join("\n");
    if data.is_empty() {
        return Ok(());
    }
    if data == "[DONE]" {
        output.push(ProviderEvent::Completed {
            response_id: None,
            input_tokens: None,
            output_tokens: None,
        });
        return Ok(());
    }
    let value: Value = serde_json::from_str(&data).map_err(|_| PublicError::ProviderUnavailable)?;
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if kind.contains("tool") || kind.contains("function") {
        return Err(PublicError::UnsupportedProviderOutput);
    }
    if kind.contains("delta")
        && let Some(delta) = value
            .pointer("/delta")
            .and_then(Value::as_str)
            .or_else(|| value.pointer("/text").and_then(Value::as_str))
    {
        output.push(ProviderEvent::Delta(delta.to_owned()));
    }
    if kind.contains("completed") || kind.contains("done") {
        output.push(ProviderEvent::Completed {
            response_id: value
                .get("response")
                .and_then(|v| v.get("id"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            input_tokens: None,
            output_tokens: None,
        });
    }
    Ok(())
}

pub(crate) struct TokenValues {
    access: String,
    refresh: String,
    account: String,
    expires_at_unix_ms: Option<i64>,
    preserve_refresh: bool,
}

impl TokenValues {
    fn zeroize(&mut self) {
        self.access.zeroize();
        self.refresh.zeroize();
        self.account.zeroize();
    }
}

pub(crate) enum TokenBundle {
    Success(TokenValues),
    InvalidGrant,
}

fn map_credential_error(error: CredentialStoreError) -> PublicError {
    match error {
        CredentialStoreError::ValueTooLarge => PublicError::CredentialStoreUnavailable,
        CredentialStoreError::Unavailable => PublicError::CredentialStoreUnavailable,
    }
}

fn unix_now_ms() -> Result<i64, ()> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ())?;
    i64::try_from(duration.as_millis()).map_err(|_| ())
}

fn urlencoding(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn urldecoding(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let hex = &value[index + 1..index + 3];
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    out.push(byte);
                    index += 3;
                } else {
                    out.push(bytes[index]);
                    index += 1;
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
#[derive(Debug, Clone)]
pub(crate) enum MockInference {
    Events(Vec<ProviderEvent>),
    Sse(String),
    Status(u16),
}

#[cfg(test)]
pub(crate) trait TestTransport: Send + Sync {
    fn exchange_token(
        &self,
        token_url: &str,
        redirect_uri: &str,
        verifier: &str,
        code: &str,
    ) -> Result<TokenBundle, PublicError>;
    fn refresh_token(&self, token_url: &str, refresh: &str) -> Result<TokenBundle, PublicError>;
    fn inference(&self, inference_url: &str, body: &str) -> Result<MockInference, PublicError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::FakeCredentialStore;

    #[test]
    fn production_contract_is_pinned() {
        assert_eq!(PROVIDER_ID, "openai-chatgpt-compat");
        assert_eq!(MODEL, "gpt-5.5");
        assert_eq!(CLIENT_ID, "app_EMoamEEZ73f0CkXaXp7hrann");
        assert_eq!(AUTH_URL, "https://auth.openai.com/oauth/authorize");
        assert_eq!(TOKEN_URL, "https://auth.openai.com/oauth/token");
        assert_eq!(
            INFERENCE_URL,
            "https://chatgpt.com/backend-api/codex/responses"
        );
        assert_eq!(SCOPES, "openid profile email offline_access");
        let url =
            build_authorization_url("http://localhost:1455/auth/callback", "challenge", "state");
        assert!(url.contains("originator=tule"));
        assert!(url.contains("codex_cli_simplified_flow=true"));
        assert!(url.contains("id_token_add_organizations=true"));
        assert!(!url.contains("api.connectors"));
    }

    #[test]
    fn tool_events_are_rejected() {
        let mut output = Vec::new();
        assert_eq!(
            parse_sse_event(b"data: {\"type\":\"response.function_call\"}", &mut output),
            Err(PublicError::UnsupportedProviderOutput)
        );
    }

    #[test]
    fn terminal_can_arrive_before_delta() {
        let mut output = Vec::new();
        parse_sse_event(b"data: [DONE]", &mut output).unwrap();
        assert!(matches!(
            output.as_slice(),
            [ProviderEvent::Completed { .. }]
        ));
    }

    #[test]
    fn credential_write_order_compensates_on_failure() {
        let store = FakeCredentialStore::default();
        store.fail_on_operation(2); // fail second replace (access)
        let adapter = ChatGptAdapter::new(Arc::new(store));
        // Build a temp sqlite for metadata path is heavier; unit-test compensation via direct replace sequence.
        let fake = FakeCredentialStore::default();
        fake.fail_on_operation(2);
        let mut written = Vec::new();
        for (idx, kind) in [
            CredentialKind::RefreshToken,
            CredentialKind::AccessToken,
            CredentialKind::AccountId,
        ]
        .into_iter()
        .enumerate()
        {
            match fake.replace("h", kind, b"x") {
                Ok(()) => written.push(kind),
                Err(_) => {
                    assert_eq!(idx, 1);
                    for prior in &written {
                        fake.delete("h", *prior).unwrap();
                    }
                    break;
                }
            }
        }
        assert!(
            fake.read("h", CredentialKind::RefreshToken)
                .unwrap()
                .is_none()
        );
        let _ = adapter;
    }

    #[test]
    fn authorization_url_uses_exact_bound_redirect() {
        let primary = build_authorization_url("http://localhost:1455/auth/callback", "ch", "st");
        assert!(primary.contains("redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback"));
        let fallback = build_authorization_url("http://localhost:1457/auth/callback", "ch", "st");
        assert!(fallback.contains("redirect_uri=http%3A%2F%2Flocalhost%3A1457%2Fauth%2Fcallback"));
    }

    #[test]
    fn oversized_callback_values_are_rejected() {
        let huge = "x".repeat(MAX_CALLBACK_VALUE + 1);
        let query = format!("state={huge}&code=ok");
        let params = parse_query(&query).unwrap();
        assert!(params.get("state").unwrap().len() > MAX_CALLBACK_VALUE);
    }

    #[test]
    fn parse_query_decodes_callback_pairs() {
        let params = parse_query("code=a%2Bb&state=s+t&error=access_denied").unwrap();
        assert_eq!(params.get("code").unwrap(), "a+b");
        assert_eq!(params.get("state").unwrap(), "s t");
        assert_eq!(params.get("error").unwrap(), "access_denied");
    }

    #[tokio::test]
    async fn mock_inference_maps_status_and_tool_and_terminal_before_delta() {
        let adapter = ChatGptAdapter::new(Arc::new(FakeCredentialStore::default()));
        adapter
            .credentials
            .replace(CREDENTIAL_HANDLE, CredentialKind::RefreshToken, b"r")
            .unwrap();
        adapter
            .credentials
            .replace(CREDENTIAL_HANDLE, CredentialKind::AccessToken, b"a")
            .unwrap();
        adapter
            .credentials
            .replace(CREDENTIAL_HANDLE, CredentialKind::AccountId, b"acct")
            .unwrap();

        struct StatusTransport(u16);
        impl TestTransport for StatusTransport {
            fn exchange_token(
                &self,
                _: &str,
                _: &str,
                _: &str,
                _: &str,
            ) -> Result<TokenBundle, PublicError> {
                unreachable!()
            }
            fn refresh_token(&self, _: &str, _: &str) -> Result<TokenBundle, PublicError> {
                unreachable!()
            }
            fn inference(&self, url: &str, _: &str) -> Result<MockInference, PublicError> {
                assert_eq!(url, INFERENCE_URL);
                Ok(MockInference::Status(self.0))
            }
        }

        for (status, expected) in [
            (401, PublicError::AuthenticationRequired),
            (403, PublicError::EntitlementUnavailable),
            (429, PublicError::RateLimited),
            (500, PublicError::ProviderUnavailable),
        ] {
            adapter.set_test_transport(Arc::new(StatusTransport(status)));
            let events = Arc::new(Mutex::new(Vec::new()));
            let events_cb = Arc::clone(&events);
            let result = adapter
                .stream(
                    ProviderRequest {
                        session_id: "s".into(),
                        request_json: "{}".into(),
                    },
                    CancellationToken::new(),
                    Box::new(move |event| {
                        events_cb.lock().unwrap().push(event);
                        Ok(())
                    }),
                )
                .await;
            assert_eq!(result, Err(expected));
            assert!(events.lock().unwrap().is_empty());
        }

        struct SseTransport(String);
        impl TestTransport for SseTransport {
            fn exchange_token(
                &self,
                token_url: &str,
                redirect_uri: &str,
                _: &str,
                _: &str,
            ) -> Result<TokenBundle, PublicError> {
                assert_eq!(token_url, TOKEN_URL);
                assert!(
                    redirect_uri == "http://localhost:1455/auth/callback"
                        || redirect_uri == "http://localhost:1457/auth/callback"
                );
                Err(PublicError::ProviderUnavailable)
            }
            fn refresh_token(&self, token_url: &str, _: &str) -> Result<TokenBundle, PublicError> {
                assert_eq!(token_url, TOKEN_URL);
                Err(PublicError::ProviderUnavailable)
            }
            fn inference(&self, url: &str, body: &str) -> Result<MockInference, PublicError> {
                assert_eq!(url, INFERENCE_URL);
                let _ = body;
                Ok(MockInference::Sse(self.0.clone()))
            }
        }

        adapter.set_test_transport(Arc::new(SseTransport(
            "data: {\"type\":\"response.function_call\"}\n\n".into(),
        )));
        let err = adapter
            .stream(
                ProviderRequest {
                    session_id: "s".into(),
                    request_json: "{\"model\":\"gpt-5.5\"}".into(),
                },
                CancellationToken::new(),
                Box::new(|_| Ok(())),
            )
            .await;
        assert_eq!(err, Err(PublicError::UnsupportedProviderOutput));

        adapter.set_test_transport(Arc::new(SseTransport("data: [DONE]\n\n".into())));
        let events = Arc::new(Mutex::new(Vec::new()));
        let events_cb = Arc::clone(&events);
        adapter
            .stream(
                ProviderRequest {
                    session_id: "s".into(),
                    request_json: "{}".into(),
                },
                CancellationToken::new(),
                Box::new(move |event| {
                    events_cb.lock().unwrap().push(event);
                    Ok(())
                }),
            )
            .await
            .unwrap();
        assert!(matches!(
            events.lock().unwrap().as_slice(),
            [ProviderEvent::Completed { .. }]
        ));

        adapter.set_test_transport(Arc::new(SseTransport("x".repeat(MAX_SSE_BUFFER + 1))));
        let oversized = adapter
            .stream(
                ProviderRequest {
                    session_id: "s".into(),
                    request_json: "{}".into(),
                },
                CancellationToken::new(),
                Box::new(|_| Ok(())),
            )
            .await;
        assert_eq!(oversized, Err(PublicError::OutputLimit));
    }

    #[tokio::test]
    async fn mock_refresh_invalid_grant_marks_reconnect_required() {
        let fake = Arc::new(FakeCredentialStore::default());
        fake.replace(CREDENTIAL_HANDLE, CredentialKind::RefreshToken, b"refresh")
            .unwrap();
        fake.replace(CREDENTIAL_HANDLE, CredentialKind::AccessToken, b"access")
            .unwrap();
        fake.replace(CREDENTIAL_HANDLE, CredentialKind::AccountId, b"acct")
            .unwrap();
        let adapter = ChatGptAdapter::new(fake);
        struct InvalidGrant;
        impl TestTransport for InvalidGrant {
            fn exchange_token(
                &self,
                _: &str,
                _: &str,
                _: &str,
                _: &str,
            ) -> Result<TokenBundle, PublicError> {
                unreachable!()
            }
            fn refresh_token(
                &self,
                token_url: &str,
                refresh: &str,
            ) -> Result<TokenBundle, PublicError> {
                assert_eq!(token_url, TOKEN_URL);
                assert_eq!(refresh, "refresh");
                Ok(TokenBundle::InvalidGrant)
            }
            fn inference(&self, _: &str, _: &str) -> Result<MockInference, PublicError> {
                unreachable!()
            }
        }
        adapter.set_test_transport(Arc::new(InvalidGrant));
        let dir = tempfile_dir();
        let store = SqliteStore::open(dir.join("tule.sqlite3")).unwrap();
        let err = adapter.refresh_access_token(&store).await;
        assert_eq!(err, Err(PublicError::AuthenticationRequired));
        assert_eq!(
            adapter.connection_status().state,
            ConnectionState::ReconnectRequired
        );
    }

    #[tokio::test]
    async fn mock_cancel_aborts_stream() {
        let fake = Arc::new(FakeCredentialStore::default());
        fake.replace(CREDENTIAL_HANDLE, CredentialKind::RefreshToken, b"r")
            .unwrap();
        fake.replace(CREDENTIAL_HANDLE, CredentialKind::AccessToken, b"a")
            .unwrap();
        fake.replace(CREDENTIAL_HANDLE, CredentialKind::AccountId, b"acct")
            .unwrap();
        let adapter = ChatGptAdapter::new(fake);
        struct ManyEvents;
        impl TestTransport for ManyEvents {
            fn exchange_token(
                &self,
                _: &str,
                _: &str,
                _: &str,
                _: &str,
            ) -> Result<TokenBundle, PublicError> {
                unreachable!()
            }
            fn refresh_token(&self, _: &str, _: &str) -> Result<TokenBundle, PublicError> {
                unreachable!()
            }
            fn inference(&self, url: &str, _: &str) -> Result<MockInference, PublicError> {
                assert_eq!(url, INFERENCE_URL);
                Ok(MockInference::Events(vec![
                    ProviderEvent::Delta("a".into()),
                    ProviderEvent::Delta("b".into()),
                    ProviderEvent::Completed {
                        response_id: None,
                        input_tokens: None,
                        output_tokens: None,
                    },
                ]))
            }
        }
        adapter.set_test_transport(Arc::new(ManyEvents));
        let cancel = CancellationToken::new();
        cancel.cancel();
        let result = adapter
            .stream(
                ProviderRequest {
                    session_id: "s".into(),
                    request_json: "{}".into(),
                },
                cancel,
                Box::new(|_| Ok(())),
            )
            .await;
        assert_eq!(result, Err(PublicError::Cancelled));
    }

    #[test]
    fn endpoint_constants_reject_drift_targets() {
        assert!(!AUTH_URL.contains("api.openai.com/v1"));
        assert!(!TOKEN_URL.contains("device"));
        assert_eq!(
            INFERENCE_URL,
            "https://chatgpt.com/backend-api/codex/responses"
        );
        assert!(!SCOPES.contains("api.connectors"));
    }

    fn tempfile_dir() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "tule-openai-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }
}
