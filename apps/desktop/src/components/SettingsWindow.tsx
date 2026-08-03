import { useEffect, useId, useState } from "react";
import {
  cancelChatgptConnect,
  connectChatgpt,
  disconnectChatgpt,
  formatModelLabel,
  getConnectionStatus,
  getProviderModelCatalog,
  getProviderModelSelection,
  isStaleConnectCancellation,
  listenProviderModelCatalogChanged,
  listenProviderModelSelectionChanged,
  refreshProviderModelCatalog,
  setProviderModelSelection,
  type ConnectionState,
  type ProviderModelCatalog,
  type ProviderModelSelection,
} from "../platform/provider";
import {
  listenConnectionStatusChanged,
  listenSettingsNavigate,
  takeSettingsLaunchCategory,
  type SettingsCategory,
} from "../platform/settings";
import { getAgentErrorCode, getSafeAgentErrorMessage } from "../platform/agents";
import {
  applyThemePreference,
  listenAppearanceChanged,
  loadThemePreference,
  saveThemePreference,
  ThemePersistenceError,
  type ThemePreference,
} from "../theme";

function connectionLabel(state: ConnectionState): string {
  switch (state) {
    case "disconnected":
      return "Disconnected";
    case "connecting":
      return "Connecting";
    case "connected":
      return "Connected";
    case "reconnect_required":
      return "Reconnect required";
    case "unavailable_in_this_build":
      return "Unavailable in this build";
  }
}

function freshnessLabel(catalog: ProviderModelCatalog | null): string {
  if (catalog === null || catalog.models.length === 0) {
    return "No catalog";
  }
  return catalog.freshness === "current" ? "Current" : "Last known";
}

