import { useId, type FormEvent, type KeyboardEvent } from "react";
import { getSafeAgentErrorMessageForCode, type AgentTurn } from "../platform/agents";

interface AgentProjectOption {
  id: string;
  displayName: string;
}

interface AgentWorkspaceProps {
  title: string;
  projectId: string | null;
  projects: readonly AgentProjectOption[];
  modelLabel: string;
  turns: readonly AgentTurn[];
  draft: string;
  connected: boolean;
  sending: boolean;
  sendBlocked: boolean;
  cancelRequested: boolean;
  activeTurnId: string | null;
  errorMessage: string | null;
  onDraftChange: (value: string) => void;
  onSend: () => void;
  onCancel: () => void;
  onProjectChange: (projectId: string | null) => void;
  onOpenSettings: () => void;
  settingsButtonRef: React.RefObject<HTMLButtonElement | null>;
}

function turnStateLabel(state: string): string | null {
  switch (state) {
    case "streaming":
    case "pending":
      return "Receiving…";
    case "cancelled":
      return "Cancelled — TULE stopped receiving";
    case "interrupted":
      return "Interrupted";
    case "failed":
      return "Failed";
    default:
      return null;
  }
}

export function AgentWorkspace({
  title,
  projectId,
  projects,
  modelLabel,
  turns,
  draft,
  connected,
  sending,
  sendBlocked,
  cancelRequested,
  activeTurnId,
  errorMessage,
  onDraftChange,
  onSend,
  onCancel,
  onProjectChange,
  onOpenSettings,
  settingsButtonRef,
}: AgentWorkspaceProps) {
  const titleId = useId();

  function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (connected && !sending && !sendBlocked) {
      onSend();
    }
  }

  function handleKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      if (connected && !sending && !sendBlocked) {
        onSend();
      }
    }
  }

  return (
    <section className="agent-workspace" aria-labelledby={titleId}>
      <header className="agent-header">
        <div className="agent-header-copy">
          <h1 id={titleId} className="session-title">
            {title}
          </h1>
          <p className="session-meta">
            <select
              className="truncate"
              aria-label="Project context"
              value={projectId ?? ""}
              disabled={sending || sendBlocked}
              onChange={(event) => onProjectChange(event.currentTarget.value || null)}
            >
              <option value="">No project</option>
              {projects.map((project) => (
                <option key={project.id} value={project.id}>
                  {project.displayName}
                </option>
              ))}
            </select>
            <span aria-hidden="true">·</span>
            <span>ChatGPT subscription</span>
            <span aria-hidden="true">·</span>
            <span>{modelLabel}</span>
          </p>
        </div>
        <button
          ref={settingsButtonRef}
          className="icon-button settings-gear"
          type="button"
          aria-label="Settings"
          onClick={onOpenSettings}
        >
          Settings
        </button>
      </header>

      <div className="transcript" aria-live="polite">
        {turns.length === 0 ? (
          <p className="transcript-empty">Start a conversation with the Agent.</p>
        ) : (
          turns.map((turn) => {
            const stateLabel = turnStateLabel(turn.state);
            return (
              <article key={turn.id} className="turn">
                <div className="turn-block user">
                  <h2>You</h2>
                  <pre className="turn-text">{turn.userText}</pre>
                </div>
                <div className="turn-block agent">
                  <h2>Agent</h2>
                  <pre className="turn-text">
                    {turn.agentText || (turn.id === activeTurnId ? "…" : "")}
                  </pre>
                  {stateLabel === null ? null : <p className="turn-state">{stateLabel}</p>}
                  {turn.errorCode === null ? null : (
                    <p className="turn-state" role="status">
                      {getSafeAgentErrorMessageForCode(turn.errorCode)}
                    </p>
                  )}
                </div>
              </article>
            );
          })
        )}
      </div>

      {errorMessage === null ? null : (
        <div className="workspace-error" role="alert">
          {errorMessage}
        </div>
      )}

      <div className="composer">
        {connected ? (
          <form onSubmit={handleSubmit}>
            <label className="sr-only" htmlFor="agent-composer">
              Message the Agent
            </label>
            <textarea
              id="agent-composer"
              value={draft}
              disabled={sending}
              rows={3}
              onChange={(event) => onDraftChange(event.currentTarget.value)}
              onKeyDown={handleKeyDown}
            />
            <div className="composer-actions">
              {sending ? (
                <button
                  className="secondary-action"
                  type="button"
                  disabled={cancelRequested}
                  onClick={onCancel}
                >
                  {cancelRequested ? "Cancelling…" : "Cancel"}
                </button>
              ) : null}
              <button
                className="primary-action"
                type="submit"
                disabled={sending || sendBlocked || draft.trim().length === 0}
              >
                {sending ? "Sending…" : "Send"}
              </button>
            </div>
          </form>
        ) : (
          <div className="composer-unavailable">
            <p>Connect ChatGPT in Settings to message the Agent.</p>
            <button className="secondary-action" type="button" onClick={onOpenSettings}>
              Open Settings
            </button>
          </div>
        )}
      </div>
    </section>
  );
}
