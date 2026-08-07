//! Fixed-contract xAI SuperGrok / X Premium subscription OAuth adapter (RFC 8628 device-code).
//!
//! Endpoint values are compile-time constants. A `cfg(test)` transport seam may
//! supply bounded mock responses while asserting production destinations; the
//! seam does not compile into release binaries.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use futures_util::StreamExt;
use reqwest::{
    Client, StatusCode,
    header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue, USER_AGENT},
    redirect::Policy,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::time::sleep;
use tule_core::{
    AgentContextError, AgentEffort, AgentRepository, AgentRequestContext, CATALOG_TTL_MS,
    CatalogCandidate, MAX_CONTEXT_UTF8, ProviderProfile, catalog_freshness,
    format_turn_user_content, select_usable_catalog_entries,
};
use zeroize::Zeroize;

use crate::{
    credentials::{CredentialKind, CredentialStore, CredentialStoreError},
    provider::{
        ConnectionState, ConnectionStatus, MODEL_ID, PROVIDER_PROFILE_ID, ProviderAdapter,
        ProviderEvent, ProviderEventSink, ProviderFuture, ProviderModelCatalogResponse,
        ProviderRequest, PublicError, build_catalog_response,
    },
    sqlite::{SqliteStore, StoredCatalogState},
};
use tokio_util::sync::CancellationToken;

pub(crate) type DevicePairingNotifier = Arc<dyn Fn(Option<DevicePairingResponse>) + Send + Sync>;

pub(crate) const PROVIDER_ID: &str = PROVIDER_PROFILE_ID;
pub(crate) const MODEL: &str = MODEL_ID;
pub(crate) const CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
pub(crate) const DEVICE_CODE_URL: &str = "https://auth.x.ai/oauth2/device/code";
pub(crate) const TOKEN_URL: &str = "https://auth.x.ai/oauth2/token";
pub(crate) const INFERENCE_URL: &str = "https://api.x.ai/v1/chat/completions";
/// TULE-owned catalog-compatibility revision for the authenticated models endpoint.
pub(crate) const CATALOG_COMPATIBILITY_REVISION: &str = "1.0.0";
pub(crate) const MODELS_URL: &str = "https://api.x.ai/v1/models";
pub(crate) const SCOPES: &str = "openid profile email offline_access grok-cli:access api:access";
pub(crate) const DEVICE_CODE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
pub(crate) const CREDENTIAL_HANDLE: &str = "xai-subscription-oauth-v1";
/// Retired ChatGPT compatibility credential handle superseded on upgrade.
pub(crate) const LEGACY_CHATGPT_CREDENTIAL_HANDLE: &str = "openai-chatgpt-compat-v1";
pub(crate) const XAI_DEVICE_PAIRING_CHANGED_EVENT: &str = "xai-device-pairing-changed";

/// Allowlisted non-secret device-code pairing metadata for the interface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DevicePairingResponse {
    pub(crate) verification_uri: String,
    pub(crate) user_code: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DeviceCodeResponse {
    pub(crate) device_code: String,
    pub(crate) user_code: String,
    pub(crate) verification_uri: String,
    pub(crate) verification_uri_complete: Option<String>,
    pub(crate) expires_in: Option<u64>,
    pub(crate) interval: Option<u64>,
}
#[cfg(not(test))]
const MAX_CATALOG_BODY: usize = 512 * 1024;
const DEVICE_CODE_DEFAULT_INTERVAL_MS: u64 = 5_000;
const DEVICE_CODE_MIN_INTERVAL_MS: u64 = 1_000;
const DEVICE_CODE_SLOW_DOWN_INCREMENT_MS: u64 = 5_000;
const DEVICE_CODE_DEFAULT_EXPIRES_MS: u64 = 5 * 60 * 1000;
const OAUTH_POLLING_SAFETY_MARGIN_MS: u64 = 3_000;
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

pub(crate) struct XaiSubscriptionAdapter {
    credentials: Arc<dyn CredentialStore>,
    client: Client,
    /// Serializes every credential-using operation for the single built-in profile.
    profile_lock: tokio::sync::Mutex<()>,
    connect_cancel: Mutex<Option<Arc<CancellationToken>>>,
    connecting: AtomicBool,
    reconnect_required: AtomicBool,
    device_pairing: Mutex<Option<DevicePairingResponse>>,
    #[cfg(test)]
    test_transport: Mutex<Option<Arc<dyn TestTransport>>>,
}

