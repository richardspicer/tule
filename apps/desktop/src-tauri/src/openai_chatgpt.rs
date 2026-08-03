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
use reqwest::{
    Client, StatusCode,
    header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue, USER_AGENT},
    redirect::Policy,
};
use serde_json::Value;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    time::timeout,
};
use tule_core::{AgentRepository, PROVIDER_PROFILE_ID, ProviderProfile};
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
const ALL_CREDENTIAL_KINDS: [CredentialKind; 3] = [
    CredentialKind::RefreshToken,
    CredentialKind::AccessToken,
    CredentialKind::AccountId,
];
/// Keep the refresh credential until every other protected value is gone so a
/// partial Disconnect remains visibly retryable rather than falsely disconnected.
const DELETE_CREDENTIAL_ORDER: [CredentialKind; 3] = [
    CredentialKind::AccountId,
    CredentialKind::AccessToken,
    CredentialKind::RefreshToken,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokenPersistence {
    Initial,
    Rotation,
}

#[derive(Clone, Copy)]
enum PersistenceCancellation<'a> {
    None,
    Operation(&'a CancellationToken),
    BrowserConnect(&'a Arc<CancellationToken>),
}

impl PersistenceCancellation<'_> {
    fn is_cancelled(self) -> bool {
        match self {
            Self::None => false,
            Self::Operation(token) => token.is_cancelled(),
            Self::BrowserConnect(token) => token.is_cancelled(),
        }
    }
}

struct CredentialSnapshot {
    entries: Vec<(CredentialKind, Option<Vec<u8>>)>,
}

impl CredentialSnapshot {
    fn value(&self, kind: CredentialKind) -> Option<&[u8]> {
        self.entries
            .iter()
            .find_map(|(stored_kind, value)| (*stored_kind == kind).then_some(value.as_deref()))
            .flatten()
    }
}

impl Drop for CredentialSnapshot {
    fn drop(&mut self) {
        for (_, value) in &mut self.entries {
            if let Some(value) = value {
                value.zeroize();
            }
        }
    }
}

pub(crate) struct ChatGptAdapter {
    credentials: Arc<dyn CredentialStore>,
    client: Client,
    /// Serializes every credential-using operation for the single built-in profile.
    profile_lock: tokio::sync::Mutex<()>,
    connect_cancel: Mutex<Option<Arc<CancellationToken>>>,
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
            profile_lock: tokio::sync::Mutex::new(()),
            connect_cancel: Mutex::new(None),
            connecting: AtomicBool::new(false),
            reconnect_required: AtomicBool::new(false),
            #[cfg(test)]
            test_transport: Mutex::new(None),
        }
    }

    #[cfg(test)]
    pub(crate) async fn ensure_fresh_access_public(
        &self,
        store: &SqliteStore,
    ) -> Result<(), PublicError> {
        self.ensure_fresh_access(store, None).await
    }

    pub(crate) async fn ensure_fresh_access_cancellable_public(
        &self,
        store: &SqliteStore,
        cancel: CancellationToken,
    ) -> Result<(), PublicError> {
        self.ensure_fresh_access(store, Some(&cancel)).await
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
        if self.connection_status_with_store(store.as_ref()).state
            == ConnectionState::UnavailableInThisBuild
        {
            return Ok(self.connection_status_with_store(store.as_ref()));
        }
        if self.connecting.swap(true, Ordering::SeqCst) {
            return Err(PublicError::SessionBusy);
        }
        let cancel = Arc::new(CancellationToken::new());
        match self.connect_cancel.lock() {
            Ok(mut slot) if slot.is_none() => *slot = Some(cancel.clone()),
            Ok(_) | Err(_) => {
                self.connecting.store(false, Ordering::SeqCst);
                return Err(PublicError::SessionBusy);
            }
        }
        let operation = match self.profile_lock.try_lock() {
            Ok(operation) => operation,
            Err(_) => {
                let was_cancelled = cancel.is_cancelled();
                let _ = self.clear_connect_cancel(&cancel);
                self.connecting.store(false, Ordering::SeqCst);
                return Err(if was_cancelled {
                    PublicError::Cancelled
                } else {
                    PublicError::SessionBusy
                });
            }
        };
        let result = self
            .connect_in_browser_inner(Arc::clone(&store), open_url, &cancel)
            .await;
        drop(operation);
        let cleanup = self.clear_connect_cancel(&cancel);
        // Clear the transient connecting lifecycle before sampling terminal status.
        self.connecting.store(false, Ordering::SeqCst);
        match (result, cleanup) {
            (Ok(()), Ok(())) => Ok(self.connection_status_with_store(store.as_ref())),
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error),
        }
    }

    async fn connect_in_browser_inner(
        &self,
        store: Arc<SqliteStore>,
        open_url: impl FnOnce(&str) -> Result<(), PublicError>,
        cancel: &Arc<CancellationToken>,
    ) -> Result<(), PublicError> {
        let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
        let csrf = CsrfToken::new_random();
        let (listener, redirect_uri) = tokio::select! {
            _ = cancel.cancelled() => return Err(PublicError::Cancelled),
            listener = bind_callback_listener() => listener?,
        };
        let auth_url =
            build_authorization_url(&redirect_uri, pkce_challenge.as_str(), csrf.secret());

        if cancel.is_cancelled() {
            return Err(PublicError::Cancelled);
        }
        open_url(&auth_url)?;

        let callback = tokio::select! {
            result = timeout(CONNECT_TIMEOUT, accept_callback(listener, csrf.secret())) => {
                match result {
                    Ok(inner) => inner,
                    Err(_) => Err(PublicError::ProviderUnavailable),
                }
            }
            _ = cancel.cancelled() => Err(PublicError::Cancelled),
        };

        let code = callback?;
        let token = self
            .exchange_token(
                &redirect_uri,
                pkce_verifier.secret(),
                &code,
                cancel.as_ref(),
            )
            .await?;
        match token {
            TokenBundle::Success(values) => {
                self.persist_tokens(
                    store.as_ref(),
                    values,
                    TokenPersistence::Initial,
                    PersistenceCancellation::BrowserConnect(cancel),
                )?;
                self.reconnect_required.store(false, Ordering::SeqCst);
            }
            TokenBundle::InvalidGrant => return Err(PublicError::AuthenticationRequired),
        }
        Ok(())
    }

    pub(crate) fn cancel_connect(&self) -> Result<(), PublicError> {
        let slot = self
            .connect_cancel
            .lock()
            .map_err(|_| PublicError::ProviderUnavailable)?;
        let cancel = slot.as_ref().ok_or(PublicError::InvalidInput)?;
        if cancel.is_cancelled() {
            return Err(PublicError::InvalidInput);
        }
        cancel.cancel();
        Ok(())
    }

    fn clear_connect_cancel(&self, cancel: &Arc<CancellationToken>) -> Result<(), PublicError> {
        let mut slot = self
            .connect_cancel
            .lock()
            .map_err(|_| PublicError::ProviderUnavailable)?;
        if slot
            .as_ref()
            .is_some_and(|active| Arc::ptr_eq(active, cancel))
        {
            slot.take();
        }
        Ok(())
    }

    pub(crate) fn disconnect(&self, store: &SqliteStore) -> Result<ConnectionStatus, PublicError> {
        let _operation = self
            .profile_lock
            .try_lock()
            .map_err(|_| PublicError::SessionBusy)?;
        self.remove_credentials_and_metadata(store, true)?;
        self.reconnect_required.store(false, Ordering::SeqCst);
        Ok(self.connection_status_with_store(store))
    }

    async fn exchange_token(
        &self,
        redirect_uri: &str,
        verifier: &str,
        code: &str,
        cancel: &CancellationToken,
    ) -> Result<TokenBundle, PublicError> {
        #[cfg(test)]
        if let Some(transport) = self.test_transport.lock().expect("lock").clone() {
            let result = transport.exchange_token(TOKEN_URL, redirect_uri, verifier, code);
            return if cancel.is_cancelled() {
                Err(PublicError::Cancelled)
            } else {
                result
            };
        }

        let request = self
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
            .send();
        let response = tokio::select! {
            _ = cancel.cancelled() => return Err(PublicError::Cancelled),
            response = request => response.map_err(|_| PublicError::ProviderUnavailable)?,
        };
        tokio::select! {
            _ = cancel.cancelled() => Err(PublicError::Cancelled),
            result = parse_token_response(response, TokenPersistence::Initial) => result,
        }
    }

    async fn refresh_access_token_locked(
        &self,
        store: &SqliteStore,
        cancel: Option<&CancellationToken>,
    ) -> Result<(), PublicError> {
        if cancel.is_some_and(CancellationToken::is_cancelled) {
            return Err(PublicError::Cancelled);
        }
        let refresh = self
            .credentials
            .read(CREDENTIAL_HANDLE, CredentialKind::RefreshToken)
            .map_err(map_credential_error)?
            .ok_or(PublicError::AuthenticationRequired)?;
        let mut refresh =
            String::from_utf8(refresh).map_err(|_| PublicError::AuthenticationRequired)?;

        #[cfg(test)]
        let token_result =
            if let Some(transport) = self.test_transport.lock().expect("lock").clone() {
                let result = transport.refresh_token(TOKEN_URL, &refresh);
                if cancel.is_some_and(CancellationToken::is_cancelled) {
                    Err(PublicError::Cancelled)
                } else {
                    result
                }
            } else {
                self.refresh_via_http(&refresh, cancel).await
            };
        #[cfg(not(test))]
        let token_result = self.refresh_via_http(&refresh, cancel).await;
        refresh.zeroize();
        let token = token_result?;

        if matches!(token, TokenBundle::InvalidGrant) {
            self.reconnect_required.store(true, Ordering::SeqCst);
            self.remove_credentials_and_metadata(store, false)?;
            return Err(PublicError::AuthenticationRequired);
        }
        if let TokenBundle::Success(values) = token {
            self.persist_tokens(
                store,
                values,
                TokenPersistence::Rotation,
                cancel.map_or(
                    PersistenceCancellation::None,
                    PersistenceCancellation::Operation,
                ),
            )?;
        }
        Ok(())
    }

    async fn refresh_via_http(
        &self,
        refresh: &str,
        cancel: Option<&CancellationToken>,
    ) -> Result<TokenBundle, PublicError> {
        let request = self
            .client
            .post(TOKEN_URL)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Accept", "application/json")
            .body(format!(
                "grant_type=refresh_token&client_id={}&refresh_token={}",
                urlencoding(CLIENT_ID),
                urlencoding(refresh)
            ))
            .send();
        let response = match cancel {
            Some(cancel) => {
                tokio::select! {
                    _ = cancel.cancelled() => return Err(PublicError::Cancelled),
                    response = request => response.map_err(|_| PublicError::ProviderUnavailable)?,
                }
            }
            None => request
                .await
                .map_err(|_| PublicError::ProviderUnavailable)?,
        };
        if response.status() == StatusCode::BAD_REQUEST {
            let body = match cancel {
                Some(cancel) => {
                    tokio::select! {
                        _ = cancel.cancelled() => return Err(PublicError::Cancelled),
                        body = read_bounded_response_body(response, MAX_PROVIDER_BODY) => body?,
                    }
                }
                None => read_bounded_response_body(response, MAX_PROVIDER_BODY).await?,
            };
            let invalid_grant = serde_json::from_slice::<Value>(&body)
                .ok()
                .and_then(|value| {
                    value
                        .get("error")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .is_some_and(|error| error == "invalid_grant");
            if invalid_grant {
                return Ok(TokenBundle::InvalidGrant);
            }
            return Err(PublicError::ProviderUnavailable);
        }
        match cancel {
            Some(cancel) => {
                tokio::select! {
                    _ = cancel.cancelled() => Err(PublicError::Cancelled),
                    result = parse_token_response(response, TokenPersistence::Rotation) => result,
                }
            }
            None => parse_token_response(response, TokenPersistence::Rotation).await,
        }
    }

    fn persist_tokens(
        &self,
        store: &SqliteStore,
        mut token: TokenValues,
        persistence: TokenPersistence,
        cancellation: PersistenceCancellation<'_>,
    ) -> Result<(), PublicError> {
        if token.access.is_empty()
            || token.account.is_empty()
            || (persistence == TokenPersistence::Initial
                && (token.preserve_refresh || token.refresh.is_empty()))
        {
            token.zeroize();
            return Err(PublicError::ProviderUnavailable);
        }
        if cancellation.is_cancelled() {
            token.zeroize();
            return Err(PublicError::Cancelled);
        }

        let mut profile = store
            .get_provider_profile(PROVIDER_PROFILE_ID)
            .map_err(|_| PublicError::AgentStorageUnavailable)?
            .ok_or(PublicError::AgentStorageUnavailable)?;
        let now = unix_now_ms().map_err(|_| PublicError::AgentStorageUnavailable)?;
        let snapshot = self.credential_snapshot()?;
        if cancellation.is_cancelled() {
            token.zeroize();
            return Err(PublicError::Cancelled);
        }
        let original_profile = profile.clone();
        profile.set_credential_metadata(None, None, now);
        if store.update_provider_profile(&profile).is_err() {
            token.zeroize();
            return Err(PublicError::AgentStorageUnavailable);
        }
        let mut written = Vec::new();
        let mut steps = Vec::with_capacity(3);
        if !token.preserve_refresh {
            steps.push(CredentialKind::RefreshToken);
        }
        steps.push(CredentialKind::AccessToken);
        steps.push(CredentialKind::AccountId);
        for kind in steps {
            let value = match kind {
                CredentialKind::RefreshToken => token.refresh.as_bytes(),
                CredentialKind::AccessToken => token.access.as_bytes(),
                CredentialKind::AccountId => token.account.as_bytes(),
            };
            match self.credentials.replace(CREDENTIAL_HANDLE, kind, value) {
                Ok(()) => written.push(kind),
                Err(error) => {
                    let restored = self.restore_persistence_state(
                        store,
                        &snapshot,
                        &written,
                        &original_profile,
                    );
                    token.zeroize();
                    if !restored {
                        self.reconnect_required.store(true, Ordering::SeqCst);
                        return Err(PublicError::CredentialStoreUnavailable);
                    }
                    return Err(map_credential_error(error));
                }
            }
            if cancellation.is_cancelled() {
                let restored =
                    self.restore_persistence_state(store, &snapshot, &written, &original_profile);
                token.zeroize();
                if !restored {
                    self.reconnect_required.store(true, Ordering::SeqCst);
                    return Err(PublicError::CredentialStoreUnavailable);
                }
                return Err(PublicError::Cancelled);
            }
        }

        profile.set_credential_metadata(
            Some(CREDENTIAL_HANDLE.to_owned()),
            token.expires_at_unix_ms,
            now,
        );
        let mut connect_commit_guard = match cancellation {
            PersistenceCancellation::BrowserConnect(cancel) => {
                let guard = match self.connect_cancel.lock() {
                    Ok(guard) => guard,
                    Err(_) => {
                        let restored = self.restore_persistence_state(
                            store,
                            &snapshot,
                            &written,
                            &original_profile,
                        );
                        token.zeroize();
                        if !restored {
                            self.reconnect_required.store(true, Ordering::SeqCst);
                            return Err(PublicError::CredentialStoreUnavailable);
                        }
                        return Err(PublicError::ProviderUnavailable);
                    }
                };
                if cancel.is_cancelled()
                    || !guard
                        .as_ref()
                        .is_some_and(|active| Arc::ptr_eq(active, cancel))
                {
                    let restored = self.restore_persistence_state(
                        store,
                        &snapshot,
                        &written,
                        &original_profile,
                    );
                    token.zeroize();
                    if !restored {
                        self.reconnect_required.store(true, Ordering::SeqCst);
                        return Err(PublicError::CredentialStoreUnavailable);
                    }
                    return Err(PublicError::Cancelled);
                }
                Some(guard)
            }
            PersistenceCancellation::Operation(cancel) if cancel.is_cancelled() => {
                let restored =
                    self.restore_persistence_state(store, &snapshot, &written, &original_profile);
                token.zeroize();
                if !restored {
                    self.reconnect_required.store(true, Ordering::SeqCst);
                    return Err(PublicError::CredentialStoreUnavailable);
                }
                return Err(PublicError::Cancelled);
            }
            _ => None,
        };
        if store.update_provider_profile(&profile).is_err() {
            let restored =
                self.restore_persistence_state(store, &snapshot, &written, &original_profile);
            token.zeroize();
            if !restored {
                self.reconnect_required.store(true, Ordering::SeqCst);
                return Err(PublicError::CredentialStoreUnavailable);
            }
            return Err(PublicError::AgentStorageUnavailable);
        }
        if let Some(slot) = connect_commit_guard.as_mut() {
            slot.take();
        }
        token.zeroize();
        self.reconnect_required.store(false, Ordering::SeqCst);
        Ok(())
    }

    async fn ensure_fresh_access(
        &self,
        store: &SqliteStore,
        cancel: Option<&CancellationToken>,
    ) -> Result<(), PublicError> {
        let _operation = match cancel {
            Some(cancel) => {
                tokio::select! {
                    _ = cancel.cancelled() => return Err(PublicError::Cancelled),
                    operation = self.profile_lock.lock() => operation,
                }
            }
            None => self.profile_lock.lock().await,
        };
        if cancel.is_some_and(CancellationToken::is_cancelled) {
            return Err(PublicError::Cancelled);
        }
        let profile = store
            .get_provider_profile(PROVIDER_PROFILE_ID)
            .map_err(|_| PublicError::AgentStorageUnavailable)?
            .ok_or(PublicError::NotConnected)?;
        if profile.credential_handle() != Some(CREDENTIAL_HANDLE) {
            self.reconnect_required.store(true, Ordering::SeqCst);
            return Err(PublicError::AuthenticationRequired);
        }
        let now = unix_now_ms().map_err(|_| PublicError::AgentStorageUnavailable)?;
        if let Some(expires) = profile.access_token_expires_at_unix_ms()
            && expires - now <= REFRESH_SKEW_SECS * 1000
        {
            self.refresh_access_token_locked(store, cancel).await?;
        }
        Ok(())
    }

    fn credential_snapshot(&self) -> Result<CredentialSnapshot, PublicError> {
        let mut entries = Vec::with_capacity(ALL_CREDENTIAL_KINDS.len());
        for kind in ALL_CREDENTIAL_KINDS {
            let value = self
                .credentials
                .read(CREDENTIAL_HANDLE, kind)
                .map_err(map_credential_error)?;
            entries.push((kind, value));
        }
        Ok(CredentialSnapshot { entries })
    }

    fn restore_snapshot(&self, snapshot: &CredentialSnapshot, changed: &[CredentialKind]) -> bool {
        let mut restored = true;
        for kind in changed.iter().rev() {
            let result = match snapshot.value(*kind) {
                Some(value) => self.credentials.replace(CREDENTIAL_HANDLE, *kind, value),
                None => self.credentials.delete(CREDENTIAL_HANDLE, *kind),
            };
            restored &= result.is_ok();
        }
        restored
    }

    fn restore_persistence_state(
        &self,
        store: &SqliteStore,
        snapshot: &CredentialSnapshot,
        changed: &[CredentialKind],
        profile: &ProviderProfile,
    ) -> bool {
        if !self.restore_snapshot(snapshot, changed) {
            return false;
        }
        store.update_provider_profile(profile).is_ok()
    }

    fn remove_credentials_and_metadata(
        &self,
        store: &SqliteStore,
        restore_on_failure: bool,
    ) -> Result<(), PublicError> {
        let mut profile = store
            .get_provider_profile(PROVIDER_PROFILE_ID)
            .map_err(|_| PublicError::AgentStorageUnavailable)?
            .ok_or(PublicError::AgentStorageUnavailable)?;
        let now = unix_now_ms().map_err(|_| PublicError::AgentStorageUnavailable)?;
        let snapshot = self.credential_snapshot()?;
        let mut deleted = Vec::new();
        for kind in DELETE_CREDENTIAL_ORDER {
            if let Err(error) = self.credentials.delete(CREDENTIAL_HANDLE, kind) {
                if restore_on_failure && !self.restore_snapshot(&snapshot, &deleted) {
                    self.reconnect_required.store(true, Ordering::SeqCst);
                }
                return Err(map_credential_error(error));
            }
            deleted.push(kind);
            match self.credentials.read(CREDENTIAL_HANDLE, kind) {
                Ok(None) => {}
                Ok(Some(_)) | Err(_) => {
                    if restore_on_failure && !self.restore_snapshot(&snapshot, &deleted) {
                        self.reconnect_required.store(true, Ordering::SeqCst);
                    }
                    return Err(PublicError::CredentialStoreUnavailable);
                }
            }
        }

        profile.set_credential_metadata(None, None, now);
        if store.update_provider_profile(&profile).is_err() {
            if restore_on_failure && !self.restore_snapshot(&snapshot, &deleted) {
                self.reconnect_required.store(true, Ordering::SeqCst);
                return Err(PublicError::CredentialStoreUnavailable);
            }
            return Err(PublicError::AgentStorageUnavailable);
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

    pub(crate) fn connection_status_with_store(&self, store: &SqliteStore) -> ConnectionStatus {
        let commit_marker = store
            .get_provider_profile(PROVIDER_PROFILE_ID)
            .map_err(|_| ())
            .and_then(|profile| {
                profile
                    .map(|profile| match profile.credential_handle() {
                        Some(CREDENTIAL_HANDLE) => true,
                        None => false,
                        Some(_) => {
                            self.reconnect_required.store(true, Ordering::SeqCst);
                            false
                        }
                    })
                    .ok_or(())
            })
            .map(Some);
        self.connection_status_for_commit_marker(commit_marker)
    }

    fn connection_status_for_commit_marker(
        &self,
        commit_marker: Result<Option<bool>, ()>,
    ) -> ConnectionStatus {
        let state = if self.connecting.load(Ordering::SeqCst) {
            ConnectionState::Connecting
        } else if self.reconnect_required.load(Ordering::SeqCst) {
            ConnectionState::ReconnectRequired
        } else {
            let present = ALL_CREDENTIAL_KINDS
                .iter()
                .try_fold(0_usize, |present, kind| {
                    self.credentials
                        .read(CREDENTIAL_HANDLE, *kind)
                        .map(|value| present + usize::from(value.is_some()))
                        .map_err(|_| ())
                });
            match (present, commit_marker) {
                (Err(()), _) | (_, Err(())) => ConnectionState::UnavailableInThisBuild,
                (Ok(present), Ok(None)) if present == ALL_CREDENTIAL_KINDS.len() => {
                    ConnectionState::Connected
                }
                (Ok(0), Ok(None)) => ConnectionState::Disconnected,
                (Ok(present), Ok(Some(true))) if present == ALL_CREDENTIAL_KINDS.len() => {
                    ConnectionState::Connected
                }
                (Ok(0), Ok(Some(false))) => ConnectionState::Disconnected,
                _ => ConnectionState::ReconnectRequired,
            }
        };
        ConnectionStatus {
            state,
            provider_id: PROVIDER_ID,
            model: MODEL,
        }
    }
}

impl ProviderAdapter for ChatGptAdapter {
    fn connection_status(&self) -> ConnectionStatus {
        self.connection_status_for_commit_marker(Ok(None))
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
            let _operation = tokio::select! {
                _ = cancel.cancelled() => return Err(PublicError::Cancelled),
                operation = self.profile_lock.lock() => operation,
            };
            let (mut access, mut account) = self.read_access_and_account()?;
            let headers = match build_inference_headers(&access, &account, &request.session_id) {
                Ok(headers) => headers,
                Err(error) => {
                    access.zeroize();
                    account.zeroize();
                    return Err(error);
                }
            };

            #[cfg(test)]
            {
                let transport = self.test_transport.lock().expect("lock").clone();
                if let Some(transport) = transport {
                    let mock = transport.inference(INFERENCE_URL, &headers, &request.request_json);
                    access.zeroize();
                    account.zeroize();
                    let result = match mock {
                        Ok(mock) => emit_mock_inference(mock, cancel, &mut on_event).await,
                        Err(error) => Err(error),
                    };
                    if matches!(result, Err(PublicError::AuthenticationRequired)) {
                        self.reconnect_required.store(true, Ordering::SeqCst);
                    }
                    return result;
                }
            }

            let request_future = self
                .client
                .post(INFERENCE_URL)
                .headers(headers)
                .body(request.request_json)
                .send();
            let response_result = tokio::select! {
                _ = cancel.cancelled() => Err(PublicError::Cancelled),
                response = request_future => response.map_err(|_| PublicError::ProviderUnavailable),
            };
            access.zeroize();
            account.zeroize();
            let response = response_result?;
            match response.status() {
                StatusCode::UNAUTHORIZED => {
                    self.reconnect_required.store(true, Ordering::SeqCst);
                    return Err(PublicError::AuthenticationRequired);
                }
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
        MockInference::WaitForCancellation => {
            cancel.cancelled().await;
            Err(PublicError::Cancelled)
        }
    }
}

fn build_inference_headers(
    access: &str,
    account: &str,
    session_id: &str,
) -> Result<HeaderMap, PublicError> {
    let mut headers = HeaderMap::with_capacity(7);
    let authorization = HeaderValue::from_str(&format!("Bearer {access}"))
        .map_err(|_| PublicError::AuthenticationRequired)?;
    let account =
        HeaderValue::from_str(account).map_err(|_| PublicError::AuthenticationRequired)?;
    let session = HeaderValue::from_str(session_id).map_err(|_| PublicError::InvalidInput)?;
    headers.insert(AUTHORIZATION, authorization);
    headers.insert(ACCEPT, HeaderValue::from_static("text/event-stream"));
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(
        HeaderName::from_static("originator"),
        HeaderValue::from_static("tule"),
    );
    headers.insert(USER_AGENT, HeaderValue::from_static("tule-desktop/0.1.0"));
    headers.insert(HeaderName::from_static("session_id"), session);
    headers.insert(HeaderName::from_static("chatgpt-account-id"), account);
    Ok(headers)
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
    let mut request_line = line.split_whitespace();
    let method = request_line.next().ok_or(PublicError::InvalidInput)?;
    let target = request_line.next().ok_or(PublicError::InvalidInput)?;
    let version = request_line.next().ok_or(PublicError::InvalidInput)?;
    if request_line.next().is_some()
        || method != "GET"
        || !version.starts_with("HTTP/1.")
        || target.contains('#')
    {
        return Err(PublicError::InvalidInput);
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != "/auth/callback" {
        return Err(PublicError::InvalidInput);
    }
    let params = parse_query(query)?;
    let state = params.get("state").ok_or(PublicError::InvalidInput)?;
    if state.len() > MAX_CALLBACK_VALUE || state != expected_state {
        let _ = socket
            .write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n")
            .await;
        return Err(PublicError::InvalidInput);
    }
    if params.contains_key("error") {
        let _ = socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 16\r\n\r\nSign-in declined")
            .await;
        return Err(PublicError::AuthenticationRequired);
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
        if map.insert(urldecoding(key)?, urldecoding(value)?).is_some() {
            return Err(PublicError::InvalidInput);
        }
    }
    Ok(map)
}

async fn parse_token_response(
    response: reqwest::Response,
    persistence: TokenPersistence,
) -> Result<TokenBundle, PublicError> {
    if !response.status().is_success() {
        return Err(PublicError::ProviderUnavailable);
    }
    let bytes = read_bounded_response_body(response, MAX_PROVIDER_BODY).await?;
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| PublicError::ProviderUnavailable)?;
    let mut access = value
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
    let account_result = extract_account_id(&id_token);
    id_token.zeroize();
    let mut account = account_result?;
    let expires_in = value
        .get("expires_in")
        .and_then(Value::as_i64)
        .unwrap_or(3600);
    let expires_at_unix_ms = unix_now_ms().ok().map(|now| now + expires_in * 1000);
    if refresh.is_empty() {
        if persistence == TokenPersistence::Initial {
            access.zeroize();
            account.zeroize();
            return Err(PublicError::ProviderUnavailable);
        }
        // Rotation without a replacement preserves the existing refresh credential.
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

async fn read_bounded_response_body(
    response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, PublicError> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(PublicError::InvalidInput);
    }
    let mut body = Vec::with_capacity(
        response
            .content_length()
            .and_then(|length| usize::try_from(length).ok())
            .unwrap_or(0)
            .min(limit),
    );
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| PublicError::ProviderUnavailable)?;
        if body.len().saturating_add(chunk.len()) > limit {
            body.zeroize();
            return Err(PublicError::InvalidInput);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
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
    loop {
        let next = tokio::select! {
            _ = cancel.cancelled() => return Err(PublicError::Cancelled),
            next = stream.next() => next,
        };
        let Some(next) = next else {
            break;
        };
        let chunk = next.map_err(|_| PublicError::ProviderUnavailable)?;
        for piece in chunk.chunks(4096) {
            if buffer.len().saturating_add(piece.len()) > MAX_SSE_BUFFER {
                return Err(PublicError::OutputLimit);
            }
            buffer.extend_from_slice(piece);
            emit_complete_sse_events(&mut buffer, &mut saw_completed, on_event)?;
            if saw_completed {
                return Ok(Vec::new());
            }
        }
    }
    if !buffer.is_empty() {
        let mut batch = Vec::new();
        parse_sse_event(&buffer, &mut batch)?;
        emit_provider_batch(batch, &mut saw_completed, on_event)?;
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
    let mut buffer = Vec::new();
    let mut saw_completed = false;
    for chunk in bytes.chunks(4096) {
        if buffer.len().saturating_add(chunk.len()) > MAX_SSE_BUFFER {
            return Err(PublicError::OutputLimit);
        }
        buffer.extend_from_slice(chunk);
        emit_complete_sse_events(&mut buffer, &mut saw_completed, on_event)?;
        if saw_completed {
            return Ok(Vec::new());
        }
    }
    if !buffer.is_empty() {
        let mut batch = Vec::new();
        parse_sse_event(&buffer, &mut batch)?;
        emit_provider_batch(batch, &mut saw_completed, on_event)?;
    }
    if saw_completed {
        Ok(Vec::new())
    } else {
        Err(PublicError::ProviderUnavailable)
    }
}

fn emit_complete_sse_events(
    buffer: &mut Vec<u8>,
    saw_completed: &mut bool,
    on_event: &mut ProviderEventSink,
) -> Result<(), PublicError> {
    while !*saw_completed {
        let Some(boundary) = find_event_boundary(buffer) else {
            break;
        };
        let event = buffer.drain(..boundary).collect::<Vec<_>>();
        let delimiter = event_delimiter_len(buffer);
        if delimiter > 0 {
            buffer.drain(..delimiter);
        }
        let mut batch = Vec::new();
        parse_sse_event(&event, &mut batch)?;
        emit_provider_batch(batch, saw_completed, on_event)?;
    }
    Ok(())
}

fn emit_provider_batch(
    batch: Vec<ProviderEvent>,
    saw_completed: &mut bool,
    on_event: &mut ProviderEventSink,
) -> Result<(), PublicError> {
    for item in batch {
        match &item {
            ProviderEvent::Completed { .. } if *saw_completed => {
                return Err(PublicError::ProviderUnavailable);
            }
            ProviderEvent::Completed { .. } => *saw_completed = true,
            ProviderEvent::Delta(_) if *saw_completed => {
                return Err(PublicError::ProviderUnavailable);
            }
            ProviderEvent::Delta(_) => {}
        }
        on_event(item)?;
    }
    Ok(())
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
        // Framing marker only. `response.completed` is the sole success terminal.
        return Ok(());
    }
    let value: Value = serde_json::from_str(&data).map_err(|_| PublicError::ProviderUnavailable)?;
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if contains_unsupported_provider_output(&value) {
        return Err(PublicError::UnsupportedProviderOutput);
    }
    match kind {
        "response.output_text.delta" => {
            let delta = value
                .get("delta")
                .and_then(Value::as_str)
                .ok_or(PublicError::ProviderUnavailable)?;
            output.push(ProviderEvent::Delta(delta.to_owned()));
        }
        "response.completed" => {
            output.push(ProviderEvent::Completed {
                response_id: value
                    .get("response")
                    .and_then(|response| response.get("id"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                input_tokens: value
                    .pointer("/response/usage/input_tokens")
                    .and_then(Value::as_u64),
                output_tokens: value
                    .pointer("/response/usage/output_tokens")
                    .and_then(Value::as_u64),
            });
        }
        "response.failed" | "response.incomplete" | "error" => {
            return Err(PublicError::ProviderUnavailable);
        }
        _ => {}
    }
    Ok(())
}

fn contains_unsupported_provider_output(value: &Value) -> bool {
    match value {
        Value::Object(fields) => fields.iter().any(|(key, value)| {
            (key == "type"
                && value
                    .as_str()
                    .is_some_and(is_unsupported_provider_output_type))
                || contains_unsupported_provider_output(value)
        }),
        Value::Array(items) => items.iter().any(contains_unsupported_provider_output),
        _ => false,
    }
}

fn is_unsupported_provider_output_type(kind: &str) -> bool {
    kind.contains("function")
        || kind.contains("tool")
        || kind.contains("computer_call")
        || kind.contains("web_search_call")
        || kind.contains("mcp_call")
        || kind.contains("shell_call")
        || kind.contains("image_generation_call")
        || kind.ends_with("_call")
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

fn urldecoding(value: &str) -> Result<String, PublicError> {
    let bytes = value.as_bytes();
    let mut out = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' => {
                if index + 2 >= bytes.len() {
                    return Err(PublicError::InvalidInput);
                }
                let high = decode_hex(bytes[index + 1]).ok_or(PublicError::InvalidInput)?;
                let low = decode_hex(bytes[index + 2]).ok_or(PublicError::InvalidInput)?;
                out.push((high << 4) | low);
                index += 3;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(out).map_err(|_| PublicError::InvalidInput)
}

const fn decode_hex(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
#[derive(Debug, Clone)]
pub(crate) enum MockInference {
    Events(Vec<ProviderEvent>),
    Sse(String),
    Status(u16),
    WaitForCancellation,
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
    fn inference(
        &self,
        inference_url: &str,
        headers: &HeaderMap,
        body: &str,
    ) -> Result<MockInference, PublicError>;
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
        for event in [
            r#"data: {"type":"response.function_call"}"#,
            r#"data: {"type":"response.output_item.added","item":{"type":"function_call"}}"#,
        ] {
            let mut output = Vec::new();
            assert_eq!(
                parse_sse_event(event.as_bytes(), &mut output),
                Err(PublicError::UnsupportedProviderOutput)
            );
        }
    }

    #[test]
    fn terminal_can_arrive_before_delta() {
        let mut output = Vec::new();
        parse_sse_event(
            br#"data: {"type":"response.completed","response":{"id":"response-1"}}"#,
            &mut output,
        )
        .unwrap();
        assert!(matches!(
            output.as_slice(),
            [ProviderEvent::Completed { .. }]
        ));

        output.clear();
        parse_sse_event(
            br#"data: {"type":"response.output_text.done"}"#,
            &mut output,
        )
        .unwrap();
        parse_sse_event(b"data: [DONE]", &mut output).unwrap();
        assert!(output.is_empty());
    }

    #[tokio::test]
    async fn sse_requires_exact_response_terminal_and_stops_after_completion() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&events);
        let mut sink: ProviderEventSink = Box::new(move |event| {
            captured.lock().unwrap().push(event);
            Ok(())
        });
        parse_sse_buffer(
            b"data: {\"type\":\"response.output_text.delta\",\"delta\":\"hello\"}\n\ndata: {\"type\":\"response.output_text.done\"}\n\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"r1\"}}\n\ndata: [DONE]\n\n",
            CancellationToken::new(),
            &mut sink,
        )
        .await
        .unwrap();
        assert!(matches!(
            events.lock().unwrap().as_slice(),
            [ProviderEvent::Delta(text), ProviderEvent::Completed { response_id: Some(id), .. }]
                if text == "hello" && id == "r1"
        ));

        let mut discard: ProviderEventSink = Box::new(|_| Ok(()));
        let incomplete = parse_sse_buffer(
            b"data: {\"type\":\"response.output_text.done\"}\n\ndata: [DONE]\n\n",
            CancellationToken::new(),
            &mut discard,
        )
        .await;
        assert_eq!(incomplete, Err(PublicError::ProviderUnavailable));

        let duplicate_events = Arc::new(Mutex::new(Vec::new()));
        let duplicate_events_cb = Arc::clone(&duplicate_events);
        let mut capture_duplicate: ProviderEventSink = Box::new(move |event| {
            duplicate_events_cb.lock().unwrap().push(event);
            Ok(())
        });
        parse_sse_buffer(
            b"data: {\"type\":\"response.completed\",\"response\":{}}\n\ndata: {\"type\":\"response.completed\",\"response\":{}}\n\n",
            CancellationToken::new(),
            &mut capture_duplicate,
        )
        .await
        .unwrap();
        assert!(matches!(
            duplicate_events.lock().unwrap().as_slice(),
            [ProviderEvent::Completed { .. }]
        ));
    }

    #[test]
    fn connection_status_requires_a_complete_credential_set_across_restart() {
        let fake = Arc::new(FakeCredentialStore::default());
        let dir = tempfile_dir();
        let store = SqliteStore::open(dir.join("tule.sqlite3")).unwrap();
        let adapter = ChatGptAdapter::new(fake.clone());
        assert_eq!(
            adapter.connection_status_with_store(&store).state,
            ConnectionState::Disconnected
        );

        fake.replace(CREDENTIAL_HANDLE, CredentialKind::RefreshToken, b"refresh")
            .unwrap();
        assert_eq!(
            adapter.connection_status_with_store(&store).state,
            ConnectionState::ReconnectRequired
        );
        let restarted = ChatGptAdapter::new(fake.clone());
        assert_eq!(
            restarted.connection_status_with_store(&store).state,
            ConnectionState::ReconnectRequired
        );

        fake.replace(CREDENTIAL_HANDLE, CredentialKind::AccessToken, b"access")
            .unwrap();
        fake.replace(CREDENTIAL_HANDLE, CredentialKind::AccountId, b"account")
            .unwrap();
        assert_eq!(
            restarted.connection_status_with_store(&store).state,
            ConnectionState::ReconnectRequired
        );

        let mut wrong_profile = store
            .get_provider_profile(PROVIDER_PROFILE_ID)
            .unwrap()
            .unwrap();
        wrong_profile.set_credential_metadata(Some("unexpected-handle".into()), None, 1);
        store.update_provider_profile(&wrong_profile).unwrap();
        assert_eq!(
            restarted.connection_status_with_store(&store).state,
            ConnectionState::ReconnectRequired
        );

        mark_profile_connected(&store, Some(i64::MAX / 2));
        let committed_restart = ChatGptAdapter::new(fake);
        assert_eq!(
            committed_restart.connection_status_with_store(&store).state,
            ConnectionState::Connected
        );
    }

    #[test]
    fn cancel_connect_rejects_noop_and_duplicate_requests() {
        let adapter = ChatGptAdapter::new(Arc::new(FakeCredentialStore::default()));
        assert_eq!(adapter.cancel_connect(), Err(PublicError::InvalidInput));

        let cancel = Arc::new(CancellationToken::new());
        *adapter.connect_cancel.lock().unwrap() = Some(cancel.clone());
        assert_eq!(adapter.cancel_connect(), Ok(()));
        assert!(cancel.is_cancelled());
        assert_eq!(adapter.cancel_connect(), Err(PublicError::InvalidInput));
    }

    #[tokio::test]
    async fn successful_browser_connect_returns_connected_after_clearing_connecting() {
        struct SuccessExchange;
        impl TestTransport for SuccessExchange {
            fn exchange_token(
                &self,
                _: &str,
                _: &str,
                _: &str,
                _: &str,
            ) -> Result<TokenBundle, PublicError> {
                Ok(TokenBundle::Success(token_values(
                    "access", "refresh", "account", false,
                )))
            }
            fn refresh_token(&self, _: &str, _: &str) -> Result<TokenBundle, PublicError> {
                unreachable!()
            }
            fn inference(
                &self,
                _: &str,
                _: &HeaderMap,
                _: &str,
            ) -> Result<MockInference, PublicError> {
                unreachable!()
            }
        }

        let fake = Arc::new(FakeCredentialStore::default());
        let adapter = ChatGptAdapter::new(fake.clone());
        adapter.set_test_transport(Arc::new(SuccessExchange));
        let dir = unique_connect_tempfile_dir("success");
        let store = Arc::new(SqliteStore::open(dir.path().join("tule.sqlite3")).unwrap());

        let status = complete_browser_connect(&adapter, Arc::clone(&store), "ok-code").await;
        assert_eq!(status.state, ConnectionState::Connected);
        assert!(!adapter.connecting.load(Ordering::SeqCst));
        assert!(adapter.connect_cancel.lock().unwrap().is_none());
        // Completion already won; late Cancel must not claim cancellation.
        assert_eq!(adapter.cancel_connect(), Err(PublicError::InvalidInput));
        assert_eq!(
            adapter.connection_status_with_store(store.as_ref()).state,
            ConnectionState::Connected
        );
        for kind in ALL_CREDENTIAL_KINDS {
            assert!(fake.peek(CREDENTIAL_HANDLE, kind).is_some());
        }
    }

    #[tokio::test]
    async fn active_browser_connect_cancellation_returns_cancelled_without_credentials() {
        let _guard = CONNECT_TEST_LOCK.lock().await;
        let fake = Arc::new(FakeCredentialStore::default());
        let adapter = Arc::new(ChatGptAdapter::new(fake.clone()));
        let dir = unique_connect_tempfile_dir("cancel");
        let store = Arc::new(SqliteStore::open(dir.path().join("tule.sqlite3")).unwrap());
        let cancel_adapter = Arc::clone(&adapter);

        let result = adapter
            .connect_in_browser(store, move |_url| cancel_adapter.cancel_connect())
            .await;
        assert_eq!(result, Err(PublicError::Cancelled));
        assert!(!adapter.connecting.load(Ordering::SeqCst));
        assert!(adapter.connect_cancel.lock().unwrap().is_none());
        assert_eq!(
            adapter.connection_status().state,
            ConnectionState::Disconnected
        );
        for kind in ALL_CREDENTIAL_KINDS {
            assert_eq!(fake.peek(CREDENTIAL_HANDLE, kind), None);
        }
    }

    #[tokio::test]
    async fn cancellation_during_token_exchange_does_not_persist_credentials() {
        struct CancelExchange(CancellationToken);
        impl TestTransport for CancelExchange {
            fn exchange_token(
                &self,
                _: &str,
                _: &str,
                _: &str,
                _: &str,
            ) -> Result<TokenBundle, PublicError> {
                self.0.cancel();
                Ok(TokenBundle::Success(token_values(
                    "access", "refresh", "account", false,
                )))
            }
            fn refresh_token(&self, _: &str, _: &str) -> Result<TokenBundle, PublicError> {
                unreachable!()
            }
            fn inference(
                &self,
                _: &str,
                _: &HeaderMap,
                _: &str,
            ) -> Result<MockInference, PublicError> {
                unreachable!()
            }
        }

        let fake = Arc::new(FakeCredentialStore::default());
        let adapter = ChatGptAdapter::new(fake.clone());
        let cancel = CancellationToken::new();
        adapter.set_test_transport(Arc::new(CancelExchange(cancel.clone())));
        assert!(matches!(
            adapter
                .exchange_token("redirect", "verifier", "code", &cancel)
                .await,
            Err(PublicError::Cancelled)
        ));
        for kind in ALL_CREDENTIAL_KINDS {
            assert_eq!(fake.peek(CREDENTIAL_HANDLE, kind), None);
        }
    }

    #[test]
    fn cancellation_during_persistence_restores_the_credential_snapshot() {
        struct CancelOnFirstWrite {
            inner: FakeCredentialStore,
            cancel: CancellationToken,
            writes: std::sync::atomic::AtomicUsize,
        }
        impl CredentialStore for CancelOnFirstWrite {
            fn read(
                &self,
                handle: &str,
                kind: CredentialKind,
            ) -> Result<Option<Vec<u8>>, CredentialStoreError> {
                self.inner.read(handle, kind)
            }
            fn replace(
                &self,
                handle: &str,
                kind: CredentialKind,
                value: &[u8],
            ) -> Result<(), CredentialStoreError> {
                self.inner.replace(handle, kind, value)?;
                if self.writes.fetch_add(1, Ordering::SeqCst) == 0 {
                    self.cancel.cancel();
                }
                Ok(())
            }
            fn delete(
                &self,
                handle: &str,
                kind: CredentialKind,
            ) -> Result<(), CredentialStoreError> {
                self.inner.delete(handle, kind)
            }
        }

        let cancel = Arc::new(CancellationToken::new());
        let credentials = Arc::new(CancelOnFirstWrite {
            inner: FakeCredentialStore::default(),
            cancel: cancel.as_ref().clone(),
            writes: std::sync::atomic::AtomicUsize::new(0),
        });
        let adapter = ChatGptAdapter::new(credentials.clone());
        *adapter.connect_cancel.lock().unwrap() = Some(cancel.clone());
        let dir = tempfile_dir();
        let store = SqliteStore::open(dir.join("tule.sqlite3")).unwrap();
        assert_eq!(
            adapter.persist_tokens(
                &store,
                token_values("access", "refresh", "account", false),
                TokenPersistence::Initial,
                PersistenceCancellation::BrowserConnect(&cancel),
            ),
            Err(PublicError::Cancelled)
        );
        for kind in ALL_CREDENTIAL_KINDS {
            assert_eq!(credentials.inner.peek(CREDENTIAL_HANDLE, kind), None);
        }
    }

    #[test]
    fn credential_write_order_compensates_on_failure() {
        for failed_operation in 4..=6 {
            let fake = Arc::new(FakeCredentialStore::default());
            fake.fail_on_operation(failed_operation);
            let adapter = ChatGptAdapter::new(fake.clone());
            let dir = tempfile_dir();
            let store = SqliteStore::open(dir.join("tule.sqlite3")).unwrap();
            let result = adapter.persist_tokens(
                &store,
                token_values("access", "refresh", "account", false),
                TokenPersistence::Initial,
                PersistenceCancellation::None,
            );
            assert_eq!(result, Err(PublicError::CredentialStoreUnavailable));
            for kind in ALL_CREDENTIAL_KINDS {
                assert_eq!(fake.peek(CREDENTIAL_HANDLE, kind), None);
            }
            assert_ne!(
                adapter.connection_status().state,
                ConnectionState::Connected
            );
        }
    }

    #[test]
    fn metadata_failure_restores_initial_credential_state() {
        let fake = Arc::new(FakeCredentialStore::default());
        let adapter = ChatGptAdapter::new(fake.clone());
        let dir = tempfile_dir();
        let store = SqliteStore::open(dir.join("tule.sqlite3")).unwrap();
        store
            .connection()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER fail_provider_update
                 BEFORE UPDATE ON provider_profiles
                 BEGIN SELECT RAISE(ABORT, 'injected'); END;",
            )
            .unwrap();

        assert_eq!(
            adapter.persist_tokens(
                &store,
                token_values("access", "refresh", "account", false),
                TokenPersistence::Initial,
                PersistenceCancellation::None,
            ),
            Err(PublicError::AgentStorageUnavailable)
        );
        for kind in ALL_CREDENTIAL_KINDS {
            assert_eq!(fake.peek(CREDENTIAL_HANDLE, kind), None);
        }
    }

    #[test]
    fn failed_rotation_restoration_leaves_the_commit_marker_cleared() {
        let fake = Arc::new(FakeCredentialStore::default());
        seed_connected(&fake);
        let dir = tempfile_dir();
        let store = SqliteStore::open(dir.join("tule.sqlite3")).unwrap();
        mark_profile_connected(&store, Some(0));
        fake.reset_operations();
        fake.fail_on_operations([5, 6]);
        let adapter = ChatGptAdapter::new(fake.clone());

        assert_eq!(
            adapter.persist_tokens(
                &store,
                token_values("new-access", "new-refresh", "new-account", false),
                TokenPersistence::Rotation,
                PersistenceCancellation::None,
            ),
            Err(PublicError::CredentialStoreUnavailable)
        );
        assert_eq!(
            store
                .get_provider_profile(PROVIDER_PROFILE_ID)
                .unwrap()
                .unwrap()
                .credential_handle(),
            None
        );

        let restarted = ChatGptAdapter::new(fake);
        assert_eq!(
            restarted.connection_status_with_store(&store).state,
            ConnectionState::ReconnectRequired
        );
    }

    #[test]
    fn initial_connection_requires_refresh_but_rotation_may_preserve_it() {
        let fake = Arc::new(FakeCredentialStore::default());
        let adapter = ChatGptAdapter::new(fake.clone());
        let dir = tempfile_dir();
        let store = SqliteStore::open(dir.join("tule.sqlite3")).unwrap();
        assert_eq!(
            adapter.persist_tokens(
                &store,
                token_values("access", "", "account", true),
                TokenPersistence::Initial,
                PersistenceCancellation::None,
            ),
            Err(PublicError::ProviderUnavailable)
        );
        for kind in ALL_CREDENTIAL_KINDS {
            assert_eq!(fake.peek(CREDENTIAL_HANDLE, kind), None);
        }

        seed_connected(&fake);
        fake.reset_operations();
        adapter
            .persist_tokens(
                &store,
                token_values("new-access", "", "new-account", true),
                TokenPersistence::Rotation,
                PersistenceCancellation::None,
            )
            .unwrap();
        assert_eq!(
            fake.peek(CREDENTIAL_HANDLE, CredentialKind::RefreshToken),
            Some(b"refresh".to_vec())
        );
        assert_eq!(
            fake.peek(CREDENTIAL_HANDLE, CredentialKind::AccessToken),
            Some(b"new-access".to_vec())
        );
    }

    #[test]
    fn disconnect_failure_restores_credentials_or_fails_closed() {
        for failed_operation in 4..=9 {
            let fake = Arc::new(FakeCredentialStore::default());
            seed_connected(&fake);
            fake.reset_operations();
            fake.fail_on_operation(failed_operation);
            let adapter = ChatGptAdapter::new(fake.clone());
            let dir = tempfile_dir();
            let store = SqliteStore::open(dir.join("tule.sqlite3")).unwrap();
            mark_profile_connected(&store, Some(i64::MAX / 2));

            assert_eq!(
                adapter.disconnect(&store),
                Err(PublicError::CredentialStoreUnavailable)
            );
            assert_eq!(
                adapter.connection_status_with_store(&store).state,
                ConnectionState::Connected
            );
            assert_eq!(
                fake.peek(CREDENTIAL_HANDLE, CredentialKind::RefreshToken),
                Some(b"refresh".to_vec())
            );
            assert_eq!(
                fake.peek(CREDENTIAL_HANDLE, CredentialKind::AccessToken),
                Some(b"access".to_vec())
            );
            assert_eq!(
                fake.peek(CREDENTIAL_HANDLE, CredentialKind::AccountId),
                Some(b"account".to_vec())
            );
        }

        let fake = Arc::new(FakeCredentialStore::default());
        seed_connected(&fake);
        fake.reset_operations();
        fake.fail_on_operations([5, 6]);
        let adapter = ChatGptAdapter::new(fake);
        let dir = tempfile_dir();
        let store = SqliteStore::open(dir.join("tule.sqlite3")).unwrap();
        mark_profile_connected(&store, Some(i64::MAX / 2));
        assert_eq!(
            adapter.disconnect(&store),
            Err(PublicError::CredentialStoreUnavailable)
        );
        assert_eq!(
            adapter.connection_status().state,
            ConnectionState::ReconnectRequired
        );
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
        assert_eq!(
            parse_query("state=one&state=two"),
            Err(PublicError::InvalidInput)
        );
        assert_eq!(
            parse_query("state=%💩&code=x"),
            Err(PublicError::InvalidInput)
        );
        assert_eq!(
            parse_query("state=%FF&code=x"),
            Err(PublicError::InvalidInput)
        );
    }

    #[tokio::test]
    async fn callback_requires_exact_method_path_and_state_before_result() {
        assert_eq!(
            callback_result(
                "GET /auth/callback?state=expected&code=ok HTTP/1.1\r\nHost: localhost\r\n\r\n",
                "expected",
            )
            .await,
            Ok("ok".to_owned())
        );
        for request in [
            "POST /auth/callback?state=expected&code=ok HTTP/1.1\r\nHost: localhost\r\n\r\n",
            "GET /wrong?state=expected&code=ok HTTP/1.1\r\nHost: localhost\r\n\r\n",
            "GET /auth/callback?state=wrong&code=ok HTTP/1.1\r\nHost: localhost\r\n\r\n",
            "GET /auth/callback?error=access_denied HTTP/1.1\r\nHost: localhost\r\n\r\n",
        ] {
            assert_eq!(
                callback_result(request, "expected").await,
                Err(PublicError::InvalidInput)
            );
        }
        assert_eq!(
            callback_result(
                "GET /auth/callback?state=expected&error=access_denied HTTP/1.1\r\nHost: localhost\r\n\r\n",
                "expected",
            )
            .await,
            Err(PublicError::AuthenticationRequired)
        );
    }

    #[tokio::test]
    async fn token_body_limit_is_enforced_during_streamed_read() {
        let response = local_http_response(StatusCode::OK, vec![b'x'; MAX_PROVIDER_BODY + 1]).await;
        assert!(matches!(
            parse_token_response(response, TokenPersistence::Initial).await,
            Err(PublicError::InvalidInput)
        ));
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
            fn inference(
                &self,
                url: &str,
                _: &HeaderMap,
                _: &str,
            ) -> Result<MockInference, PublicError> {
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
            fn inference(
                &self,
                url: &str,
                headers: &HeaderMap,
                body: &str,
            ) -> Result<MockInference, PublicError> {
                assert_eq!(url, INFERENCE_URL);
                assert_eq!(headers.len(), 7);
                assert_eq!(headers.get(AUTHORIZATION).unwrap(), "Bearer a");
                assert_eq!(headers.get(ACCEPT).unwrap(), "text/event-stream");
                assert_eq!(headers.get(CONTENT_TYPE).unwrap(), "application/json");
                assert_eq!(headers.get("originator").unwrap(), "tule");
                assert_eq!(headers.get(USER_AGENT).unwrap(), "tule-desktop/0.1.0");
                assert_eq!(headers.get("session_id").unwrap(), "s");
                assert_eq!(headers.get("ChatGPT-Account-Id").unwrap(), "acct");
                assert!(!body.is_empty());
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

        adapter.set_test_transport(Arc::new(SseTransport(
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"r1\"}}\n\ndata: [DONE]\n\n".into(),
        )));
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
        assert_eq!(
            adapter.connection_status().state,
            ConnectionState::ReconnectRequired
        );

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
            fn inference(
                &self,
                _: &str,
                _: &HeaderMap,
                _: &str,
            ) -> Result<MockInference, PublicError> {
                unreachable!()
            }
        }
        adapter.set_test_transport(Arc::new(InvalidGrant));
        let dir = tempfile_dir();
        let store = SqliteStore::open(dir.join("tule.sqlite3")).unwrap();
        let _operation = adapter.profile_lock.lock().await;
        let err = adapter.refresh_access_token_locked(&store, None).await;
        assert_eq!(err, Err(PublicError::AuthenticationRequired));
        assert_eq!(
            adapter.connection_status().state,
            ConnectionState::ReconnectRequired
        );
    }

    #[tokio::test]
    async fn concurrent_freshness_checks_share_one_refresh() {
        use std::sync::atomic::AtomicUsize;

        let fake = Arc::new(FakeCredentialStore::default());
        seed_connected(&fake);
        let adapter = ChatGptAdapter::new(fake);
        let calls = Arc::new(AtomicUsize::new(0));
        struct RefreshOnce(Arc<AtomicUsize>);
        impl TestTransport for RefreshOnce {
            fn exchange_token(
                &self,
                _: &str,
                _: &str,
                _: &str,
                _: &str,
            ) -> Result<TokenBundle, PublicError> {
                unreachable!()
            }
            fn refresh_token(&self, url: &str, refresh: &str) -> Result<TokenBundle, PublicError> {
                assert_eq!(url, TOKEN_URL);
                assert_eq!(refresh, "refresh");
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(TokenBundle::Success(TokenValues {
                    access: "new-access".into(),
                    refresh: "new-refresh".into(),
                    account: "account".into(),
                    expires_at_unix_ms: Some(i64::MAX / 2),
                    preserve_refresh: false,
                }))
            }
            fn inference(
                &self,
                _: &str,
                _: &HeaderMap,
                _: &str,
            ) -> Result<MockInference, PublicError> {
                unreachable!()
            }
        }
        adapter.set_test_transport(Arc::new(RefreshOnce(Arc::clone(&calls))));
        let dir = tempfile_dir();
        let store = SqliteStore::open(dir.join("tule.sqlite3")).unwrap();
        let mut profile = store
            .get_provider_profile(PROVIDER_PROFILE_ID)
            .unwrap()
            .unwrap();
        profile.set_credential_metadata(Some(CREDENTIAL_HANDLE.into()), Some(0), 1);
        store.update_provider_profile(&profile).unwrap();

        let (first, second) = tokio::join!(
            adapter.ensure_fresh_access_public(&store),
            adapter.ensure_fresh_access_public(&store)
        );
        first.unwrap();
        second.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn pre_stream_cancellation_interrupts_refresh_without_rotating_credentials() {
        let fake = Arc::new(FakeCredentialStore::default());
        seed_connected(&fake);
        let adapter = ChatGptAdapter::new(fake.clone());
        let cancel = CancellationToken::new();
        struct CancelRefresh(CancellationToken);
        impl TestTransport for CancelRefresh {
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
                self.0.cancel();
                Ok(TokenBundle::Success(token_values(
                    "new-access",
                    "new-refresh",
                    "new-account",
                    false,
                )))
            }
            fn inference(
                &self,
                _: &str,
                _: &HeaderMap,
                _: &str,
            ) -> Result<MockInference, PublicError> {
                unreachable!()
            }
        }
        adapter.set_test_transport(Arc::new(CancelRefresh(cancel.clone())));
        let dir = tempfile_dir();
        let store = SqliteStore::open(dir.join("tule.sqlite3")).unwrap();
        let mut profile = store
            .get_provider_profile(PROVIDER_PROFILE_ID)
            .unwrap()
            .unwrap();
        profile.set_credential_metadata(Some(CREDENTIAL_HANDLE.into()), Some(0), 1);
        store.update_provider_profile(&profile).unwrap();

        assert_eq!(
            adapter
                .ensure_fresh_access_cancellable_public(&store, cancel)
                .await,
            Err(PublicError::Cancelled)
        );
        assert_eq!(
            fake.peek(CREDENTIAL_HANDLE, CredentialKind::RefreshToken),
            Some(b"refresh".to_vec())
        );
        assert_eq!(
            fake.peek(CREDENTIAL_HANDLE, CredentialKind::AccessToken),
            Some(b"access".to_vec())
        );
        assert_eq!(
            fake.peek(CREDENTIAL_HANDLE, CredentialKind::AccountId),
            Some(b"account".to_vec())
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
            fn inference(
                &self,
                url: &str,
                _: &HeaderMap,
                _: &str,
            ) -> Result<MockInference, PublicError> {
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

    #[tokio::test]
    async fn idle_stream_cancels_promptly_and_blocks_disconnect() {
        let fake = Arc::new(FakeCredentialStore::default());
        seed_connected(&fake);
        let adapter = ChatGptAdapter::new(fake);
        struct WaitTransport;
        impl TestTransport for WaitTransport {
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
            fn inference(
                &self,
                url: &str,
                _: &HeaderMap,
                _: &str,
            ) -> Result<MockInference, PublicError> {
                assert_eq!(url, INFERENCE_URL);
                Ok(MockInference::WaitForCancellation)
            }
        }
        adapter.set_test_transport(Arc::new(WaitTransport));
        let dir = tempfile_dir();
        let store = SqliteStore::open(dir.join("tule.sqlite3")).unwrap();
        let cancel = CancellationToken::new();
        let control_cancel = cancel.clone();
        let stream = adapter.stream(
            ProviderRequest {
                session_id: "session".into(),
                request_json: "{}".into(),
            },
            cancel,
            Box::new(|_| Ok(())),
        );
        let control = async {
            tokio::task::yield_now().await;
            assert_eq!(adapter.disconnect(&store), Err(PublicError::SessionBusy));
            control_cancel.cancel();
        };
        let (result, ()) = tokio::join!(stream, control);
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

    // Fixed callback ports cannot be shared across parallel browser-connect tests.
    static CONNECT_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    fn unique_connect_tempfile_dir(label: &str) -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix(&format!("tule-openai-connect-{label}-"))
            .tempdir()
            .expect("tempdir")
    }

    async fn complete_browser_connect(
        adapter: &ChatGptAdapter,
        store: Arc<SqliteStore>,
        code: &str,
    ) -> ConnectionStatus {
        let _guard = CONNECT_TEST_LOCK.lock().await;
        let code = code.to_owned();
        adapter
            .connect_in_browser(store, move |url| {
                let query = url.split_once('?').map(|(_, query)| query).expect("query");
                let params = parse_query(query).expect("authorization query");
                let state = params.get("state").expect("state").clone();
                let redirect = params.get("redirect_uri").expect("redirect_uri").clone();
                let callback = format!("{redirect}?state={state}&code={code}");
                tokio::spawn(async move {
                    let response = reqwest::get(callback).await.expect("callback request");
                    assert!(response.status().is_success());
                });
                Ok(())
            })
            .await
            .expect("browser connect should succeed")
    }

    fn token_values(
        access: &str,
        refresh: &str,
        account: &str,
        preserve_refresh: bool,
    ) -> TokenValues {
        TokenValues {
            access: access.into(),
            refresh: refresh.into(),
            account: account.into(),
            expires_at_unix_ms: Some(i64::MAX / 2),
            preserve_refresh,
        }
    }

    fn seed_connected(store: &FakeCredentialStore) {
        store
            .replace(CREDENTIAL_HANDLE, CredentialKind::RefreshToken, b"refresh")
            .unwrap();
        store
            .replace(CREDENTIAL_HANDLE, CredentialKind::AccessToken, b"access")
            .unwrap();
        store
            .replace(CREDENTIAL_HANDLE, CredentialKind::AccountId, b"account")
            .unwrap();
    }

    fn mark_profile_connected(store: &SqliteStore, expires_at_unix_ms: Option<i64>) {
        let mut profile = store
            .get_provider_profile(PROVIDER_PROFILE_ID)
            .unwrap()
            .unwrap();
        profile.set_credential_metadata(Some(CREDENTIAL_HANDLE.into()), expires_at_unix_ms, 1);
        store.update_provider_profile(&profile).unwrap();
    }

    async fn callback_result(request: &str, expected_state: &str) -> Result<String, PublicError> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = accept_callback(listener, expected_state);
        let client = async {
            let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
            socket.write_all(request.as_bytes()).await.unwrap();
            let mut response = Vec::new();
            let _ = socket.read_to_end(&mut response).await;
        };
        let (result, ()) = tokio::join!(server, client);
        result
    }

    async fn local_http_response(status: StatusCode, body: Vec<u8>) -> reqwest::Response {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            let _ = socket.read(&mut request).await;
            let header = format!(
                "HTTP/1.1 {} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                status.as_u16(),
                body.len()
            );
            socket.write_all(header.as_bytes()).await.unwrap();
            socket.write_all(&body).await.unwrap();
        };
        let client = async move {
            Client::new()
                .get(format!("http://{address}"))
                .send()
                .await
                .unwrap()
        };
        let ((), response) = tokio::join!(server, client);
        response
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
