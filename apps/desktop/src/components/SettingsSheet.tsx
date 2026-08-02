import { useEffect, useId, useRef } from "react";
import type { ConnectionState } from "../platform/provider";
import type { ThemePreference } from "../theme";

interface SettingsSheetProps {
  open: boolean;
  connectionState: ConnectionState;
  model: string;
  theme: ThemePreference;
  busy: boolean;
  turnBusy: boolean;
  cancelRequested: boolean;
  statusMessage: string | null;
  errorMessage: string | null;
  onClose: () => void;
  onConnect: () => void;
  onCancelConnect: () => void;
  onDisconnect: () => void;
  onThemeChange: (theme: ThemePreference) => void;
  returnFocusRef: React.RefObject<HTMLButtonElement | null>;
}

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

export function SettingsSheet({
  open,
  connectionState,
  model,
  theme,
  busy,
  turnBusy,
  cancelRequested,
  statusMessage,
  errorMessage,
  onClose,
  onConnect,
  onCancelConnect,
  onDisconnect,
  onThemeChange,
  returnFocusRef,
}: SettingsSheetProps) {
  const titleId = useId();
  const closeRef = useRef<HTMLButtonElement>(null);
  const sheetRef = useRef<HTMLDivElement>(null);
  const wasOpenRef = useRef(false);
  const displayModel = model === "gpt-5.5" ? "GPT-5.5" : model;

  useEffect(() => {
    if (!open) {
      if (wasOpenRef.current) {
        returnFocusRef.current?.focus();
      }
      wasOpenRef.current = false;
      return;
    }

    wasOpenRef.current = true;
    closeRef.current?.focus();

    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
        return;
      }

      if (event.key !== "Tab") {
        return;
      }

      const focusable = Array.from(
        sheetRef.current?.querySelectorAll<HTMLElement>(
          'button:not([disabled]), select:not([disabled]), input:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
        ) ?? [],
      );
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (first === undefined || last === undefined) {
        event.preventDefault();
        return;
      }

      const activeElement = document.activeElement;
      if (
        event.shiftKey &&
        (activeElement === first || !sheetRef.current?.contains(activeElement))
      ) {
        event.preventDefault();
        last.focus();
      } else if (
        !event.shiftKey &&
        (activeElement === last || !sheetRef.current?.contains(activeElement))
      ) {
        event.preventDefault();
        first.focus();
      }
    }

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [open, onClose, returnFocusRef]);

  if (!open) {
    return null;
  }

  const connecting = connectionState === "connecting";
  const canConnect = connectionState === "disconnected" || connectionState === "reconnect_required";
  const canDisconnect = connectionState === "connected";

  return (
    <div className="settings-layer">
      <div className="settings-backdrop" aria-hidden="true" />
      <div
        ref={sheetRef}
        className="settings-sheet"
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
      >
        <div className="settings-header">
          <h2 id={titleId}>Settings</h2>
          <button ref={closeRef} className="icon-button" type="button" onClick={onClose}>
            Close
          </button>
        </div>

        <section className="settings-section" aria-labelledby="provider-settings-title">
          <h3 id="provider-settings-title">ChatGPT subscription</h3>
          <p className="settings-label-row">
            <span className="experimental-label">Experimental</span>
          </p>
          <p className="settings-disclosure">
            Uses a compatibility sign-in path that is not an official TULE integration and may stop
            working.
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
              onClick={onCancelConnect}
            >
              {cancelRequested ? "Cancelling…" : "Cancel connection"}
            </button>
          ) : canConnect ? (
            <button className="primary-action" type="button" disabled={busy} onClick={onConnect}>
              Connect in browser
            </button>
          ) : null}
          {canDisconnect ? (
            <button
              className="secondary-action"
              type="button"
              disabled={busy || turnBusy}
              onClick={onDisconnect}
            >
              Disconnect
            </button>
          ) : null}
          {connectionState === "connected" ? (
            <p className="settings-note">Removed from this device clears local credentials only.</p>
          ) : null}
          {turnBusy && connectionState === "connected" ? (
            <p className="settings-note">
              Disconnect is unavailable while the Agent is responding.
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

        <section className="settings-section" aria-labelledby="appearance-settings-title">
          <h3 id="appearance-settings-title">Appearance</h3>
          <label className="sr-only" htmlFor="appearance-preference">
            Appearance
          </label>
          <select
            id="appearance-preference"
            value={theme}
            onChange={(event) => onThemeChange(event.currentTarget.value as ThemePreference)}
          >
            <option value="system">System</option>
            <option value="light">Light</option>
            <option value="dark">Dark</option>
          </select>
        </section>
      </div>
    </div>
  );
}
