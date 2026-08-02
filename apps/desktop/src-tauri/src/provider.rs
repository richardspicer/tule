//! Provider-neutral native contracts. Provider implementations never expose secrets to IPC.

use std::{future::Future, pin::Pin};

use serde::Serialize;
use tokio_util::sync::CancellationToken;

/// Every error which may cross the native IPC boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PublicError {
    NotConnected,
    InvalidInput,
    ContextLimit,
    SessionBusy,
    AuthenticationRequired,
    EntitlementUnavailable,
    RateLimited,
    ProviderUnavailable,
    UnsupportedProviderOutput,
    OutputLimit,
    Cancelled,
    #[allow(dead_code)]
    Interrupted,
    CredentialStoreUnavailable,
    AgentStorageUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    ReconnectRequired,
    UnavailableInThisBuild,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConnectionStatus {
    pub(crate) state: ConnectionState,
    pub(crate) provider_id: &'static str,
    pub(crate) model: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderRequest {
    pub(crate) session_id: String,
    pub(crate) request_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProviderEvent {
    Delta(String),
    Completed {
        response_id: Option<String>,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    },
}

pub(crate) type ProviderFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<ProviderEvent>, PublicError>> + Send + 'a>>;

/// Sink for ordered provider events during an in-flight stream.
pub(crate) type ProviderEventSink = Box<dyn FnMut(ProviderEvent) -> Result<(), PublicError> + Send>;

/// A replaceable adapter boundary. Implementations own all HTTP and credential access.
pub(crate) trait ProviderAdapter: Send + Sync {
    fn connection_status(&self) -> ConnectionStatus;
    fn stream<'a>(
        &'a self,
        request: ProviderRequest,
        cancel: CancellationToken,
        on_event: ProviderEventSink,
    ) -> ProviderFuture<'a>;
}

#[cfg(test)]
#[allow(dead_code)]
pub(crate) struct FakeProvider {
    status: ConnectionStatus,
    result: Result<Vec<ProviderEvent>, PublicError>,
}

#[cfg(test)]
#[allow(dead_code)]
impl FakeProvider {
    pub(crate) fn new(
        status: ConnectionStatus,
        result: Result<Vec<ProviderEvent>, PublicError>,
    ) -> Self {
        Self { status, result }
    }
}

#[cfg(test)]
impl ProviderAdapter for FakeProvider {
    fn connection_status(&self) -> ConnectionStatus {
        self.status.clone()
    }
    fn stream<'a>(
        &'a self,
        _: ProviderRequest,
        cancel: CancellationToken,
        mut on_event: ProviderEventSink,
    ) -> ProviderFuture<'a> {
        let result = self.result.clone();
        Box::pin(async move {
            if cancel.is_cancelled() {
                return Err(PublicError::Cancelled);
            }
            match result {
                Ok(events) => {
                    for event in events {
                        if cancel.is_cancelled() {
                            return Err(PublicError::Cancelled);
                        }
                        on_event(event)?;
                    }
                    Ok(Vec::new())
                }
                Err(error) => Err(error),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_errors_serialize_without_internal_detail() {
        assert_eq!(
            serde_json::to_string(&PublicError::UnsupportedProviderOutput).unwrap(),
            "\"unsupported_provider_output\""
        );
    }
}
