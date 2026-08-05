import {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type FormEvent,
  type KeyboardEvent,
  type UIEvent,
} from "react";
import {
  formatSourceAttachmentSummary,
  getSafeAgentErrorMessageForCode,
  sourceAttachmentKindLabel,
  type AgentSourceMetadata,
  type AgentTurn,
  type PendingSourceAttachment,
} from "../platform/agents";
import { AttachFileIcon, AttachFolderIcon, JumpToLatestIcon, RemoveIcon, SendIcon } from "./icons";
import { Tooltip } from "./Tooltip";
import { TuleWordmark } from "./TuleWordmark";

/** Compact initial composer height in CSS pixels. */
export const COMPOSER_MIN_HEIGHT_PX = 56;
/** Bounded maximum composer height before internal scrolling. */
export const COMPOSER_MAX_HEIGHT_PX = 160;

interface AgentProjectOption {
  id: string;
  displayName: string;
}

interface AgentModelOption {
  id: string;
  displayName: string;
}

interface AgentWorkspaceProps {
  title: string;
  projectId: string | null;
  projects: readonly AgentProjectOption[];
  modelLabel: string;
  modelOptions: readonly AgentModelOption[];
  selectedModelId: string | null;
  modelLocked: boolean;
  turns: readonly AgentTurn[];
  draft: string;
  pendingAttachment: PendingSourceAttachment | null;
  connected: boolean;
  sending: boolean;
  sendBlocked: boolean;
  cancelRequested: boolean;
  activeTurnId: string | null;
  errorMessage: string | null;
  onDraftChange: (value: string) => void;
  onSend: () => void;
  onCancel: () => void;
  onAttach: () => void;
  onAttachFolder: () => void;
  onRemoveAttachment: () => void;
  onProjectChange: (projectId: string | null) => void;
  onModelChange: (modelId: string) => void;
  onOpenProvidersSettings: () => void;
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

function formatPersistedAttachmentSummary(source: AgentSourceMetadata): string {
  return formatSourceAttachmentSummary(source);
}

function attachmentActionLabel(
  pendingAttachment: PendingSourceAttachment | null,
  kind: "file" | "folder",
): string {
  if (pendingAttachment === null) {
    return kind === "folder" ? "Attach folder" : "Attach file";
  }
  return kind === "folder" ? "Replace folder" : "Replace file";
}

function isNearBottom(element: HTMLElement): boolean {
  return element.scrollHeight - element.scrollTop - element.clientHeight <= 48;
}

export function AgentWorkspace({
  title,
  projectId,
  projects,
  modelLabel,
  modelOptions,
  selectedModelId,
  modelLocked,
  turns,
  draft,
  pendingAttachment,
  connected,
  sending,
  sendBlocked,
  cancelRequested,
  activeTurnId,
  errorMessage,
  onDraftChange,
  onSend,
  onCancel,
  onAttach,
  onAttachFolder,
  onRemoveAttachment,
  onProjectChange,
  onModelChange,
  onOpenProvidersSettings,
}: AgentWorkspaceProps) {
  const titleId = useId();
  const modelSelectId = useId();
  const transcriptRef = useRef<HTMLDivElement>(null);
  const composerRef = useRef<HTMLTextAreaElement>(null);
  const followRef = useRef(true);
  const [showJumpToLatest, setShowJumpToLatest] = useState(false);
  const previousTurnCountRef = useRef(0);
  const isEmptyNewSession = turns.length === 0 && title === "New session";

  useLayoutEffect(() => {
    const composer = composerRef.current;
    if (composer === null) {
      return;
    }

    composer.style.height = "auto";
    const next = Math.min(
      COMPOSER_MAX_HEIGHT_PX,
      Math.max(COMPOSER_MIN_HEIGHT_PX, composer.scrollHeight),
    );
    composer.style.height = `${next}px`;
    composer.style.overflowY = composer.scrollHeight > COMPOSER_MAX_HEIGHT_PX ? "auto" : "hidden";
  }, [draft, connected]);

  useLayoutEffect(() => {
    const transcript = transcriptRef.current;
    if (transcript === null) {
      return;
    }

    const turnCountIncreased = turns.length > previousTurnCountRef.current;
    previousTurnCountRef.current = turns.length;

    if (turnCountIncreased) {
      followRef.current = true;
      setShowJumpToLatest(false);
      const latest = transcript.querySelector<HTMLElement>(".turn:last-of-type");
      latest?.scrollIntoView?.({ block: "nearest" });
      return;
    }

    if (followRef.current) {
      transcript.scrollTop = transcript.scrollHeight;
    }
  }, [turns, activeTurnId]);

  useEffect(() => {
    if (!sending) {
      return;
    }

    const transcript = transcriptRef.current;
    if (transcript !== null && followRef.current) {
      transcript.scrollTop = transcript.scrollHeight;
    }
  }, [turns, sending]);

  function handleTranscriptScroll(event: UIEvent<HTMLDivElement>) {
    const nearBottom = isNearBottom(event.currentTarget);
    followRef.current = nearBottom;
    setShowJumpToLatest(!nearBottom && turns.length > 0);
  }

  function jumpToLatest() {
    const transcript = transcriptRef.current;
    if (transcript === null) {
      return;
    }

    followRef.current = true;
    setShowJumpToLatest(false);
    transcript.scrollTop = transcript.scrollHeight;
  }

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
            {modelLocked || modelOptions.length === 0 ? (
              <span>{modelLabel}</span>
            ) : (
              <>
                <label className="sr-only" htmlFor={modelSelectId}>
                  Model for this session
                </label>
                <select
                  id={modelSelectId}
                  className="truncate"
                  aria-label="Model for this session"
                  value={selectedModelId ?? ""}
                  disabled={sending || sendBlocked}
                  onChange={(event) => onModelChange(event.currentTarget.value)}
                >
                  {selectedModelId === null || selectedModelId === "" ? (
                    <option value="" disabled>
                      Choose a model
                    </option>
                  ) : null}
                  {modelOptions.map((model) => (
                    <option key={model.id} value={model.id}>
                      {model.displayName}
                    </option>
                  ))}
                </select>
              </>
            )}
          </p>
          {modelLocked ? (
            <p className="session-model-note">Start a new session to change models.</p>
          ) : null}
        </div>
      </header>

      <div className="transcript-shell">
        <div
          ref={transcriptRef}
          className="transcript"
          aria-live="polite"
          onScroll={handleTranscriptScroll}
        >
          <div className="transcript-measure">
            {isEmptyNewSession ? (
              <div className="transcript-empty-state">
                <TuleWordmark />
                <p className="transcript-empty">Start a conversation with the Agent.</p>
              </div>
            ) : turns.length === 0 ? (
              <p className="transcript-empty">Start a conversation with the Agent.</p>
            ) : (
              turns.map((turn) => {
                const stateLabel = turnStateLabel(turn.state);
                const attachment = turn.sources[0] ?? null;
                return (
                  <article key={turn.id} className="turn">
                    <div className="turn-block user">
                      <h2>You</h2>
                      <pre className="turn-text">{turn.userText}</pre>
                      {attachment === null ? null : (
                        <p className="turn-attachment">
                          Attached {sourceAttachmentKindLabel(attachment.originKind)} snapshot:{" "}
                          {formatPersistedAttachmentSummary(attachment)}
                        </p>
                      )}
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
        </div>
        {showJumpToLatest ? (
          <div className="jump-to-latest">
            <Tooltip label="Jump to latest" align="end">
              <button
                className="icon-button jump-to-latest-button"
                type="button"
                aria-label="Jump to latest"
                onClick={jumpToLatest}
              >
                <JumpToLatestIcon />
              </button>
            </Tooltip>
          </div>
        ) : null}
      </div>

      {errorMessage === null ? null : (
        <div className="workspace-error" role="alert">
          {errorMessage}
        </div>
      )}

      <div className="composer">
        <div className="composer-measure">
          {connected ? (
            <form onSubmit={handleSubmit}>
              <label className="sr-only" htmlFor="agent-composer">
                Message the Agent
              </label>
              <textarea
                ref={composerRef}
                id="agent-composer"
                value={draft}
                disabled={sending}
                rows={2}
                onChange={(event) => onDraftChange(event.currentTarget.value)}
                onKeyDown={handleKeyDown}
              />
              {pendingAttachment === null ? null : (
                <div className="composer-attachment" aria-live="polite">
                  <p>
                    Captured {sourceAttachmentKindLabel(pendingAttachment.originKind)} snapshot:{" "}
                    {formatSourceAttachmentSummary(pendingAttachment)}
                  </p>
                  <Tooltip label="Remove">
                    <button
                      className="icon-button composer-icon"
                      type="button"
                      disabled={sending}
                      aria-label={`Remove attachment ${pendingAttachment.displayName}`}
                      onClick={onRemoveAttachment}
                    >
                      <RemoveIcon />
                    </button>
                  </Tooltip>
                </div>
              )}
              <div className="composer-actions">
                <Tooltip label={attachmentActionLabel(pendingAttachment, "file")}>
                  <button
                    className="icon-button composer-icon"
                    type="button"
                    disabled={sending}
                    aria-label={attachmentActionLabel(pendingAttachment, "file")}
                    onClick={onAttach}
                  >
                    <AttachFileIcon />
                  </button>
                </Tooltip>
                <Tooltip label={attachmentActionLabel(pendingAttachment, "folder")}>
                  <button
                    className="icon-button composer-icon"
                    type="button"
                    disabled={sending}
                    aria-label={attachmentActionLabel(pendingAttachment, "folder")}
                    onClick={onAttachFolder}
                  >
                    <AttachFolderIcon />
                  </button>
                </Tooltip>
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
                {sending ? (
                  <button className="primary-action" type="submit" disabled>
                    Sending…
                  </button>
                ) : (
                  <Tooltip label="Send">
                    <button
                      className="icon-button composer-icon composer-send"
                      type="submit"
                      disabled={sendBlocked || draft.trim().length === 0}
                      aria-label="Send"
                    >
                      <SendIcon />
                    </button>
                  </Tooltip>
                )}
              </div>
            </form>
          ) : (
            <div className="composer-unavailable">
              <p>Connect ChatGPT in Settings to message the Agent.</p>
              <button className="secondary-action" type="button" onClick={onOpenProvidersSettings}>
                Open Settings
              </button>
            </div>
          )}
        </div>
      </div>
    </section>
  );
}