impl XaiSubscriptionAdapter {
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
            device_pairing: Mutex::new(None),
            #[cfg(test)]
            test_transport: Mutex::new(None),
        }
    }

    /// Removes retired ChatGPT credential material so it cannot remain the active send path.
    pub(crate) fn supersede_legacy_chatgpt_credentials(&self) {
        for kind in ALL_CREDENTIAL_KINDS {
            let _ = self
                .credentials
                .delete(LEGACY_CHATGPT_CREDENTIAL_HANDLE, kind);
        }
    }

    pub(crate) fn device_pairing(&self) -> Option<DevicePairingResponse> {
        self.device_pairing.lock().ok()?.clone()
    }

    fn set_device_pairing(&self, pairing: Option<DevicePairingResponse>) {
        if let Ok(mut slot) = self.device_pairing.lock() {
            *slot = pairing;
        }
    }

    fn notify_device_pairing(
        on_pairing: Option<&DevicePairingNotifier>,
        pairing: Option<DevicePairingResponse>,
    ) {
        if let Some(notify) = on_pairing {
            notify(pairing);
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

    /// Test-only helper that injects a device-code response and completes token polling via transport.
    #[cfg(test)]
    pub(crate) async fn connect_with_test_device_code(
        &self,
        store: Arc<SqliteStore>,
    ) -> Result<ConnectionStatus, PublicError> {
        self.connect_device_code(store, |_| Ok(()), None).await
    }

    pub(crate) async fn connect_device_code(
        &self,
        store: Arc<SqliteStore>,
        open_url: impl FnOnce(&str) -> Result<(), PublicError>,
        on_pairing: Option<DevicePairingNotifier>,
    ) -> Result<ConnectionStatus, PublicError> {
        if self.connection_status_with_store(store.as_ref()).state
            == ConnectionState::UnavailableInThisBuild
        {
            return Ok(self.connection_status_with_store(store.as_ref()));
        }
        if self.connecting.swap(true, Ordering::SeqCst) {
            return Err(PublicError::SessionBusy);
        }
        self.set_device_pairing(None);
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
            .connect_device_code_inner(Arc::clone(&store), open_url, &cancel, on_pairing.as_ref())
            .await;
        drop(operation);
        let cleanup = self.clear_connect_cancel(&cancel);
        self.connecting.store(false, Ordering::SeqCst);
        self.set_device_pairing(None);
        Self::notify_device_pairing(on_pairing.as_ref(), None);
        match (result, cleanup) {
            (Ok(()), Ok(())) => Ok(self.connection_status_with_store(store.as_ref())),
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error),
        }
    }

    async fn connect_device_code_inner(
        &self,
        store: Arc<SqliteStore>,
        open_url: impl FnOnce(&str) -> Result<(), PublicError>,
        cancel: &Arc<CancellationToken>,
        on_pairing: Option<&DevicePairingNotifier>,
    ) -> Result<(), PublicError> {
        if cancel.is_cancelled() {
            return Err(PublicError::Cancelled);
        }
        let device = self.request_device_code(cancel).await?;
        let open_target = device
            .verification_uri_complete
            .as_deref()
            .unwrap_or(&device.verification_uri);
        let pairing = DevicePairingResponse {
            verification_uri: device.verification_uri.clone(),
            user_code: device.user_code.clone(),
        };
        self.set_device_pairing(Some(pairing.clone()));
        Self::notify_device_pairing(on_pairing, Some(pairing));
        if cancel.is_cancelled() {
            return Err(PublicError::Cancelled);
        }
        open_url(open_target)?;
        let token = self.poll_device_code_token(&device, cancel).await?;
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

    async fn request_device_code(
        &self,
        cancel: &CancellationToken,
    ) -> Result<DeviceCodeResponse, PublicError> {
        #[cfg(test)]
        if let Some(transport) = self.test_transport.lock().expect("lock").clone() {
            if cancel.is_cancelled() {
                return Err(PublicError::Cancelled);
            }
            return transport.request_device_code(DEVICE_CODE_URL);
        }

        let body_with_referrer = format!(
            "client_id={}&scope={}&referrer=tule",
            urlencoding(CLIENT_ID),
            urlencoding(SCOPES)
        );
        let body_without_referrer = format!(
            "client_id={}&scope={}",
            urlencoding(CLIENT_ID),
            urlencoding(SCOPES)
        );
        let request_with = self
            .client
            .post(DEVICE_CODE_URL)
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header(ACCEPT, "application/json")
            .body(body_with_referrer);
        let response = tokio::select! {
            _ = cancel.cancelled() => return Err(PublicError::Cancelled),
            response = request_with.send() => response.map_err(|_| PublicError::ProviderUnavailable)?,
        };
        let response = if response.status().is_success() {
            response
        } else {
            let retry = self
                .client
                .post(DEVICE_CODE_URL)
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(ACCEPT, "application/json")
                .body(body_without_referrer);
            tokio::select! {
                _ = cancel.cancelled() => return Err(PublicError::Cancelled),
                response = retry.send() => response.map_err(|_| PublicError::ProviderUnavailable)?,
            }
        };
        if !response.status().is_success() {
            return Err(PublicError::ProviderUnavailable);
        }
        let bytes = tokio::select! {
            _ = cancel.cancelled() => return Err(PublicError::Cancelled),
            body = read_bounded_response_body(response, MAX_PROVIDER_BODY) => body?,
        };
        let device: DeviceCodeResponse =
            serde_json::from_slice(&bytes).map_err(|_| PublicError::ProviderUnavailable)?;
        if device.device_code.is_empty()
            || device.user_code.is_empty()
            || device.verification_uri.is_empty()
        {
            return Err(PublicError::ProviderUnavailable);
        }
        Ok(device)
    }

    async fn poll_device_code_token(
        &self,
        device: &DeviceCodeResponse,
        cancel: &CancellationToken,
    ) -> Result<TokenBundle, PublicError> {
        let expires_ms = positive_seconds_to_ms(device.expires_in, DEVICE_CODE_DEFAULT_EXPIRES_MS);
        let deadline =
            unix_now_ms().map_err(|_| PublicError::ProviderUnavailable)? + expires_ms as i64;
        let mut interval_ms =
            positive_seconds_to_ms(device.interval, DEVICE_CODE_DEFAULT_INTERVAL_MS)
                .max(DEVICE_CODE_MIN_INTERVAL_MS);

        loop {
            let now = unix_now_ms().map_err(|_| PublicError::ProviderUnavailable)?;
            if now >= deadline {
                break;
            }
            if cancel.is_cancelled() {
                return Err(PublicError::Cancelled);
            }
            #[cfg(test)]
            if let Some(transport) = self.test_transport.lock().expect("lock").clone() {
                return transport.poll_device_code_token(TOKEN_URL, &device.device_code);
            }

            let body = format!(
                "grant_type={}&client_id={}&device_code={}",
                urlencoding(DEVICE_CODE_GRANT),
                urlencoding(CLIENT_ID),
                urlencoding(&device.device_code)
            );
            let request = self
                .client
                .post(TOKEN_URL)
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(ACCEPT, "application/json")
                .body(body);
            let response = tokio::select! {
                _ = cancel.cancelled() => return Err(PublicError::Cancelled),
                response = request.send() => response.map_err(|_| PublicError::ProviderUnavailable)?,
            };
            if response.status().is_success() {
                return tokio::select! {
                    _ = cancel.cancelled() => Err(PublicError::Cancelled),
                    result = parse_token_response(response, TokenPersistence::Initial) => result,
                };
            }
            let error_body = tokio::select! {
                _ = cancel.cancelled() => return Err(PublicError::Cancelled),
                body = read_bounded_response_body(response, MAX_PROVIDER_BODY) => body?,
            };
            let error: Value = serde_json::from_slice(&error_body).unwrap_or(Value::Null);
            let code = error
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let now = unix_now_ms().map_err(|_| PublicError::ProviderUnavailable)?;
            let remaining = (deadline - now).max(0) as u64;
            match code {
                "authorization_pending" => {
                    sleep(Duration::from_millis(
                        (interval_ms + OAUTH_POLLING_SAFETY_MARGIN_MS).min(remaining),
                    ))
                    .await;
                }
                "slow_down" => {
                    interval_ms += DEVICE_CODE_SLOW_DOWN_INCREMENT_MS;
                    sleep(Duration::from_millis(
                        (interval_ms + OAUTH_POLLING_SAFETY_MARGIN_MS).min(remaining),
                    ))
                    .await;
                }
                "access_denied" | "authorization_denied" => {
                    return Err(PublicError::AuthenticationRequired);
                }
                "expired_token" => return Err(PublicError::ProviderUnavailable),
                "invalid_grant" => return Ok(TokenBundle::InvalidGrant),
                _ => return Err(PublicError::ProviderUnavailable),
            }
        }
        Err(PublicError::ProviderUnavailable)
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

    /// Returns the persisted catalog, refreshing when forced, stale, or missing.
    ///
    /// Failures preserve any last validated snapshot and selected default, but
    /// always surface the bounded error (never report stale success).
    pub(crate) async fn refresh_model_catalog(
        &self,
        store: &SqliteStore,
        force: bool,
    ) -> Result<ProviderModelCatalogResponse, PublicError> {
        let _operation = self.profile_lock.lock().await;
        match self.connection_status_with_store(store).state {
            ConnectionState::Connected => {}
            ConnectionState::ReconnectRequired => {
                return Err(PublicError::AuthenticationRequired);
            }
            ConnectionState::UnavailableInThisBuild => {
                return Err(PublicError::ProviderUnavailable);
            }
            _ => return Err(PublicError::NotConnected),
        }

        // Refresh expired access under the same gate already held (no nested lock).
        self.ensure_fresh_access_locked(store, None).await?;

        let now = unix_now_ms().map_err(|_| PublicError::AgentStorageUnavailable)?;
        let snapshot = store
            .get_catalog_snapshot(PROVIDER_PROFILE_ID)
            .map_err(|_| PublicError::AgentStorageUnavailable)?;
        if !force
            && let Some(snapshot) = snapshot.as_ref()
            && catalog_freshness(snapshot.state.retrieved_at_unix_ms, now)
                == tule_core::CatalogFreshness::Current
            && snapshot.state.compatibility_revision == CATALOG_COMPATIBILITY_REVISION
        {
            return build_catalog_response(store);
        }

        let etag = snapshot.as_ref().and_then(|item| item.state.etag.clone());
        let generation = store
            .current_credential_generation(PROVIDER_PROFILE_ID)
            .map_err(|_| PublicError::AgentStorageUnavailable)?;
        let (mut access, mut account) = self.read_access_and_account()?;
        let headers = match build_catalog_headers(&access, etag.as_deref()) {
            Ok(headers) => headers,
            Err(error) => {
                access.zeroize();
                account.zeroize();
                return Err(error);
            }
        };

        #[cfg(test)]
        let fetch_result: Result<ModelsFetchResult, PublicError> = {
            let transport = self.test_transport.lock().expect("lock").clone();
            if let Some(transport) = transport {
                let mock = transport.models(MODELS_URL, &headers, etag.as_deref());
                access.zeroize();
                account.zeroize();
                mock.map(ModelsFetchResult::from)
            } else {
                access.zeroize();
                account.zeroize();
                Err(PublicError::ProviderUnavailable)
            }
        };
        #[cfg(not(test))]
        let fetch_result = {
            let result = self.fetch_models_http(headers).await;
            access.zeroize();
            account.zeroize();
            result
        };

        match fetch_result {
            Ok(ModelsFetchResult::NotModified {
                etag: response_etag,
            }) => {
                store
                    .touch_catalog_retrieval(
                        PROVIDER_PROFILE_ID,
                        now,
                        now,
                        response_etag.as_deref().or(etag.as_deref()),
                    )
                    .map_err(|_| PublicError::AgentStorageUnavailable)?;
                build_catalog_response(store)
            }
            Ok(ModelsFetchResult::Models {
                body,
                etag: response_etag,
            }) => {
                let entries = parse_catalog_body(&body)?;
                let entries = store
                    .filter_rejected_catalog_entries(PROVIDER_PROFILE_ID, entries)
                    .map_err(|_| PublicError::AgentStorageUnavailable)?;
                if entries.is_empty() {
                    // Authenticated empty usable catalog after local rejection
                    // filtering is a contract failure. Preserve last-known snapshot.
                    return Err(PublicError::ProviderUnavailable);
                }
                store
                    .replace_catalog_snapshot(
                        PROVIDER_PROFILE_ID,
                        &StoredCatalogState {
                            credential_generation: generation,
                            compatibility_revision: CATALOG_COMPATIBILITY_REVISION.to_owned(),
                            etag: response_etag,
                            retrieved_at_unix_ms: now,
                            updated_at_unix_ms: now,
                        },
                        &entries,
                    )
                    .map_err(|_| PublicError::ProviderUnavailable)?;
                let _ = CATALOG_TTL_MS;
                build_catalog_response(store)
            }
            Ok(ModelsFetchResult::Status(401)) => Err(PublicError::AuthenticationRequired),
            Ok(ModelsFetchResult::Status(403)) => Err(PublicError::EntitlementUnavailable),
            Ok(ModelsFetchResult::Status(429)) => Err(PublicError::RateLimited),
            Ok(ModelsFetchResult::Status(_)) | Err(PublicError::ProviderUnavailable) => {
                Err(PublicError::ProviderUnavailable)
            }
            Err(error) => Err(error),
        }
    }

    /// Whether a connected profile needs a catalog fetch (missing or stale).
    pub(crate) fn catalog_needs_refresh(&self, store: &SqliteStore) -> Result<bool, PublicError> {
        let now = unix_now_ms().map_err(|_| PublicError::AgentStorageUnavailable)?;
        let snapshot = store
            .get_catalog_snapshot(PROVIDER_PROFILE_ID)
            .map_err(|_| PublicError::AgentStorageUnavailable)?;
        let Some(snapshot) = snapshot else {
            return Ok(true);
        };
        Ok(catalog_freshness(snapshot.state.retrieved_at_unix_ms, now)
            == tule_core::CatalogFreshness::Stale
            || snapshot.state.compatibility_revision != CATALOG_COMPATIBILITY_REVISION)
    }

    /// Public get-command path: refresh when missing/stale, and surface refresh
    /// failures instead of returning stale success.
    pub(crate) async fn load_connected_catalog(
        &self,
        store: &SqliteStore,
    ) -> Result<ProviderModelCatalogResponse, PublicError> {
        if self.catalog_needs_refresh(store)? {
            self.refresh_model_catalog(store, false).await
        } else {
            build_catalog_response(store)
        }
    }

    #[cfg(not(test))]
    async fn fetch_models_http(
        &self,
        headers: HeaderMap,
    ) -> Result<ModelsFetchResult, PublicError> {
        let response = self
            .client
            .get(MODELS_URL)
            .headers(headers)
            .send()
            .await
            .map_err(|_| PublicError::ProviderUnavailable)?;
        let status = response.status();
        let etag = response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        if status == StatusCode::NOT_MODIFIED {
            return Ok(ModelsFetchResult::NotModified { etag });
        }
        if status == StatusCode::UNAUTHORIZED {
            return Ok(ModelsFetchResult::Status(401));
        }
        if status == StatusCode::FORBIDDEN {
            return Ok(ModelsFetchResult::Status(403));
        }
        if status == StatusCode::TOO_MANY_REQUESTS {
            return Ok(ModelsFetchResult::Status(429));
        }
        if !status.is_success() {
            return Ok(ModelsFetchResult::Status(status.as_u16()));
        }
        let body = read_bounded_response_body(response, MAX_CATALOG_BODY).await?;
        let body = String::from_utf8(body).map_err(|_| PublicError::ProviderUnavailable)?;
        Ok(ModelsFetchResult::Models { body, etag })
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
        let previous_account = snapshot.value(CredentialKind::AccountId);
        let account_changed = previous_account != Some(token.account.as_bytes());
        if account_changed && store.seal_catalog_reads().is_err() {
            token.zeroize();
            return Err(PublicError::AgentStorageUnavailable);
        }
        let original_profile = profile.clone();
        profile.set_credential_metadata(None, None, now);
        if store.update_provider_profile(&profile).is_err() {
            if account_changed {
                let _ = store.clear_catalog_read_seal();
            }
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
                        account_changed,
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
                let restored = self.restore_persistence_state(
                    store,
                    &snapshot,
                    &written,
                    &original_profile,
                    account_changed,
                );
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
                            account_changed,
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
                        account_changed,
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
                let restored = self.restore_persistence_state(
                    store,
                    &snapshot,
                    &written,
                    &original_profile,
                    account_changed,
                );
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
            let restored = self.restore_persistence_state(
                store,
                &snapshot,
                &written,
                &original_profile,
                account_changed,
            );
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
        if account_changed {
            match store.invalidate_catalog_for_credential_change(PROVIDER_PROFILE_ID, now) {
                Ok(_) => {}
                Err(_) => {
                    // Credentials were written for the new account. Compensate back
                    // to the prior generation when possible; otherwise seal public
                    // reads independently of best-effort scrubbing.
                    if !self.restore_persistence_state(
                        store,
                        &snapshot,
                        &written,
                        &original_profile,
                        true,
                    ) {
                        let _ = store.scrub_catalog_entries(PROVIDER_PROFILE_ID);
                        self.reconnect_required.store(true, Ordering::SeqCst);
                        return Err(PublicError::CredentialStoreUnavailable);
                    }
                    return Err(PublicError::AgentStorageUnavailable);
                }
            }
        }
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
        self.ensure_fresh_access_locked(store, cancel).await
    }

    /// Access-token refresh path that assumes `profile_lock` is already held.
    async fn ensure_fresh_access_locked(
        &self,
        store: &SqliteStore,
        cancel: Option<&CancellationToken>,
    ) -> Result<(), PublicError> {
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

    fn restore_snapshot_and_catalog_access(
        &self,
        store: &SqliteStore,
        snapshot: &CredentialSnapshot,
        changed: &[CredentialKind],
    ) -> bool {
        self.restore_snapshot(snapshot, changed) && store.clear_catalog_read_seal().is_ok()
    }

    fn restore_persistence_state(
        &self,
        store: &SqliteStore,
        snapshot: &CredentialSnapshot,
        changed: &[CredentialKind],
        profile: &ProviderProfile,
        clear_catalog_read_seal: bool,
    ) -> bool {
        if !self.restore_snapshot(snapshot, changed) {
            return false;
        }
        if store.update_provider_profile(profile).is_err() {
            return false;
        }
        !clear_catalog_read_seal || store.clear_catalog_read_seal().is_ok()
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
        let original_profile = profile.clone();
        let now = unix_now_ms().map_err(|_| PublicError::AgentStorageUnavailable)?;
        let snapshot = self.credential_snapshot()?;
        store
            .seal_catalog_reads()
            .map_err(|_| PublicError::AgentStorageUnavailable)?;
        let mut deleted = Vec::new();
        for kind in DELETE_CREDENTIAL_ORDER {
            if let Err(error) = self.credentials.delete(CREDENTIAL_HANDLE, kind) {
                if restore_on_failure
                    && !self.restore_snapshot_and_catalog_access(store, &snapshot, &deleted)
                {
                    self.reconnect_required.store(true, Ordering::SeqCst);
                }
                return Err(map_credential_error(error));
            }
            deleted.push(kind);
            match self.credentials.read(CREDENTIAL_HANDLE, kind) {
                Ok(None) => {}
                Ok(Some(_)) | Err(_) => {
                    if restore_on_failure
                        && !self.restore_snapshot_and_catalog_access(store, &snapshot, &deleted)
                    {
                        self.reconnect_required.store(true, Ordering::SeqCst);
                    }
                    return Err(PublicError::CredentialStoreUnavailable);
                }
            }
        }

        profile.set_credential_metadata(None, None, now);
        if store.update_provider_profile(&profile).is_err() {
            if restore_on_failure
                && !self.restore_persistence_state(
                    store,
                    &snapshot,
                    &deleted,
                    &original_profile,
                    true,
                )
            {
                self.reconnect_required.store(true, Ordering::SeqCst);
                return Err(PublicError::CredentialStoreUnavailable);
            }
            return Err(PublicError::AgentStorageUnavailable);
        }
        if store
            .invalidate_catalog_for_credential_change(PROVIDER_PROFILE_ID, now)
            .is_err()
        {
            // Fail closed: never report clean disconnect while a prior catalog
            // generation may still be readable.
            if restore_on_failure
                && !self.restore_persistence_state(
                    store,
                    &snapshot,
                    &deleted,
                    &original_profile,
                    true,
                )
            {
                self.reconnect_required.store(true, Ordering::SeqCst);
                return Err(PublicError::CredentialStoreUnavailable);
            }
            if !restore_on_failure {
                self.reconnect_required.store(true, Ordering::SeqCst);
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

impl ProviderAdapter for XaiSubscriptionAdapter {
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
            let headers = match build_inference_headers(&access) {
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
                StatusCode::BAD_REQUEST => {
                    let body = tokio::select! {
                        _ = cancel.cancelled() => return Err(PublicError::Cancelled),
                        body = response.bytes() => body.map_err(|_| PublicError::ProviderUnavailable)?,
                    };
                    return Err(map_bad_request_body(&body));
                }
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
        MockInference::Status(400) => Err(PublicError::ProviderUnavailable),
        MockInference::Status(_) => Err(PublicError::ProviderUnavailable),
        MockInference::StatusBody { status, body } => match status {
            401 => Err(PublicError::AuthenticationRequired),
            403 => Err(PublicError::EntitlementUnavailable),
            429 => Err(PublicError::RateLimited),
            400 => Err(map_bad_request_body(body.as_bytes())),
            _ => Err(PublicError::ProviderUnavailable),
        },
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

/// Revisioned exact-id Effort capability for chat/completions `reasoning_effort`.
///
/// Fail closed for unknown model ids. Defaults encode documented provider
/// defaults for allowlisted models (`high` for `grok-4.5`; same default applied
/// to `grok-4.3` until a model-specific documented default differs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EffortCapability {
    pub(crate) default: AgentEffort,
}

/// Returns Effort capability when the exact model id is in the adapter allowlist.
#[must_use]
pub(crate) fn effort_capability_for_model(model_id: &str) -> Option<EffortCapability> {
    match model_id {
        "grok-4.3" | "grok-4.5" => Some(EffortCapability {
            default: AgentEffort::High,
        }),
        _ => None,
    }
}

/// Maps product Effort to the chat/completions `reasoning_effort` wire value.
#[must_use]
pub(crate) fn map_effort_to_reasoning_effort(effort: AgentEffort) -> &'static str {
    match effort {
        AgentEffort::Low => "low",
        AgentEffort::Medium => "medium",
        AgentEffort::High => "high",
    }
}

/// Resolves Effort for a send using the adapter capability table.
///
/// Returns `(effort_available, resolved_effort)`. Client-supplied values are
/// rejected when Effort is unavailable for the model.
pub(crate) fn resolve_effort_for_send(
    model_id: &str,
    client_effort: Option<&str>,
) -> Result<(bool, Option<AgentEffort>), PublicError> {
    match effort_capability_for_model(model_id) {
        None => {
            if client_effort.is_some() {
                return Err(PublicError::InvalidInput);
            }
            Ok((false, None))
        }
        Some(capability) => {
            let effort = match client_effort {
                None => capability.default,
                Some(value) => AgentEffort::parse(value).map_err(|_| PublicError::InvalidInput)?,
            };
            Ok((true, Some(effort)))
        }
    }
}

/// Assembles the deterministic xAI chat/completions JSON body.
///
/// Preserves the Phase 1 streaming wire contract: `model`, system/user/assistant
/// `messages` (system instruction included), `stream: true`, and streamed usage
/// reporting. When the frozen model is Effort-capable, includes mapped
/// `reasoning_effort`; otherwise omits that field. Does not emit Speed /
/// `service_tier` parameters.
pub(crate) fn assemble_chat_completions_request_json(
    context: &AgentRequestContext,
) -> Result<String, AgentContextError> {
    let mut body = String::from("{\"model\":");
    append_json_string(&mut body, &context.model_id);
    body.push_str(",\"messages\":[{\"role\":\"system\",\"content\":");
    append_json_string(&mut body, &context.instructions);
    body.push('}');

    for turn in &context.completed_history {
        let framed = format_turn_user_content(&turn.user_text, turn.source.as_ref());
        body.push_str(",{\"role\":\"user\",\"content\":");
        append_json_string(&mut body, &framed);
        body.push_str("},{\"role\":\"assistant\",\"content\":");
        append_json_string(&mut body, &turn.agent_text);
        body.push('}');
    }
    let current =
        format_turn_user_content(&context.current_user_text, context.current_source.as_ref());
    body.push_str(",{\"role\":\"user\",\"content\":");
    append_json_string(&mut body, &current);
    body.push_str("}],\"stream\":true,\"stream_options\":{\"include_usage\":true}");

    if let Some(capability) = effort_capability_for_model(&context.model_id) {
        let effort = context.effort.unwrap_or(capability.default);
        body.push_str(",\"reasoning_effort\":");
        append_json_string(&mut body, map_effort_to_reasoning_effort(effort));
    }
    body.push('}');

    if body.len() > MAX_CONTEXT_UTF8 {
        return Err(AgentContextError::ContextLimit {
            byte_count: body.len(),
        });
    }

    Ok(body)
}

fn append_json_string(output: &mut String, value: &str) {
    output.push('"');
    for ch in value.chars() {
        match ch {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\u{08}' => output.push_str("\\b"),
            '\u{0c}' => output.push_str("\\f"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            ch if (ch as u32) < 0x20 => {
                output.push_str(&format!("\\u{:04x}", ch as u32));
            }
            ch => output.push(ch),
        }
    }
    output.push('"');
}

fn build_inference_headers(access: &str) -> Result<HeaderMap, PublicError> {
    let mut headers = HeaderMap::with_capacity(4);
    let authorization = HeaderValue::from_str(&format!("Bearer {access}"))
        .map_err(|_| PublicError::AuthenticationRequired)?;
    headers.insert(AUTHORIZATION, authorization);
    headers.insert(ACCEPT, HeaderValue::from_static("text/event-stream"));
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(USER_AGENT, HeaderValue::from_static("tule-desktop/0.1.0"));
    Ok(headers)
}

fn positive_seconds_to_ms(value: Option<u64>, default_ms: u64) -> u64 {
    value
        .filter(|seconds| *seconds > 0)
        .map(|seconds| seconds.saturating_mul(1000))
        .unwrap_or(default_ms)
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
        .get("sub")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or(PublicError::ProviderUnavailable)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum SseProtocol {
    #[default]
    Unknown,
    ChatCompletions,
    Responses,
}

#[derive(Debug, Default)]
struct SseStreamState {
    protocol: SseProtocol,
    completed: bool,
    chat_completions: ChatCompletionsAccumulator,
}

impl SseStreamState {
    fn enter_protocol(&mut self, protocol: SseProtocol) -> Result<(), PublicError> {
        match self.protocol {
            SseProtocol::Unknown => {
                self.protocol = protocol;
                Ok(())
            }
            current if current == protocol => Ok(()),
            _ => Err(PublicError::ProviderUnavailable),
        }
    }
}

#[derive(Debug, Default)]
struct ChatCompletionsAccumulator {
    response_id: Option<String>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    saw_finish_reason: bool,
}

impl ChatCompletionsAccumulator {
    fn merge_metadata(&mut self, value: &Value) -> Result<(), PublicError> {
        self.merge_response_id(value.get("id"))?;
        self.merge_usage(value.get("usage"))
    }

    fn merge_response_id(&mut self, value: Option<&Value>) -> Result<(), PublicError> {
        let Some(value) = value else {
            return Ok(());
        };
        if value.is_null() {
            return Ok(());
        }
        let id = value.as_str().ok_or(PublicError::ProviderUnavailable)?;
        if id.is_empty() {
            return Ok(());
        }
        match &self.response_id {
            Some(current) if current != id => Err(PublicError::ProviderUnavailable),
            Some(_) => Ok(()),
            None => {
                self.response_id = Some(id.to_owned());
                Ok(())
            }
        }
    }

    fn merge_usage(&mut self, value: Option<&Value>) -> Result<(), PublicError> {
        let Some(value) = value else {
            return Ok(());
        };
        if value.is_null() {
            return Ok(());
        }
        let usage = value.as_object().ok_or(PublicError::ProviderUnavailable)?;
        Self::merge_token_total(&mut self.input_tokens, usage.get("prompt_tokens"))?;
        Self::merge_token_total(&mut self.output_tokens, usage.get("completion_tokens"))
    }

    fn merge_token_total(
        current: &mut Option<u64>,
        value: Option<&Value>,
    ) -> Result<(), PublicError> {
        let Some(value) = value else {
            return Ok(());
        };
        if value.is_null() {
            return Ok(());
        }
        *current = Some(value.as_u64().ok_or(PublicError::ProviderUnavailable)?);
        Ok(())
    }

    fn completed_event(&self) -> ProviderEvent {
        ProviderEvent::Completed {
            response_id: self.response_id.clone(),
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
        }
    }
}

async fn parse_sse_response(
    response: reqwest::Response,
    cancel: CancellationToken,
    on_event: &mut ProviderEventSink,
) -> Result<Vec<ProviderEvent>, PublicError> {
    let mut buffer = Vec::new();
    let mut state = SseStreamState::default();
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
            if cancel.is_cancelled() {
                return Err(PublicError::Cancelled);
            }
            if buffer.len().saturating_add(piece.len()) > MAX_SSE_BUFFER {
                return Err(PublicError::OutputLimit);
            }
            buffer.extend_from_slice(piece);
            emit_complete_sse_events(&mut buffer, &mut state, &cancel, on_event)?;
            if state.completed {
                return Ok(Vec::new());
            }
        }
    }
    if !buffer.is_empty() {
        if cancel.is_cancelled() {
            return Err(PublicError::Cancelled);
        }
        let mut batch = Vec::new();
        parse_sse_event(&buffer, &mut state, &mut batch)?;
        emit_provider_batch(batch, &mut state, &cancel, on_event)?;
    }
    finish_sse_stream(&mut state, &cancel, on_event)?;
    Ok(Vec::new())
}

#[cfg(test)]
async fn parse_sse_buffer(
    bytes: &[u8],
    cancel: CancellationToken,
    on_event: &mut ProviderEventSink,
) -> Result<Vec<ProviderEvent>, PublicError> {
    let fragments = bytes.chunks(4096).collect::<Vec<_>>();
    parse_sse_fragments(&fragments, cancel, on_event).await
}

#[cfg(test)]
async fn parse_sse_fragments(
    fragments: &[&[u8]],
    cancel: CancellationToken,
    on_event: &mut ProviderEventSink,
) -> Result<Vec<ProviderEvent>, PublicError> {
    if cancel.is_cancelled() {
        return Err(PublicError::Cancelled);
    }
    let mut buffer = Vec::new();
    let mut state = SseStreamState::default();
    for fragment in fragments {
        for piece in fragment.chunks(4096) {
            if cancel.is_cancelled() {
                return Err(PublicError::Cancelled);
            }
            if buffer.len().saturating_add(piece.len()) > MAX_SSE_BUFFER {
                return Err(PublicError::OutputLimit);
            }
            buffer.extend_from_slice(piece);
            emit_complete_sse_events(&mut buffer, &mut state, &cancel, on_event)?;
            if state.completed {
                return Ok(Vec::new());
            }
        }
    }
    if !buffer.is_empty() {
        if cancel.is_cancelled() {
            return Err(PublicError::Cancelled);
        }
        let mut batch = Vec::new();
        parse_sse_event(&buffer, &mut state, &mut batch)?;
        emit_provider_batch(batch, &mut state, &cancel, on_event)?;
    }
    finish_sse_stream(&mut state, &cancel, on_event)?;
    Ok(Vec::new())
}

fn emit_complete_sse_events(
    buffer: &mut Vec<u8>,
    state: &mut SseStreamState,
    cancel: &CancellationToken,
    on_event: &mut ProviderEventSink,
) -> Result<(), PublicError> {
    while !state.completed {
        let Some(boundary) = find_event_boundary(buffer) else {
            break;
        };
        if cancel.is_cancelled() {
            return Err(PublicError::Cancelled);
        }
        let event = buffer.drain(..boundary).collect::<Vec<_>>();
        let delimiter = event_delimiter_len(buffer);
        if delimiter > 0 {
            buffer.drain(..delimiter);
        }
        let mut batch = Vec::new();
        parse_sse_event(&event, state, &mut batch)?;
        emit_provider_batch(batch, state, cancel, on_event)?;
    }
    Ok(())
}

fn emit_provider_batch(
    batch: Vec<ProviderEvent>,
    state: &mut SseStreamState,
    cancel: &CancellationToken,
    on_event: &mut ProviderEventSink,
) -> Result<(), PublicError> {
    for item in batch {
        if cancel.is_cancelled() {
            return Err(PublicError::Cancelled);
        }
        match &item {
            ProviderEvent::Completed { .. } if state.completed => {
                return Err(PublicError::ProviderUnavailable);
            }
            ProviderEvent::Completed { .. } => state.completed = true,
            ProviderEvent::Delta(_) if state.completed => {
                return Err(PublicError::ProviderUnavailable);
            }
            ProviderEvent::Delta(_) => {}
        }
        on_event(item)?;
    }
    Ok(())
}

fn finish_sse_stream(
    state: &mut SseStreamState,
    cancel: &CancellationToken,
    on_event: &mut ProviderEventSink,
) -> Result<(), PublicError> {
    if state.completed {
        return Ok(());
    }
    if cancel.is_cancelled() {
        return Err(PublicError::Cancelled);
    }
    if state.protocol == SseProtocol::ChatCompletions && state.chat_completions.saw_finish_reason {
        return emit_provider_batch(
            vec![state.chat_completions.completed_event()],
            state,
            cancel,
            on_event,
        );
    }
    Err(PublicError::ProviderUnavailable)
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

fn parse_sse_event(
    event: &[u8],
    state: &mut SseStreamState,
    output: &mut Vec<ProviderEvent>,
) -> Result<(), PublicError> {
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
        state.enter_protocol(SseProtocol::ChatCompletions)?;
        output.push(state.chat_completions.completed_event());
        return Ok(());
    }
    let value: Value = serde_json::from_str(&data).map_err(|_| PublicError::ProviderUnavailable)?;
    if contains_unsupported_provider_output(&value) {
        return Err(PublicError::UnsupportedProviderOutput);
    }
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if kind.starts_with("response.") || kind == "error" {
        state.enter_protocol(SseProtocol::Responses)?;
    }
    // OpenAI-compatible chat/completions streaming (`choices[].delta.content`).
    if let Some(choices) = value.get("choices") {
        state.enter_protocol(SseProtocol::ChatCompletions)?;
        state.chat_completions.merge_metadata(&value)?;
        let choices = choices.as_array().ok_or(PublicError::ProviderUnavailable)?;
        let already_finished = state.chat_completions.saw_finish_reason;
        let mut frame_finished = false;
        for choice in choices {
            if choice
                .get("delta")
                .and_then(|delta| delta.get("tool_calls"))
                .is_some()
            {
                return Err(PublicError::UnsupportedProviderOutput);
            }
            if let Some(content) = choice
                .get("delta")
                .and_then(|delta| delta.get("content"))
                .and_then(Value::as_str)
                && !content.is_empty()
            {
                if already_finished || frame_finished {
                    return Err(PublicError::ProviderUnavailable);
                }
                output.push(ProviderEvent::Delta(content.to_owned()));
            }
            match choice.get("finish_reason") {
                None | Some(Value::Null) => {}
                Some(Value::String(reason)) if reason.is_empty() => {}
                Some(Value::String(_)) => frame_finished = true,
                Some(_) => return Err(PublicError::ProviderUnavailable),
            }
        }
        if frame_finished {
            state.chat_completions.saw_finish_reason = true;
        }
        return Ok(());
    }
    if value.get("usage").is_some() {
        state.enter_protocol(SseProtocol::ChatCompletions)?;
        state.chat_completions.merge_metadata(&value)?;
        return Ok(());
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

#[cfg(test)]
impl TokenValues {
    pub(crate) fn for_test(access: &str, refresh: &str, account: &str) -> Self {
        Self {
            access: access.into(),
            refresh: refresh.into(),
            account: account.into(),
            expires_at_unix_ms: Some(i64::MAX / 2),
            preserve_refresh: false,
        }
    }
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

#[derive(Debug, Clone)]
enum ModelsFetchResult {
    NotModified { etag: Option<String> },
    Models { body: String, etag: Option<String> },
    Status(u16),
}

fn build_catalog_headers(access: &str, etag: Option<&str>) -> Result<HeaderMap, PublicError> {
    let mut headers = HeaderMap::with_capacity(4);
    let authorization = HeaderValue::from_str(&format!("Bearer {access}"))
        .map_err(|_| PublicError::AuthenticationRequired)?;
    headers.insert(AUTHORIZATION, authorization);
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert(USER_AGENT, HeaderValue::from_static("tule-desktop/0.1.0"));
    if let Some(etag) = etag {
        let value = HeaderValue::from_str(etag).map_err(|_| PublicError::ProviderUnavailable)?;
        headers.insert(reqwest::header::IF_NONE_MATCH, value);
    }
    Ok(headers)
}

fn parse_catalog_body(body: &str) -> Result<Vec<tule_core::ModelCatalogEntry>, PublicError> {
    let value: Value = serde_json::from_str(body).map_err(|_| PublicError::ProviderUnavailable)?;
    let models = value
        .get("data")
        .or_else(|| value.get("models"))
        .and_then(Value::as_array)
        .ok_or(PublicError::ProviderUnavailable)?;
    let mut candidates = Vec::with_capacity(models.len());
    for (index, item) in models.iter().enumerate() {
        let Some(object) = item.as_object() else {
            continue;
        };
        let Some(model_id) = object
            .get("id")
            .or_else(|| object.get("slug"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        let display_name = object
            .get("display_name")
            .or_else(|| object.get("name"))
            .and_then(Value::as_str)
            .unwrap_or(model_id);
        let description = object
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let visibility = object
            .get("visibility")
            .and_then(Value::as_str)
            .unwrap_or("list")
            .to_owned();
        let input_modalities = object.get("input_modalities").and_then(|value| {
            value.as_array().map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
        });
        let sort_order = object
            .get("priority")
            .and_then(Value::as_i64)
            .unwrap_or(index as i64) as i32;
        let is_provider_default = object
            .get("is_default")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let tool_mode = object
            .get("tool_mode")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let use_responses_lite = object
            .get("use_responses_lite")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        candidates.push(CatalogCandidate {
            model_id: model_id.to_owned(),
            display_name: display_name.to_owned(),
            description,
            visibility,
            input_modalities,
            tool_mode,
            use_responses_lite,
            sort_order,
            is_provider_default,
        });
    }
    Ok(select_usable_catalog_entries(candidates))
}

/// Maps an HTTP 400 body to model rejection only for allowlisted provider signals.
fn map_bad_request_body(body: &[u8]) -> PublicError {
    if is_model_rejection_body(body) {
        PublicError::ModelUnavailable
    } else {
        PublicError::ProviderUnavailable
    }
}

fn is_model_rejection_body(body: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(body) else {
        return false;
    };
    let lower = text.to_ascii_lowercase();
    lower.contains("model_not_found")
        || lower.contains("model_not_available")
        || lower.contains("unsupported model")
        || lower.contains("\"code\":\"invalid_model\"")
        || lower.contains("is not supported when using")
        || (lower.contains("model") && lower.contains("is not supported"))
}

#[cfg(test)]
#[derive(Debug, Clone)]
pub(crate) enum MockInference {
    Events(Vec<ProviderEvent>),
    Sse(String),
    Status(u16),
    StatusBody { status: u16, body: String },
    WaitForCancellation,
}

#[cfg(test)]
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) enum MockModelsResponse {
    NotModified { etag: Option<String> },
    Models { body: String, etag: Option<String> },
    Status(u16),
}

#[cfg(test)]
impl From<MockModelsResponse> for ModelsFetchResult {
    fn from(value: MockModelsResponse) -> Self {
        match value {
            MockModelsResponse::NotModified { etag } => Self::NotModified { etag },
            MockModelsResponse::Models { body, etag } => Self::Models { body, etag },
            MockModelsResponse::Status(status) => Self::Status(status),
        }
    }
}

#[cfg(test)]
pub(crate) trait TestTransport: Send + Sync {
    fn request_device_code(&self, device_url: &str) -> Result<DeviceCodeResponse, PublicError> {
        let _ = device_url;
        Err(PublicError::ProviderUnavailable)
    }
    fn poll_device_code_token(
        &self,
        token_url: &str,
        device_code: &str,
    ) -> Result<TokenBundle, PublicError> {
        let _ = (token_url, device_code);
        Err(PublicError::ProviderUnavailable)
    }
    fn refresh_token(&self, token_url: &str, refresh: &str) -> Result<TokenBundle, PublicError>;
    fn inference(
        &self,
        inference_url: &str,
        headers: &HeaderMap,
        body: &str,
    ) -> Result<MockInference, PublicError>;
    fn models(
        &self,
        models_url: &str,
        headers: &HeaderMap,
        etag: Option<&str>,
    ) -> Result<MockModelsResponse, PublicError> {
        let _ = (models_url, headers, etag);
        Err(PublicError::ProviderUnavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::FakeCredentialStore;
    use reqwest::header::HeaderMap;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn production_contract_is_pinned() {
        assert_eq!(PROVIDER_ID, "xai-subscription-oauth");
        assert_eq!(MODEL, "grok-3");
        assert_eq!(PROVIDER_PROFILE_ID, PROVIDER_ID);
        assert_eq!(MODEL_ID, MODEL);
        assert_eq!(CLIENT_ID, "b1a00492-073a-47ea-816f-4c329264a828");
        assert_eq!(DEVICE_CODE_URL, "https://auth.x.ai/oauth2/device/code");
        assert_eq!(TOKEN_URL, "https://auth.x.ai/oauth2/token");
        assert_eq!(INFERENCE_URL, "https://api.x.ai/v1/chat/completions");
        assert_eq!(CATALOG_COMPATIBILITY_REVISION, "1.0.0");
        assert_eq!(MODELS_URL, "https://api.x.ai/v1/models");
        assert_eq!(
            SCOPES,
            "openid profile email offline_access grok-cli:access api:access"
        );
    }

    #[test]
    fn chat_completions_request_is_byte_for_byte_deterministic() {
        let context = tule_core::build_agent_request_context(
            &[tule_core::CompletedTurnContext {
                user_text: "Hello \"world\"\n".to_owned(),
                agent_text: "Reply\\path".to_owned(),
                source: None,
            }],
            "Next message",
            Some("Exact\ninstructions"),
            MODEL,
            None,
            None,
        )
        .unwrap();
        let json = assemble_chat_completions_request_json(&context).unwrap();
        let expected = format!(
            r#"{{"model":"grok-3","messages":[{{"role":"system","content":"{}\n\nSaved Project instructions:\n---\nExact\ninstructions\n---"}},{{"role":"user","content":"Hello \"world\"\n"}},{{"role":"assistant","content":"Reply\\path"}},{{"role":"user","content":"Next message"}}],"stream":true,"stream_options":{{"include_usage":true}}}}"#,
            tule_core::FIXED_INSTRUCTION
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n")
        );
        assert_eq!(json, expected);
        assert!(!json.contains("\"store\""));
        assert!(!json.contains("\"input\""));
        assert!(json.contains("\"stream\":true"));
        assert!(json.contains("\"stream_options\":{\"include_usage\":true}"));
        assert!(!json.contains("reasoning_effort"));
        assert!(!json.contains("service_tier"));
    }

    #[test]
    fn chat_completions_request_limit_counts_stream_usage_envelope() {
        let base = AgentRequestContext {
            model_id: "model".to_owned(),
            instructions: String::new(),
            completed_history: Vec::new(),
            current_user_text: String::new(),
            current_source: None,
            effort: None,
        };
        let base_len = assemble_chat_completions_request_json(&base).unwrap().len();
        let available = MAX_CONTEXT_UTF8.checked_sub(base_len).unwrap();

        let mut exact = base.clone();
        exact.current_user_text = "x".repeat(available);
        assert_eq!(
            assemble_chat_completions_request_json(&exact)
                .unwrap()
                .len(),
            MAX_CONTEXT_UTF8
        );

        exact.current_user_text.push('x');
        assert!(matches!(
            assemble_chat_completions_request_json(&exact),
            Err(AgentContextError::ContextLimit { byte_count })
                if byte_count == MAX_CONTEXT_UTF8 + 1
        ));
    }

    #[test]
    fn chat_completions_includes_system_instruction_without_project_block() {
        let context =
            tule_core::build_agent_request_context(&[], "Hello", None, "other-model", None, None)
                .unwrap();
        let json = assemble_chat_completions_request_json(&context).unwrap();
        assert!(!json.contains("Saved Project instructions"));
        assert!(json.contains(tule_core::FIXED_INSTRUCTION));
        assert!(json.contains("\"model\":\"other-model\""));
        assert!(json.contains("\"messages\""));
        assert!(json.contains("\"role\":\"system\""));
        assert!(json.contains("\"stream_options\":{\"include_usage\":true}"));
        assert!(!json.contains("reasoning_effort"));
    }

    #[test]
    fn effort_capable_model_maps_reasoning_effort_on_wire() {
        let context = tule_core::build_agent_request_context(
            &[],
            "Hello",
            None,
            "grok-4.5",
            None,
            Some(AgentEffort::Medium),
        )
        .unwrap();
        let json = assemble_chat_completions_request_json(&context).unwrap();
        assert!(json.contains("\"model\":\"grok-4.5\""));
        assert!(json.contains("\"reasoning_effort\":\"medium\""));
        assert!(json.contains("\"stream\":true"));
        assert!(json.contains("\"stream_options\":{\"include_usage\":true}"));
        assert!(!json.contains("service_tier"));

        let default_context =
            tule_core::build_agent_request_context(&[], "Hello", None, "grok-4.3", None, None)
                .unwrap();
        let default_json = assemble_chat_completions_request_json(&default_context).unwrap();
        assert!(default_json.contains("\"reasoning_effort\":\"high\""));
    }

    #[test]
    fn non_capable_model_omits_reasoning_effort_even_if_context_has_effort() {
        let context = tule_core::build_agent_request_context(
            &[],
            "Hello",
            None,
            "grok-3",
            None,
            Some(AgentEffort::High),
        )
        .unwrap();
        let json = assemble_chat_completions_request_json(&context).unwrap();
        assert!(!json.contains("reasoning_effort"));
        assert!(json.contains("\"stream_options\":{\"include_usage\":true}"));
        assert_eq!(
            resolve_effort_for_send("grok-3", Some("high")),
            Err(PublicError::InvalidInput)
        );
        assert_eq!(resolve_effort_for_send("grok-3", None), Ok((false, None)));
        assert_eq!(
            resolve_effort_for_send("grok-4.5", None),
            Ok((true, Some(AgentEffort::High)))
        );
        assert_eq!(
            resolve_effort_for_send("grok-4.5", Some("low")),
            Ok((true, Some(AgentEffort::Low)))
        );
        assert_eq!(
            resolve_effort_for_send("grok-4.5", Some("xhigh")),
            Err(PublicError::InvalidInput)
        );
        assert!(effort_capability_for_model("unknown-model").is_none());
    }

    #[test]
    fn tool_events_are_rejected() {
        for event in [
            r#"data: {"choices":[{"delta":{"tool_calls":[{"id":"call"}]}}]}"#,
            r#"data: {"type":"response.function_call"}"#,
            r#"data: {"type":"response.output_item.added","item":{"type":"function_call"}}"#,
        ] {
            let mut state = SseStreamState::default();
            let mut output = Vec::new();
            assert_eq!(
                parse_sse_event(event.as_bytes(), &mut state, &mut output),
                Err(PublicError::UnsupportedProviderOutput)
            );
        }
    }

    #[test]
    fn chat_completions_waits_for_done_and_keeps_empty_choices_usage() {
        let mut state = SseStreamState::default();
        let mut output = Vec::new();
        parse_sse_event(
            br#"data: {"id":"chatcmpl-1","choices":[{"delta":{"content":"hello"},"finish_reason":null}],"usage":null}"#,
            &mut state,
            &mut output,
        )
        .unwrap();
        assert!(matches!(
            output.as_slice(),
            [ProviderEvent::Delta(text)] if text == "hello"
        ));

        output.clear();
        parse_sse_event(
            br#"data: {"id":"chatcmpl-1","choices":[{"delta":{},"finish_reason":"stop"}],"usage":null}"#,
            &mut state,
            &mut output,
        )
        .unwrap();
        assert!(output.is_empty());
        assert!(state.chat_completions.saw_finish_reason);

        parse_sse_event(
            br#"data: {"id":"chatcmpl-1","choices":[],"usage":{"prompt_tokens":11,"completion_tokens":2,"total_tokens":13}}"#,
            &mut state,
            &mut output,
        )
        .unwrap();
        assert!(output.is_empty());

        parse_sse_event(b"data: [DONE]", &mut state, &mut output).unwrap();
        assert!(matches!(
            output.as_slice(),
            [ProviderEvent::Completed {
                response_id: Some(id),
                input_tokens: Some(11),
                output_tokens: Some(2),
            }] if id == "chatcmpl-1"
        ));
    }

    #[tokio::test]
    async fn chat_completions_documented_usage_shape_completes_once() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&events);
        let mut sink: ProviderEventSink = Box::new(move |event| {
            captured.lock().unwrap().push(event);
            Ok(())
        });
        parse_sse_buffer(
            b"data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"delta\":{\"content\":\"hello\"},\"finish_reason\":null}],\"usage\":null}\n\ndata: {\"id\":\"chatcmpl-1\",\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":null}\n\ndata: {\"id\":\"chatcmpl-1\",\"choices\":[],\"usage\":{\"prompt_tokens\":11,\"completion_tokens\":2,\"total_tokens\":13}}\n\ndata: [DONE]\n\n",
            CancellationToken::new(),
            &mut sink,
        )
        .await
        .unwrap();
        assert!(matches!(
            events.lock().unwrap().as_slice(),
            [
                ProviderEvent::Delta(text),
                ProviderEvent::Completed {
                    response_id: Some(id),
                    input_tokens: Some(11),
                    output_tokens: Some(2),
                }
            ] if text == "hello" && id == "chatcmpl-1"
        ));
    }

    #[tokio::test]
    async fn chat_completions_latest_usage_wins_at_clean_eof() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&events);
        let mut sink: ProviderEventSink = Box::new(move |event| {
            captured.lock().unwrap().push(event);
            Ok(())
        });
        parse_sse_buffer(
            b"data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"delta\":{\"content\":\"a\"},\"finish_reason\":null}],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":null}}\n\ndata: {\"id\":\"chatcmpl-1\",\"choices\":[{\"delta\":{\"content\":\"b\"},\"finish_reason\":null}],\"usage\":{\"prompt_tokens\":null,\"completion_tokens\":1}}\n\ndata: {\"id\":\"chatcmpl-1\",\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":null}}\n\n",
            CancellationToken::new(),
            &mut sink,
        )
        .await
        .unwrap();
        assert!(matches!(
            events.lock().unwrap().as_slice(),
            [
                ProviderEvent::Delta(first),
                ProviderEvent::Delta(second),
                ProviderEvent::Completed {
                    input_tokens: Some(7),
                    output_tokens: Some(1),
                    ..
                }
            ] if first == "a" && second == "b"
        ));
    }

    #[tokio::test]
    async fn chat_completions_null_usage_does_not_erase_prior_totals() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&events);
        let mut sink: ProviderEventSink = Box::new(move |event| {
            captured.lock().unwrap().push(event);
            Ok(())
        });
        parse_sse_buffer(
            b"data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"delta\":{},\"finish_reason\":null}],\"usage\":{\"prompt_tokens\":9,\"completion_tokens\":2}}\n\ndata: {\"id\":\"chatcmpl-1\",\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":null}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":null,\"completion_tokens\":null}}\n\ndata: [DONE]\n\n",
            CancellationToken::new(),
            &mut sink,
        )
        .await
        .unwrap();
        assert!(matches!(
            events.lock().unwrap().as_slice(),
            [ProviderEvent::Completed {
                input_tokens: Some(9),
                output_tokens: Some(2),
                ..
            }]
        ));

        let partial = Arc::new(Mutex::new(Vec::new()));
        let partial_captured = Arc::clone(&partial);
        let mut partial_sink: ProviderEventSink = Box::new(move |event| {
            partial_captured.lock().unwrap().push(event);
            Ok(())
        });
        parse_sse_buffer(
            b"data: {\"id\":\"chatcmpl-2\",\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":11}}\n\ndata: [DONE]\n\n",
            CancellationToken::new(),
            &mut partial_sink,
        )
        .await
        .unwrap();
        assert!(matches!(
            partial.lock().unwrap().as_slice(),
            [ProviderEvent::Completed {
                input_tokens: Some(11),
                output_tokens: None,
                ..
            }]
        ));
    }

    #[tokio::test]
    async fn chat_completions_done_only_and_incomplete_streams_keep_prior_behavior() {
        let mut discard: ProviderEventSink = Box::new(|_| Ok(()));
        let incomplete = parse_sse_buffer(
            b"data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"delta\":{\"content\":\"partial\"},\"finish_reason\":null}]}\n\n",
            CancellationToken::new(),
            &mut discard,
        )
        .await;
        assert_eq!(incomplete, Err(PublicError::ProviderUnavailable));

        let done_only = Arc::new(Mutex::new(Vec::new()));
        let done_only_cb = Arc::clone(&done_only);
        let mut capture_done: ProviderEventSink = Box::new(move |event| {
            done_only_cb.lock().unwrap().push(event);
            Ok(())
        });
        parse_sse_buffer(
            b"data: [DONE]\n\n",
            CancellationToken::new(),
            &mut capture_done,
        )
        .await
        .unwrap();
        assert!(matches!(
            done_only.lock().unwrap().as_slice(),
            [ProviderEvent::Completed { .. }]
        ));
    }

    #[test]
    fn chat_completions_rejects_malformed_metadata_and_late_content() {
        for invalid in [
            "[]",
            "{\"prompt_tokens\":\"3\"}",
            "{\"prompt_tokens\":-1}",
            "{\"prompt_tokens\":1.5}",
            "{\"prompt_tokens\":true}",
            "{\"prompt_tokens\":18446744073709551616}",
        ] {
            let event = format!("data: {{\"choices\":[],\"usage\":{invalid}}}");
            let mut state = SseStreamState::default();
            let mut output = Vec::new();
            assert_eq!(
                parse_sse_event(event.as_bytes(), &mut state, &mut output),
                Err(PublicError::ProviderUnavailable)
            );
        }

        let mut state = SseStreamState::default();
        let mut output = Vec::new();
        parse_sse_event(
            br#"data: {"id":"chatcmpl-1","choices":[{"delta":{},"finish_reason":"stop"}]}"#,
            &mut state,
            &mut output,
        )
        .unwrap();
        assert_eq!(
            parse_sse_event(
                br#"data: {"id":"chatcmpl-1","choices":[{"delta":{"content":"late"},"finish_reason":null}]}"#,
                &mut state,
                &mut output,
            ),
            Err(PublicError::ProviderUnavailable)
        );

        let mut same_frame_finish = SseStreamState::default();
        assert_eq!(
            parse_sse_event(
                br#"data: {"id":"chatcmpl-1","choices":[{"delta":{},"finish_reason":"stop"},{"delta":{"content":"late"},"finish_reason":null}]}"#,
                &mut same_frame_finish,
                &mut Vec::new(),
            ),
            Err(PublicError::ProviderUnavailable)
        );

        let mut malformed_finish = SseStreamState::default();
        assert_eq!(
            parse_sse_event(
                br#"data: {"choices":[{"delta":{},"finish_reason":1}]}"#,
                &mut malformed_finish,
                &mut Vec::new(),
            ),
            Err(PublicError::ProviderUnavailable)
        );
    }

    #[test]
    fn chat_completions_rejects_conflicting_ids_and_mixed_protocols() {
        let mut state = SseStreamState::default();
        parse_sse_event(
            br#"data: {"id":"chatcmpl-1","choices":[{"delta":{},"finish_reason":null}]}"#,
            &mut state,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(
            parse_sse_event(
                br#"data: {"id":"chatcmpl-2","choices":[],"usage":{"prompt_tokens":1}}"#,
                &mut state,
                &mut Vec::new(),
            ),
            Err(PublicError::ProviderUnavailable)
        );

        let mut mixed = SseStreamState::default();
        parse_sse_event(
            br#"data: {"type":"response.output_text.delta","delta":"hello"}"#,
            &mut mixed,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(
            parse_sse_event(
                br#"data: {"id":"chatcmpl-1","choices":[]}"#,
                &mut mixed,
                &mut Vec::new(),
            ),
            Err(PublicError::ProviderUnavailable)
        );
    }

    #[tokio::test]
    async fn chat_completions_cancellation_before_terminal_emits_no_completion() {
        let cancel = CancellationToken::new();
        let sink_cancel = cancel.clone();
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&events);
        let mut sink: ProviderEventSink = Box::new(move |event| {
            captured.lock().unwrap().push(event);
            sink_cancel.cancel();
            Ok(())
        });
        let result = parse_sse_buffer(
            b"data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"delta\":{\"content\":\"hello\"},\"finish_reason\":null}]}\n\ndata: {\"id\":\"chatcmpl-1\",\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":1}}\n\ndata: [DONE]\n\n",
            cancel,
            &mut sink,
        )
        .await;
        assert_eq!(result, Err(PublicError::Cancelled));
        assert!(matches!(
            events.lock().unwrap().as_slice(),
            [ProviderEvent::Delta(text)] if text == "hello"
        ));
    }

    #[tokio::test]
    async fn chat_completions_fragmented_utf8_and_delimiters_match_contiguous_stream() {
        let body = "data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"delta\":{\"content\":\"héllo\"},\"finish_reason\":null}]}\r\n\r\ndata: {\"id\":\"chatcmpl-1\",\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":1}}\n\ndata: [DONE]\n\n";
        let bytes = body.as_bytes();
        let multibyte = body.find('é').unwrap();
        let done = body.find("[DONE]").unwrap();
        let fragments = [
            &bytes[..2],
            &bytes[2..multibyte + 1],
            &bytes[multibyte + 1..done + 2],
            &bytes[done + 2..bytes.len() - 1],
            &bytes[bytes.len() - 1..],
        ];
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&events);
        let mut sink: ProviderEventSink = Box::new(move |event| {
            captured.lock().unwrap().push(event);
            Ok(())
        });
        parse_sse_fragments(&fragments, CancellationToken::new(), &mut sink)
            .await
            .unwrap();
        assert!(matches!(
            events.lock().unwrap().as_slice(),
            [
                ProviderEvent::Delta(text),
                ProviderEvent::Completed {
                    input_tokens: Some(5),
                    output_tokens: Some(1),
                    ..
                }
            ] if text == "héllo"
        ));
    }

    #[test]
    fn supersede_legacy_chatgpt_credentials_clears_retired_handle() {
        let fake = Arc::new(FakeCredentialStore::default());
        fake.replace(
            LEGACY_CHATGPT_CREDENTIAL_HANDLE,
            CredentialKind::AccessToken,
            b"legacy-access",
        )
        .unwrap();
        fake.replace(
            LEGACY_CHATGPT_CREDENTIAL_HANDLE,
            CredentialKind::RefreshToken,
            b"legacy-refresh",
        )
        .unwrap();
        fake.replace(
            CREDENTIAL_HANDLE,
            CredentialKind::AccessToken,
            b"xai-access",
        )
        .unwrap();

        let adapter = XaiSubscriptionAdapter::new(fake.clone());
        adapter.supersede_legacy_chatgpt_credentials();

        assert!(
            fake.peek(
                LEGACY_CHATGPT_CREDENTIAL_HANDLE,
                CredentialKind::AccessToken
            )
            .is_none()
        );
        assert!(
            fake.peek(
                LEGACY_CHATGPT_CREDENTIAL_HANDLE,
                CredentialKind::RefreshToken
            )
            .is_none()
        );
        assert_eq!(
            fake.peek(CREDENTIAL_HANDLE, CredentialKind::AccessToken)
                .as_deref(),
            Some(b"xai-access".as_slice())
        );
    }

    #[test]
    fn connection_status_requires_a_complete_credential_set_across_restart() {
        let fake = Arc::new(FakeCredentialStore::default());
        let dir = tempfile_dir();
        let store = SqliteStore::open(dir.join("tule.sqlite3")).unwrap();
        let adapter = XaiSubscriptionAdapter::new(fake.clone());
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
        let restarted = XaiSubscriptionAdapter::new(fake.clone());
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
        let committed_restart = XaiSubscriptionAdapter::new(fake);
        assert_eq!(
            committed_restart.connection_status_with_store(&store).state,
            ConnectionState::Connected
        );
    }

    #[test]
    fn cancel_connect_rejects_noop_and_duplicate_requests() {
        let adapter = XaiSubscriptionAdapter::new(Arc::new(FakeCredentialStore::default()));
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
            fn request_device_code(&self, _: &str) -> Result<DeviceCodeResponse, PublicError> {
                Ok(DeviceCodeResponse {
                    device_code: "device".into(),
                    user_code: "ABCD".into(),
                    verification_uri: "https://auth.x.ai/device".into(),
                    verification_uri_complete: None,
                    expires_in: Some(300),
                    interval: Some(5),
                })
            }
            fn poll_device_code_token(&self, _: &str, _: &str) -> Result<TokenBundle, PublicError> {
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
        let adapter = XaiSubscriptionAdapter::new(fake.clone());
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn active_browser_connect_cancellation_returns_cancelled_without_credentials() {
        struct DeviceCodeOnly;
        impl TestTransport for DeviceCodeOnly {
            fn request_device_code(&self, _: &str) -> Result<DeviceCodeResponse, PublicError> {
                Ok(DeviceCodeResponse {
                    device_code: "device".into(),
                    user_code: "ABCD".into(),
                    verification_uri: "https://auth.x.ai/device".into(),
                    verification_uri_complete: None,
                    expires_in: Some(300),
                    interval: Some(5),
                })
            }
            fn refresh_token(&self, _: &str, _: &str) -> Result<TokenBundle, PublicError> {
                unreachable!("refresh should not run during cancelled connect")
            }
            fn inference(
                &self,
                _: &str,
                _: &HeaderMap,
                _: &str,
            ) -> Result<MockInference, PublicError> {
                unreachable!("inference should not run during cancelled connect")
            }
        }

        let _guard = CONNECT_TEST_LOCK.lock().await;
        let fake = Arc::new(FakeCredentialStore::default());
        let adapter = Arc::new(XaiSubscriptionAdapter::new(fake.clone()));
        adapter.set_test_transport(Arc::new(DeviceCodeOnly));
        let dir = unique_connect_tempfile_dir("cancel");
        let store = Arc::new(SqliteStore::open(dir.path().join("tule.sqlite3")).unwrap());
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();

        let connect_adapter = Arc::clone(&adapter);
        let connect_store = Arc::clone(&store);
        let connect = tokio::spawn(async move {
            connect_adapter
                .connect_device_code(
                    connect_store,
                    move |_url| {
                        entered_tx.send(()).unwrap();
                        release_rx.recv().unwrap();
                        Ok(())
                    },
                    None,
                )
                .await
        });

        entered_rx.recv().expect("connect entered open_url");
        assert_eq!(adapter.cancel_connect(), Ok(()));
        release_tx.send(()).expect("release open_url");
        assert_eq!(connect.await.expect("join"), Err(PublicError::Cancelled));
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
    async fn cancellation_during_token_poll_does_not_persist_credentials() {
        struct CancelPoll(CancellationToken);
        impl TestTransport for CancelPoll {
            fn request_device_code(&self, _: &str) -> Result<DeviceCodeResponse, PublicError> {
                Ok(DeviceCodeResponse {
                    device_code: "device".into(),
                    user_code: "ABCD".into(),
                    verification_uri: "https://auth.x.ai/device".into(),
                    verification_uri_complete: None,
                    expires_in: Some(300),
                    interval: Some(5),
                })
            }
            fn poll_device_code_token(&self, _: &str, _: &str) -> Result<TokenBundle, PublicError> {
                self.0.cancel();
                Err(PublicError::Cancelled)
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
        let adapter = XaiSubscriptionAdapter::new(fake.clone());
        let cancel = CancellationToken::new();
        adapter.set_test_transport(Arc::new(CancelPoll(cancel.clone())));
        let dir = unique_connect_tempfile_dir("cancel-poll");
        let store = Arc::new(SqliteStore::open(dir.path().join("tule.sqlite3")).unwrap());
        assert!(matches!(
            adapter.connect_device_code(store, |_| Ok(()), None).await,
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
        let adapter = XaiSubscriptionAdapter::new(credentials.clone());
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
            let adapter = XaiSubscriptionAdapter::new(fake.clone());
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
        let adapter = XaiSubscriptionAdapter::new(fake.clone());
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
        let adapter = XaiSubscriptionAdapter::new(fake.clone());

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

        let restarted = XaiSubscriptionAdapter::new(fake);
        assert_eq!(
            restarted.connection_status_with_store(&store).state,
            ConnectionState::ReconnectRequired
        );
    }

    #[test]
    fn initial_connection_requires_refresh_but_rotation_may_preserve_it() {
        let fake = Arc::new(FakeCredentialStore::default());
        let adapter = XaiSubscriptionAdapter::new(fake.clone());
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
            let adapter = XaiSubscriptionAdapter::new(fake.clone());
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
        let adapter = XaiSubscriptionAdapter::new(fake);
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
    fn endpoint_constants_reject_drift_targets() {
        assert!(!DEVICE_CODE_URL.contains("api.x.ai/v1"));
        assert_eq!(TOKEN_URL, "https://auth.x.ai/oauth2/token");
        assert_eq!(INFERENCE_URL, "https://api.x.ai/v1/chat/completions");
        assert!(!SCOPES.contains("api.connectors"));
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
        let adapter = XaiSubscriptionAdapter::new(Arc::new(FakeCredentialStore::default()));
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
                assert_eq!(headers.len(), 4);
                assert_eq!(headers.get(AUTHORIZATION).unwrap(), "Bearer a");
                assert_eq!(headers.get(USER_AGENT).unwrap(), "tule-desktop/0.1.0");
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
                    request_json: "{\"model\":\"grok-3\"}".into(),
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
        let adapter = XaiSubscriptionAdapter::new(fake);
        struct InvalidGrant;
        impl TestTransport for InvalidGrant {
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
        let adapter = XaiSubscriptionAdapter::new(fake);
        let calls = Arc::new(AtomicUsize::new(0));
        struct RefreshOnce(Arc<AtomicUsize>);
        impl TestTransport for RefreshOnce {
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
        let adapter = XaiSubscriptionAdapter::new(fake.clone());
        let cancel = CancellationToken::new();
        struct CancelRefresh(CancellationToken);
        impl TestTransport for CancelRefresh {
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
        let adapter = XaiSubscriptionAdapter::new(fake);
        struct ManyEvents;
        impl TestTransport for ManyEvents {
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
        let adapter = XaiSubscriptionAdapter::new(fake);
        struct WaitTransport;
        impl TestTransport for WaitTransport {
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

    // Fixed device-code connect tests serialize on a single lock.
    static CONNECT_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    fn unique_connect_tempfile_dir(label: &str) -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix(&format!("tule-openai-connect-{label}-"))
            .tempdir()
            .expect("tempdir")
    }

    async fn complete_browser_connect(
        adapter: &XaiSubscriptionAdapter,
        store: Arc<SqliteStore>,
        _code: &str,
    ) -> ConnectionStatus {
        let _guard = CONNECT_TEST_LOCK.lock().await;
        adapter
            .connect_with_test_device_code(Arc::clone(&store))
            .await
            .expect("device connect should succeed")
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

    #[test]
    fn catalog_parser_allowlists_ordered_text_models_and_ignores_supported_in_api() {
        let body = r#"{
          "models": [
            {
              "slug": "hidden",
              "display_name": "Hidden",
              "visibility": "hide",
              "priority": 1,
              "supported_in_api": true,
              "input_modalities": ["text"]
            },
            {
              "slug": "image-only",
              "display_name": "Image",
              "visibility": "list",
              "priority": 2,
              "supported_in_api": true,
              "input_modalities": ["image"]
            },
            {
              "slug": "spark",
              "display_name": "Spark",
              "description": "subscription",
              "visibility": "list",
              "priority": 20,
              "supported_in_api": false,
              "input_modalities": ["text"]
            },
            {
              "slug": "gpt-5.5",
              "display_name": "GPT-5.5",
              "visibility": "list",
              "priority": 10,
              "supported_in_api": true,
              "is_default": true
            }
          ]
        }"#;
        let entries = parse_catalog_body(body).unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.model_id.as_str())
                .collect::<Vec<_>>(),
            vec!["gpt-5.5", "spark"]
        );
        assert!(entries[0].is_provider_default);
        assert_eq!(entries[1].description.as_deref(), Some("subscription"));
    }

    #[test]
    fn authenticated_empty_usable_catalog_is_an_error() {
        let body = r#"{"models":[{"slug":"hidden","display_name":"Hidden","visibility":"hide"}]}"#;
        let entries = parse_catalog_body(body).unwrap();
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn catalog_refresh_uses_etag_and_marks_stale_on_failure() {
        use std::sync::atomic::AtomicUsize;

        let fake = Arc::new(FakeCredentialStore::default());
        seed_connected(&fake);
        let adapter = XaiSubscriptionAdapter::new(fake);
        let dir = tempfile_dir();
        let store = SqliteStore::open(dir.join("catalog.sqlite3")).unwrap();
        mark_profile_connected(&store, Some(i64::MAX / 2));
        let body = r#"{
          "models": [
            {
              "slug": "gpt-5.5",
              "display_name": "GPT-5.5",
              "visibility": "list",
              "priority": 1,
              "supported_in_api": false
            },
            {
              "slug": "other",
              "display_name": "Other",
              "visibility": "list",
              "priority": 2
            }
          ]
        }"#;
        struct CatalogTransport {
            body: String,
            calls: Arc<AtomicUsize>,
        }
        impl TestTransport for CatalogTransport {
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
            fn models(
                &self,
                url: &str,
                headers: &HeaderMap,
                etag: Option<&str>,
            ) -> Result<MockModelsResponse, PublicError> {
                assert_eq!(url, MODELS_URL);
                assert_eq!(headers.get(USER_AGENT).unwrap(), "tule-desktop/0.1.0");
                let call = self.calls.fetch_add(1, Ordering::SeqCst);
                if call == 0 {
                    assert!(etag.is_none());
                    return Ok(MockModelsResponse::Models {
                        body: self.body.clone(),
                        etag: Some("\"v1\"".into()),
                    });
                }
                if call == 1 {
                    assert_eq!(etag, Some("\"v1\""));
                    return Ok(MockModelsResponse::NotModified {
                        etag: Some("\"v1\"".into()),
                    });
                }
                Err(PublicError::ProviderUnavailable)
            }
        }
        let calls = Arc::new(AtomicUsize::new(0));
        adapter.set_test_transport(Arc::new(CatalogTransport {
            body: body.into(),
            calls: Arc::clone(&calls),
        }));

        let first = adapter.refresh_model_catalog(&store, true).await.unwrap();
        assert_eq!(first.models.len(), 2);
        assert_eq!(first.freshness, "current");
        assert_eq!(first.compatibility_revision.as_deref(), Some("1.0.0"));

        let second = adapter.refresh_model_catalog(&store, true).await.unwrap();
        assert_eq!(second.models.len(), 2);
        assert_eq!(second.freshness, "current");

        let failed = adapter.refresh_model_catalog(&store, true).await;
        assert_eq!(failed, Err(PublicError::ProviderUnavailable));
        let stale = crate::provider::build_stale_catalog_response(&store).unwrap();
        assert_eq!(stale.models.len(), 2);
        assert_eq!(stale.freshness, "stale");
        assert_eq!(calls.load(Ordering::SeqCst), 3);

        let generation_before = store
            .current_credential_generation(PROVIDER_PROFILE_ID)
            .unwrap();
        adapter.disconnect(&store).unwrap();
        let generation_after = store
            .current_credential_generation(PROVIDER_PROFILE_ID)
            .unwrap();
        assert!(generation_after > generation_before);
        assert!(
            store
                .get_catalog_snapshot(PROVIDER_PROFILE_ID)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            store
                .get_model_selection(PROVIDER_PROFILE_ID)
                .unwrap()
                .selected_model_id
                .as_deref(),
            Some("grok-3")
        );
    }

    #[test]
    fn catalog_parser_excludes_tool_only_and_responses_lite() {
        let body = r#"{
          "models": [
            {
              "slug": "tool-only",
              "display_name": "Tool",
              "visibility": "list",
              "priority": 1,
              "tool_mode": "code_mode_only",
              "input_modalities": ["text"]
            },
            {
              "slug": "lite",
              "display_name": "Lite",
              "visibility": "list",
              "priority": 2,
              "use_responses_lite": true,
              "input_modalities": ["text"]
            },
            {
              "slug": "ordinary",
              "display_name": "Ordinary",
              "visibility": "list",
              "priority": 3,
              "supported_in_api": false,
              "mystery_field": {"nested": true},
              "input_modalities": ["text"]
            }
          ]
        }"#;
        let entries = parse_catalog_body(body).unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.model_id.as_str())
                .collect::<Vec<_>>(),
            vec!["ordinary"]
        );
    }

    #[tokio::test]
    async fn catalog_refresh_refreshes_expired_access_token() {
        use std::sync::atomic::AtomicUsize;

        let fake = Arc::new(FakeCredentialStore::default());
        seed_connected(&fake);
        let adapter = XaiSubscriptionAdapter::new(fake.clone());
        let dir = tempfile_dir();
        let store = SqliteStore::open(dir.join("expired-catalog.sqlite3")).unwrap();
        mark_profile_connected(&store, Some(0));
        let refresh_calls = Arc::new(AtomicUsize::new(0));
        struct ExpiredCatalogTransport {
            refresh_calls: Arc<AtomicUsize>,
        }
        impl TestTransport for ExpiredCatalogTransport {
            fn refresh_token(&self, _: &str, _: &str) -> Result<TokenBundle, PublicError> {
                self.refresh_calls.fetch_add(1, Ordering::SeqCst);
                Ok(TokenBundle::Success(TokenValues {
                    access: "fresh-access".into(),
                    refresh: "refresh".into(),
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
            fn models(
                &self,
                _: &str,
                headers: &HeaderMap,
                _: Option<&str>,
            ) -> Result<MockModelsResponse, PublicError> {
                assert_eq!(
                    headers.get(reqwest::header::AUTHORIZATION).unwrap(),
                    "Bearer fresh-access"
                );
                Ok(MockModelsResponse::Models {
                    body: r#"{"models":[{"slug":"gpt-5.5","display_name":"GPT-5.5","visibility":"list","priority":1}]}"#.into(),
                    etag: None,
                })
            }
        }
        adapter.set_test_transport(Arc::new(ExpiredCatalogTransport {
            refresh_calls: Arc::clone(&refresh_calls),
        }));
        let catalog = adapter.refresh_model_catalog(&store, true).await.unwrap();
        assert_eq!(catalog.models.len(), 1);
        assert_eq!(refresh_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn catalog_refresh_authenticated_empty_preserves_snapshot_and_errors() {
        let fake = Arc::new(FakeCredentialStore::default());
        seed_connected(&fake);
        let adapter = XaiSubscriptionAdapter::new(fake);
        let dir = tempfile_dir();
        let store = SqliteStore::open(dir.join("empty-catalog.sqlite3")).unwrap();
        mark_profile_connected(&store, Some(i64::MAX / 2));
        store
            .replace_catalog_snapshot(
                PROVIDER_PROFILE_ID,
                &StoredCatalogState {
                    credential_generation: 0,
                    compatibility_revision: CATALOG_COMPATIBILITY_REVISION.to_owned(),
                    etag: Some("\"v1\"".into()),
                    retrieved_at_unix_ms: 1,
                    updated_at_unix_ms: 1,
                },
                &[tule_core::ModelCatalogEntry {
                    model_id: "gpt-5.5".into(),
                    display_name: "GPT-5.5".into(),
                    description: None,
                    sort_order: 1,
                    is_provider_default: true,
                }],
            )
            .unwrap();
        struct EmptyTransport;
        impl TestTransport for EmptyTransport {
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
            fn models(
                &self,
                _: &str,
                _: &HeaderMap,
                _: Option<&str>,
            ) -> Result<MockModelsResponse, PublicError> {
                Ok(MockModelsResponse::Models {
                    body: r#"{"models":[{"slug":"hidden","display_name":"Hidden","visibility":"hide"}]}"#.into(),
                    etag: None,
                })
            }
        }
        adapter.set_test_transport(Arc::new(EmptyTransport));
        assert_eq!(
            adapter.refresh_model_catalog(&store, true).await,
            Err(PublicError::ProviderUnavailable)
        );
        assert_eq!(
            store
                .get_catalog_snapshot(PROVIDER_PROFILE_ID)
                .unwrap()
                .unwrap()
                .entries[0]
                .model_id,
            "gpt-5.5"
        );
    }

    #[test]
    fn disconnect_catalog_invalidation_failure_is_not_clean_disconnect() {
        let fake = Arc::new(FakeCredentialStore::default());
        seed_connected(&fake);
        let adapter = XaiSubscriptionAdapter::new(fake.clone());
        let dir = tempfile_dir();
        let store = SqliteStore::open(dir.join("disconnect-fail.sqlite3")).unwrap();
        mark_profile_connected(&store, Some(i64::MAX / 2));
        store
            .replace_catalog_snapshot(
                PROVIDER_PROFILE_ID,
                &StoredCatalogState {
                    credential_generation: 3,
                    compatibility_revision: CATALOG_COMPATIBILITY_REVISION.to_owned(),
                    etag: None,
                    retrieved_at_unix_ms: 1,
                    updated_at_unix_ms: 1,
                },
                &[tule_core::ModelCatalogEntry {
                    model_id: "gpt-5.5".into(),
                    display_name: "GPT-5.5".into(),
                    description: None,
                    sort_order: 1,
                    is_provider_default: true,
                }],
            )
            .unwrap();
        store.set_fail_catalog_invalidation(true);
        assert_eq!(
            adapter.disconnect(&store),
            Err(PublicError::AgentStorageUnavailable)
        );
        assert_ne!(
            adapter.connection_status_with_store(&store).state,
            ConnectionState::Disconnected
        );
        assert!(
            store
                .get_catalog_snapshot(PROVIDER_PROFILE_ID)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn account_change_catalog_invalidation_failure_compensates_prior_account() {
        let fake = Arc::new(FakeCredentialStore::default());
        seed_connected(&fake);
        let adapter = XaiSubscriptionAdapter::new(fake.clone());
        let dir = tempfile_dir();
        let store = SqliteStore::open(dir.join("account-fail.sqlite3")).unwrap();
        mark_profile_connected(&store, Some(i64::MAX / 2));
        store
            .replace_catalog_snapshot(
                PROVIDER_PROFILE_ID,
                &StoredCatalogState {
                    credential_generation: 2,
                    compatibility_revision: CATALOG_COMPATIBILITY_REVISION.to_owned(),
                    etag: None,
                    retrieved_at_unix_ms: 1,
                    updated_at_unix_ms: 1,
                },
                &[tule_core::ModelCatalogEntry {
                    model_id: "prior-account-model".into(),
                    display_name: "Prior".into(),
                    description: None,
                    sort_order: 1,
                    is_provider_default: false,
                }],
            )
            .unwrap();
        store.set_fail_catalog_invalidation(true);
        assert_eq!(
            adapter.persist_tokens(
                &store,
                token_values("access-2", "refresh-2", "account-2", false),
                TokenPersistence::Rotation,
                PersistenceCancellation::None,
            ),
            Err(PublicError::AgentStorageUnavailable)
        );
        // Compensated back to the original committed account.
        assert_eq!(
            fake.peek(CREDENTIAL_HANDLE, CredentialKind::AccountId)
                .as_deref(),
            Some(b"account".as_slice())
        );
        assert_eq!(
            fake.peek(CREDENTIAL_HANDLE, CredentialKind::AccessToken)
                .as_deref(),
            Some(b"access".as_slice())
        );
        let public = crate::provider::build_catalog_response(&store).unwrap();
        assert_eq!(
            public
                .models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            vec!["prior-account-model"]
        );
        assert_ne!(
            fake.peek(CREDENTIAL_HANDLE, CredentialKind::AccountId)
                .as_deref(),
            Some(b"account-2".as_slice())
        );
    }

    #[test]
    fn account_change_invalidation_and_restore_failure_scrubs_prior_catalog() {
        let fake = Arc::new(FakeCredentialStore::default());
        seed_connected(&fake);
        let dir = tempfile_dir();
        let store = SqliteStore::open(dir.join("account-scrub.sqlite3")).unwrap();
        mark_profile_connected(&store, Some(i64::MAX / 2));
        store
            .replace_catalog_snapshot(
                PROVIDER_PROFILE_ID,
                &StoredCatalogState {
                    credential_generation: 2,
                    compatibility_revision: CATALOG_COMPATIBILITY_REVISION.to_owned(),
                    etag: None,
                    retrieved_at_unix_ms: 1,
                    updated_at_unix_ms: 1,
                },
                &[tule_core::ModelCatalogEntry {
                    model_id: "prior-account-model".into(),
                    display_name: "Prior".into(),
                    description: None,
                    sort_order: 1,
                    is_provider_default: false,
                }],
            )
            .unwrap();
        store.set_fail_catalog_invalidation(true);
        // Snapshot reads are ops 1-3, credential writes 4-6, restore replaces 7-9.
        fake.reset_operations();
        fake.fail_on_operations([7, 8, 9]);
        let adapter = XaiSubscriptionAdapter::new(fake.clone());
        assert_eq!(
            adapter.persist_tokens(
                &store,
                token_values("access-2", "refresh-2", "account-2", false),
                TokenPersistence::Rotation,
                PersistenceCancellation::None,
            ),
            Err(PublicError::CredentialStoreUnavailable)
        );
        let public = crate::provider::build_catalog_response(&store).unwrap();
        assert!(
            public.models.is_empty(),
            "unrestored new-account commit must not observe prior models"
        );
        assert!(
            !public
                .models
                .iter()
                .any(|model| model.id == "prior-account-model")
        );
    }

    #[test]
    fn account_change_scrub_failure_seals_public_reads_across_restart() {
        let fake = Arc::new(FakeCredentialStore::default());
        seed_connected(&fake);
        let dir = tempfile_dir();
        let path = dir.join("account-seal.sqlite3");
        let store = SqliteStore::open(&path).unwrap();
        mark_profile_connected(&store, Some(i64::MAX / 2));
        store
            .replace_catalog_snapshot(
                PROVIDER_PROFILE_ID,
                &StoredCatalogState {
                    credential_generation: 2,
                    compatibility_revision: CATALOG_COMPATIBILITY_REVISION.to_owned(),
                    etag: None,
                    retrieved_at_unix_ms: 1,
                    updated_at_unix_ms: 1,
                },
                &[tule_core::ModelCatalogEntry {
                    model_id: "prior-account-model".into(),
                    display_name: "Prior".into(),
                    description: None,
                    sort_order: 1,
                    is_provider_default: false,
                }],
            )
            .unwrap();
        store
            .set_model_selection(PROVIDER_PROFILE_ID, Some("prior-account-model"), 1)
            .unwrap();
        store.set_fail_catalog_invalidation(true);
        store.set_fail_catalog_scrub(true);
        fake.reset_operations();
        fake.fail_on_operations([7, 8, 9]);
        let adapter = XaiSubscriptionAdapter::new(fake);
        assert_eq!(
            adapter.persist_tokens(
                &store,
                token_values("access-2", "refresh-2", "account-2", false),
                TokenPersistence::Rotation,
                PersistenceCancellation::None,
            ),
            Err(PublicError::CredentialStoreUnavailable)
        );
        assert_eq!(
            adapter.connection_status_with_store(&store).state,
            ConnectionState::ReconnectRequired
        );
        assert!(store.catalog_reads_are_sealed().unwrap());
        // Scrub failed, so sqlite still holds prior-generation rows.
        assert_eq!(
            store
                .get_catalog_snapshot(PROVIDER_PROFILE_ID)
                .unwrap()
                .unwrap()
                .entries[0]
                .model_id,
            "prior-account-model"
        );
        // Public reads remain sealed independently of scrub success in-process.
        let public = crate::provider::build_catalog_response(&store).unwrap();
        assert!(public.models.is_empty());
        let selection = crate::provider::build_selection_response(&store).unwrap();
        assert!(selection.selected_model_id.is_none());

        drop(store);
        let reopened = SqliteStore::open(&path).unwrap();
        assert!(reopened.catalog_reads_are_sealed().unwrap());
        assert_eq!(
            reopened
                .get_catalog_snapshot(PROVIDER_PROFILE_ID)
                .unwrap()
                .unwrap()
                .entries[0]
                .model_id,
            "prior-account-model"
        );
        let public = crate::provider::build_catalog_response(&reopened).unwrap();
        assert!(public.models.is_empty());
        let selection = crate::provider::build_selection_response(&reopened).unwrap();
        assert!(selection.selected_model_id.is_none());
    }

    #[tokio::test]
    async fn connected_catalog_get_surfaces_refresh_error_while_preserving_stale() {
        let fake = Arc::new(FakeCredentialStore::default());
        seed_connected(&fake);
        let adapter = XaiSubscriptionAdapter::new(fake);
        let dir = tempfile_dir();
        let store = SqliteStore::open(dir.join("stale-get.sqlite3")).unwrap();
        mark_profile_connected(&store, Some(i64::MAX / 2));
        store
            .replace_catalog_snapshot(
                PROVIDER_PROFILE_ID,
                &StoredCatalogState {
                    credential_generation: 0,
                    compatibility_revision: CATALOG_COMPATIBILITY_REVISION.to_owned(),
                    etag: Some("\"v1\"".into()),
                    retrieved_at_unix_ms: 1,
                    updated_at_unix_ms: 1,
                },
                &[tule_core::ModelCatalogEntry {
                    model_id: "gpt-5.5".into(),
                    display_name: "GPT-5.5".into(),
                    description: None,
                    sort_order: 1,
                    is_provider_default: true,
                }],
            )
            .unwrap();
        struct FailTransport;
        impl TestTransport for FailTransport {
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
            fn models(
                &self,
                _: &str,
                _: &HeaderMap,
                _: Option<&str>,
            ) -> Result<MockModelsResponse, PublicError> {
                Err(PublicError::ProviderUnavailable)
            }
        }
        adapter.set_test_transport(Arc::new(FailTransport));
        assert_eq!(
            adapter.load_connected_catalog(&store).await,
            Err(PublicError::ProviderUnavailable)
        );
        let stale = crate::provider::build_stale_catalog_response(&store).unwrap();
        assert_eq!(stale.models.len(), 1);
        assert_eq!(stale.freshness, "stale");
        assert_eq!(stale.models[0].id, "gpt-5.5");
    }

    #[tokio::test]
    async fn inference_distinguishes_model_rejection_from_unrelated_bad_request() {
        let fake = Arc::new(FakeCredentialStore::default());
        seed_connected(&fake);
        let adapter = XaiSubscriptionAdapter::new(fake);
        let dir = tempfile_dir();
        let store = SqliteStore::open(dir.join("reject.sqlite3")).unwrap();
        mark_profile_connected(&store, Some(i64::MAX / 2));

        struct RejectTransport;
        impl TestTransport for RejectTransport {
            fn refresh_token(&self, _: &str, _: &str) -> Result<TokenBundle, PublicError> {
                unreachable!()
            }
            fn inference(
                &self,
                _: &str,
                _: &HeaderMap,
                _: &str,
            ) -> Result<MockInference, PublicError> {
                Ok(MockInference::StatusBody {
                    status: 400,
                    body: "The 'bad-model' model is not supported when using Codex with a ChatGPT account.".into(),
                })
            }
        }
        adapter.set_test_transport(Arc::new(RejectTransport));
        let rejected = adapter
            .stream(
                ProviderRequest {
                    session_id: "s".into(),
                    request_json: "{}".into(),
                },
                CancellationToken::new(),
                Box::new(|_| Ok(())),
            )
            .await;
        assert_eq!(rejected, Err(PublicError::ModelUnavailable));

        struct UnrelatedTransport;
        impl TestTransport for UnrelatedTransport {
            fn refresh_token(&self, _: &str, _: &str) -> Result<TokenBundle, PublicError> {
                unreachable!()
            }
            fn inference(
                &self,
                _: &str,
                _: &HeaderMap,
                _: &str,
            ) -> Result<MockInference, PublicError> {
                Ok(MockInference::StatusBody {
                    status: 400,
                    body: "invalid request schema".into(),
                })
            }
        }
        adapter.set_test_transport(Arc::new(UnrelatedTransport));
        let unrelated = adapter
            .stream(
                ProviderRequest {
                    session_id: "s".into(),
                    request_json: "{}".into(),
                },
                CancellationToken::new(),
                Box::new(|_| Ok(())),
            )
            .await;
        assert_eq!(unrelated, Err(PublicError::ProviderUnavailable));
        let _ = store;
    }

    #[tokio::test]
    async fn model_rejection_recovery_with_refresh_failure_keeps_identifier_unavailable() {
        let fake = Arc::new(FakeCredentialStore::default());
        seed_connected(&fake);
        let adapter = XaiSubscriptionAdapter::new(fake);
        let dir = tempfile_dir();
        let store = SqliteStore::open(dir.join("reject-stale.sqlite3")).unwrap();
        mark_profile_connected(&store, Some(i64::MAX / 2));
        store
            .replace_catalog_snapshot(
                PROVIDER_PROFILE_ID,
                &StoredCatalogState {
                    credential_generation: 0,
                    compatibility_revision: CATALOG_COMPATIBILITY_REVISION.to_owned(),
                    etag: None,
                    retrieved_at_unix_ms: 1,
                    updated_at_unix_ms: 1,
                },
                &[
                    tule_core::ModelCatalogEntry {
                        model_id: "bad-model".into(),
                        display_name: "Bad".into(),
                        description: None,
                        sort_order: 1,
                        is_provider_default: false,
                    },
                    tule_core::ModelCatalogEntry {
                        model_id: "good-model".into(),
                        display_name: "Good".into(),
                        description: None,
                        sort_order: 2,
                        is_provider_default: true,
                    },
                ],
            )
            .unwrap();
        crate::provider::persist_model_selection(&store, "bad-model").unwrap();

        struct RejectThenFailRefresh;
        impl TestTransport for RejectThenFailRefresh {
            fn refresh_token(&self, _: &str, _: &str) -> Result<TokenBundle, PublicError> {
                unreachable!()
            }
            fn inference(
                &self,
                _: &str,
                _: &HeaderMap,
                _: &str,
            ) -> Result<MockInference, PublicError> {
                Ok(MockInference::StatusBody {
                    status: 400,
                    body: "unsupported model bad-model".into(),
                })
            }
            fn models(
                &self,
                _: &str,
                _: &HeaderMap,
                _: Option<&str>,
            ) -> Result<MockModelsResponse, PublicError> {
                Err(PublicError::RateLimited)
            }
        }
        adapter.set_test_transport(Arc::new(RejectThenFailRefresh));
        assert_eq!(
            adapter
                .stream(
                    ProviderRequest {
                        session_id: "s".into(),
                        request_json: "{}".into(),
                    },
                    CancellationToken::new(),
                    Box::new(|_| Ok(())),
                )
                .await,
            Err(PublicError::ModelUnavailable)
        );

        let (catalog, selection) =
            crate::provider::apply_model_rejection(&store, "bad-model").unwrap();
        assert!(selection.requires_selection);
        assert!(selection.selected_model_id.is_none());
        assert!(!catalog.models.iter().any(|model| model.id == "bad-model"));
        assert_eq!(
            adapter.refresh_model_catalog(&store, true).await,
            Err(PublicError::RateLimited)
        );
        let stale = crate::provider::build_stale_catalog_response(&store).unwrap();
        assert_eq!(stale.freshness, "stale");
        assert!(!stale.models.iter().any(|model| model.id == "bad-model"));
        assert!(stale.models.iter().any(|model| model.id == "good-model"));
        assert_eq!(
            crate::provider::validate_new_session_model(&store, "bad-model"),
            Err(PublicError::ModelUnavailable)
        );
    }

    #[tokio::test]
    async fn refresh_rejects_catalog_containing_only_locally_rejected_models() {
        let fake = Arc::new(FakeCredentialStore::default());
        seed_connected(&fake);
        let adapter = XaiSubscriptionAdapter::new(fake);
        let dir = tempfile_dir();
        let store = SqliteStore::open(dir.join("reject-only-refresh.sqlite3")).unwrap();
        mark_profile_connected(&store, Some(i64::MAX / 2));
        store
            .replace_catalog_snapshot(
                PROVIDER_PROFILE_ID,
                &StoredCatalogState {
                    credential_generation: 0,
                    compatibility_revision: CATALOG_COMPATIBILITY_REVISION.to_owned(),
                    etag: None,
                    retrieved_at_unix_ms: 1,
                    updated_at_unix_ms: 1,
                },
                &[tule_core::ModelCatalogEntry {
                    model_id: "good-model".into(),
                    display_name: "Good".into(),
                    description: None,
                    sort_order: 1,
                    is_provider_default: true,
                }],
            )
            .unwrap();
        store
            .record_rejected_model(PROVIDER_PROFILE_ID, "bad-model", 1)
            .unwrap();
        struct RejectedOnlyTransport;
        impl TestTransport for RejectedOnlyTransport {
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
            fn models(
                &self,
                _: &str,
                _: &HeaderMap,
                _: Option<&str>,
            ) -> Result<MockModelsResponse, PublicError> {
                Ok(MockModelsResponse::Models {
                    body: r#"{"models":[{"slug":"bad-model","display_name":"Bad","visibility":"list","priority":1,"input_modalities":["text"]}]}"#.into(),
                    etag: Some("\"rej\"".into()),
                })
            }
        }
        adapter.set_test_transport(Arc::new(RejectedOnlyTransport));
        assert_eq!(
            adapter.refresh_model_catalog(&store, true).await,
            Err(PublicError::ProviderUnavailable)
        );
        let preserved = crate::provider::build_catalog_response(&store).unwrap();
        assert_eq!(
            preserved
                .models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            vec!["good-model"]
        );
    }

    #[tokio::test]
    async fn refresh_persists_mixed_catalog_without_locally_rejected_models() {
        let fake = Arc::new(FakeCredentialStore::default());
        seed_connected(&fake);
        let adapter = XaiSubscriptionAdapter::new(fake);
        let dir = tempfile_dir();
        let store = SqliteStore::open(dir.join("reject-mixed-refresh.sqlite3")).unwrap();
        mark_profile_connected(&store, Some(i64::MAX / 2));
        store
            .record_rejected_model(PROVIDER_PROFILE_ID, "bad-model", 1)
            .unwrap();
        struct MixedTransport;
        impl TestTransport for MixedTransport {
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
            fn models(
                &self,
                _: &str,
                _: &HeaderMap,
                _: Option<&str>,
            ) -> Result<MockModelsResponse, PublicError> {
                Ok(MockModelsResponse::Models {
                    body: r#"{
                      "models": [
                        {"slug":"bad-model","display_name":"Bad","visibility":"list","priority":1,"input_modalities":["text"]},
                        {"slug":"good-model","display_name":"Good","visibility":"list","priority":2,"input_modalities":["text"]}
                      ]
                    }"#
                    .into(),
                    etag: Some("\"mix\"".into()),
                })
            }
        }
        adapter.set_test_transport(Arc::new(MixedTransport));
        let catalog = adapter.refresh_model_catalog(&store, true).await.unwrap();
        assert_eq!(
            catalog
                .models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            vec!["good-model"]
        );
        assert!(!catalog.models.iter().any(|model| model.id == "bad-model"));
    }
}
