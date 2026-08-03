import { useEffect, useId, useState } from "react";
import {
  cancelChatgptConnect,
  connectChatgpt,
  disconnectChatgpt,
  getConnectionStatus,
  type ConnectionState,
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

export function SettingsWindow() {
  const titleId = useId();
  const [category, setCategory] = useState<SettingsCategory>("providers");
  const [theme, setTheme] = useState<ThemePreference>("system");
  const [connectionState, setConnectionState] = useState<ConnectionState>("disconnected");
  const [model, setModel] = useState("gpt-5.5");
  const [busy, setBusy] = useState(false);
  const [cancelRequested, setCancelRequested] = useState(false);
  const [statusMessage, setStatusMessage] = useState<string | null>(null);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [appearanceError, setAppearanceError] = useState<string | null>(null);
  const displayModel = model === "gpt-5.5" ? "GPT-5.5" : model;

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

    void getConnectionStatus()
      .then((status) => {
        if (active) {
          setConnectionState(status.state);
          setModel(status.model);
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
      setModel(status.model);
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
      setModel(status.model);
    } catch (error: unknown) {
      if (getAgentErrorCode(error) === "cancelled") {
        setStatusMessage("Browser connection cancelled.");
      } else {
        setErrorMessage(getSafeAgentErrorMessage(error));
      }
      const status = await getConnectionStatus().catch(() => null);
      if (status !== null) {
        setConnectionState(status.state);
        setModel(status.model);
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
      setModel(status.model);
      if (status.state === "disconnected") {
        setStatusMessage("Removed from this device");
      }
    } catch (error: unknown) {
      setErrorMessage(getSafeAgentErrorMessage(error));
    } finally {
      setBusy(false);
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
              <span>Model</span>
              <strong>{displayModel}</strong>
            </p>
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