export function SettingsWindow() {
  const titleId = useId();
  const modelSelectId = useId();
  const [category, setCategory] = useState<SettingsCategory>("providers");
  const [theme, setTheme] = useState<ThemePreference>("system");
  const [connectionState, setConnectionState] = useState<ConnectionState>("disconnected");
  const [catalog, setCatalog] = useState<ProviderModelCatalog | null>(null);
  const [selection, setSelection] = useState<ProviderModelSelection | null>(null);
  const [busy, setBusy] = useState(false);
  const [cancelRequested, setCancelRequested] = useState(false);
  const [statusMessage, setStatusMessage] = useState<string | null>(null);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [appearanceError, setAppearanceError] = useState<string | null>(null);

  const selectedModelId = selection?.selectedModelId ?? "";
  const displayModel =
    selectedModelId === ""
      ? "Choose a model"
      : formatModelLabel(selectedModelId, catalog?.models ?? []);

  useEffect(() => {
    let active = true;

    void takeSettingsLaunchCategory()
      .then((launchCategory) => {
        if (active && launchCategory !== null) {
          setCategory(launchCategory);
        }
      })
      .catch(() => undefined);

    void loadThemePreference().then((preference) => {
      if (active) {
        setTheme(preference);
        applyThemePreference(preference);
      }
    });

    void Promise.all([getConnectionStatus(), getProviderModelSelection()])
      .then(async ([status, nextSelection]) => {
        if (!active) {
          return;
        }
        setConnectionState(status.state);
        setSelection(nextSelection);
        try {
          // Connected installs refresh missing/stale catalogs via get; force
          // refresh recovers from an initial empty failure path.
          const nextCatalog =
            status.state === "connected"
              ? await refreshProviderModelCatalog().catch(async (error: unknown) => {
                  if (active) {
                    setErrorMessage(getSafeAgentErrorMessage(error));
                  }
                  return getProviderModelCatalog();
                })
              : await getProviderModelCatalog();
          if (active) {
            setCatalog(nextCatalog);
          }
        } catch {
          /* bounded load failure leaves empty catalog; Refresh remains available */
        }
      })
      .catch(() => {
        if (active) {
          setConnectionState("unavailable_in_this_build");
        }
      });

    return () => {
      active = false;
    };
  }, []);

  useEffect(() => {
    let disposed = false;
    const cleanups: (() => void)[] = [];

    void listenAppearanceChanged((preference) => {
      setTheme(preference);
      applyThemePreference(preference);
    }).then((unlisten) => {
      if (disposed) {
        unlisten();
      } else {
        cleanups.push(unlisten);
      }
    });

    void listenConnectionStatusChanged((status) => {
      setConnectionState(status.state);
      if (status.state !== "connecting") {
        setCancelRequested(false);
        setStatusMessage((current) =>
          current === "Cancelling browser connection…" ? null : current,
        );
      }
      if (status.state === "connected") {
        void refreshProviderModelCatalog()
          .then(async (nextCatalog) => {
            setCatalog(nextCatalog);
            setSelection(await getProviderModelSelection());
          })
          .catch(async (error: unknown) => {
            setErrorMessage(getSafeAgentErrorMessage(error));
            const [nextCatalog, nextSelection] = await Promise.all([
              getProviderModelCatalog().catch(() => null),
              getProviderModelSelection().catch(() => null),
            ]);
            if (nextCatalog !== null) {
              setCatalog(nextCatalog);
            }
            if (nextSelection !== null) {
              setSelection(nextSelection);
            }
          });
      }
    }).then((unlisten) => {
      if (disposed) {
        unlisten();
      } else {
        cleanups.push(unlisten);
      }
    });

    void listenProviderModelCatalogChanged((next) => {
      setCatalog(next);
    }).then((unlisten) => {
      if (disposed) {
        unlisten();
      } else {
        cleanups.push(unlisten);
      }
    });

    void listenProviderModelSelectionChanged((next) => {
      setSelection(next);
    }).then((unlisten) => {
      if (disposed) {
        unlisten();
      } else {
        cleanups.push(unlisten);
      }
    });

    void listenSettingsNavigate((next) => {
      setCategory(next);
    }).then((unlisten) => {
      if (disposed) {
        unlisten();
      } else {
        cleanups.push(unlisten);
      }
    });

    return () => {
      disposed = true;
      for (const cleanup of cleanups) {
        cleanup();
      }
    };
  }, []);

  async function handleConnect() {
    setBusy(true);
    setCancelRequested(false);
    setStatusMessage(null);
    setErrorMessage(null);
    setConnectionState("connecting");
    try {
      const status = await connectChatgpt();
      setConnectionState(status.state);
      setStatusMessage(null);
      setErrorMessage(null);
      const [nextCatalog, nextSelection] = await Promise.all([
        getProviderModelCatalog(),
        getProviderModelSelection(),
      ]);
      setCatalog(nextCatalog);
      setSelection(nextSelection);
    } catch (error: unknown) {
      if (getAgentErrorCode(error) === "cancelled") {
        setStatusMessage("Browser connection cancelled.");
        setErrorMessage(null);
      } else {
        setStatusMessage(null);
        setErrorMessage(getSafeAgentErrorMessage(error));
      }
      const status = await getConnectionStatus().catch(() => null);
      if (status !== null) {
        setConnectionState(status.state);
      } else {
        setConnectionState("disconnected");
      }
    } finally {
      setCancelRequested(false);
      setBusy(false);
    }
  }

  async function handleCancelConnect() {
    if (connectionState !== "connecting" || cancelRequested) {
      return;
    }

    setCancelRequested(true);
    setStatusMessage("Cancelling browser connection…");
    setErrorMessage(null);
    try {
      await cancelChatgptConnect();
    } catch (error: unknown) {
      // Completion already won: reconcile to the terminal status and never show
      // Agent-composer validation such as "Enter a valid message".
      if (isStaleConnectCancellation(error)) {
        const status = await getConnectionStatus().catch(() => null);
        if (status !== null) {
          setConnectionState(status.state);
        }
        setCancelRequested(false);
        setStatusMessage(null);
        setErrorMessage(null);
        return;
      }
      setCancelRequested(false);
      setStatusMessage(null);
      setErrorMessage(getSafeAgentErrorMessage(error));
    }
  }

  async function handleDisconnect() {
    setBusy(true);
    setStatusMessage(null);
    setErrorMessage(null);
    try {
      const status = await disconnectChatgpt();
      setConnectionState(status.state);
      const [nextCatalog, nextSelection] = await Promise.all([
        getProviderModelCatalog(),
        getProviderModelSelection(),
      ]);
      setCatalog(nextCatalog);
      setSelection(nextSelection);
      if (status.state === "disconnected") {
        setStatusMessage("Removed from this device");
      }
    } catch (error: unknown) {
      setErrorMessage(getSafeAgentErrorMessage(error));
    } finally {
      setBusy(false);
    }
  }

  async function handleRefreshCatalog() {
    setBusy(true);
    setErrorMessage(null);
    try {
      const nextCatalog = await refreshProviderModelCatalog();
      setCatalog(nextCatalog);
      setSelection(await getProviderModelSelection());
    } catch (error: unknown) {
      setErrorMessage(getSafeAgentErrorMessage(error));
      const [nextCatalog, nextSelection] = await Promise.all([
        getProviderModelCatalog().catch(() => null),
        getProviderModelSelection().catch(() => null),
      ]);
      if (nextCatalog !== null) {
        setCatalog(nextCatalog);
      }
      if (nextSelection !== null) {
        setSelection(nextSelection);
      }
    } finally {
      setBusy(false);
    }
  }

  async function handleDefaultModelChange(modelId: string) {
    setErrorMessage(null);
    try {
      const next = await setProviderModelSelection(modelId);
      setSelection(next);
    } catch (error: unknown) {
      setErrorMessage(getSafeAgentErrorMessage(error));
    }
  }

  async function handleThemeChange(next: ThemePreference) {
    setTheme(next);
    applyThemePreference(next);
    setAppearanceError(null);
    try {
      await saveThemePreference(next);
    } catch (error: unknown) {
      setAppearanceError(
        error instanceof ThemePersistenceError ? error.message : "Appearance could not be saved.",
      );
    }
  }

  const connecting = connectionState === "connecting";
  const canConnect = connectionState === "disconnected" || connectionState === "reconnect_required";
  const canDisconnect = connectionState === "connected";
  const canSelectModel = connectionState === "connected" && (catalog?.models.length ?? 0) > 0;
  const canRefreshModels = connectionState === "connected";

  return (
    <div className="settings-window" aria-labelledby={titleId}>
      <h1 id={titleId} className="sr-only">
        Settings
      </h1>
      <nav className="settings-nav" aria-label="Settings categories">
        <button
          className={`settings-nav-item${category === "providers" ? " is-selected" : ""}`}
          type="button"
          aria-current={category === "providers" ? "page" : undefined}
          onClick={() => setCategory("providers")}
        >
          Providers
        </button>
        <button
          className={`settings-nav-item${category === "appearance" ? " is-selected" : ""}`}
          type="button"
          aria-current={category === "appearance" ? "page" : undefined}
          onClick={() => setCategory("appearance")}
        >
          Appearance
        </button>
      </nav>

      <div className="settings-content">
        {category === "providers" ? (
          <section className="settings-section" aria-labelledby="provider-settings-title">
            <h2 id="provider-settings-title">ChatGPT subscription</h2>
            <p className="settings-label-row">
              <span className="experimental-label">Experimental</span>
            </p>
            <p className="settings-disclosure">
              Uses a compatibility sign-in path that is not an official TULE integration and may
              stop working.
            </p>
            <p className="settings-meta">
              <span>Status</span>
              <strong>{connectionLabel(connectionState)}</strong>
            </p>
            <p className="settings-meta">
              <span>Default model</span>
              <strong>{displayModel}</strong>
            </p>
            <p className="settings-meta">
              <span>Catalog</span>
              <strong>{freshnessLabel(catalog)}</strong>
            </p>
            {canSelectModel ? (
              <>
                <label className="settings-field-label" htmlFor={modelSelectId}>
                  Default model for new sessions
                </label>
                <select
                  id={modelSelectId}
                  value={selectedModelId}
                  disabled={busy}
                  onChange={(event) => void handleDefaultModelChange(event.currentTarget.value)}
                >
                  {selection?.requiresSelection || selectedModelId === "" ? (
                    <option value="" disabled>
                      Choose a model
                    </option>
                  ) : null}
                  {(catalog?.models ?? []).map((model) => (
                    <option key={model.id} value={model.id}>
                      {model.displayName}
                    </option>
                  ))}
                </select>
              </>
            ) : canRefreshModels ? (
              <p className="settings-disclosure">
                No usable models are available yet. Refresh to recover the catalog.
              </p>
            ) : null}
            {canRefreshModels ? (
              <button
                className="secondary-action"
                type="button"
                disabled={busy}
                onClick={() => void handleRefreshCatalog()}
              >
                Refresh models
              </button>
            ) : null}
            {connecting ? (
              <button
                className="secondary-action"
                type="button"
                disabled={cancelRequested}
                onClick={() => void handleCancelConnect()}
              >
                {cancelRequested ? "Cancelling…" : "Cancel connection"}
              </button>
            ) : canConnect ? (
              <button
                className="primary-action"
                type="button"
                disabled={busy}
                onClick={() => void handleConnect()}
              >
                Connect in browser
              </button>
            ) : null}
            {canDisconnect ? (
              <button
                className="secondary-action"
                type="button"
                disabled={busy}
                onClick={() => void handleDisconnect()}
              >
                Disconnect
              </button>
            ) : null}
            {connectionState === "connected" ? (
              <p className="settings-note">
                Removed from this device clears local credentials only.
              </p>
            ) : null}
            {statusMessage === null ? null : (
              <p className="settings-note" role="status">
                {statusMessage}
              </p>
            )}
            {errorMessage === null ? null : (
              <p className="field-error" role="alert">
                {errorMessage}
              </p>
            )}
            {connectionState === "unavailable_in_this_build" ? (
              <p className="settings-note" role="status">
                ChatGPT connection is unavailable in this build.
              </p>
            ) : null}
          </section>
        ) : (
          <section className="settings-section" aria-labelledby="appearance-settings-title">
            <h2 id="appearance-settings-title">Appearance</h2>
            <label className="sr-only" htmlFor="appearance-preference">
              Appearance
            </label>
            <select
              id="appearance-preference"
              value={theme}
              onChange={(event) =>
                void handleThemeChange(event.currentTarget.value as ThemePreference)
              }
            >
              <option value="system">System</option>
              <option value="light">Light</option>
              <option value="dark">Dark</option>
            </select>
            {appearanceError === null ? null : (
              <p className="field-error" role="alert">
                {appearanceError}
              </p>
            )}
          </section>
        )}
      </div>
    </div>
  );
}
