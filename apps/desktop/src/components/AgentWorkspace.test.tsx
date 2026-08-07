import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { AgentWorkspace, COMPOSER_MAX_HEIGHT_PX, COMPOSER_MIN_HEIGHT_PX } from "./AgentWorkspace";
import type { AgentEvent, AgentTurn } from "../platform/agents";

function stubTranscriptMetrics(
  transcript: HTMLElement,
  values: { scrollHeight: number; clientHeight: number; scrollTop: number },
) {
  Object.defineProperty(transcript, "scrollHeight", {
    configurable: true,
    get: () => values.scrollHeight,
  });
  Object.defineProperty(transcript, "clientHeight", {
    configurable: true,
    get: () => values.clientHeight,
  });
  Object.defineProperty(transcript, "scrollTop", {
    configurable: true,
    get: () => values.scrollTop,
    set: (next: number) => {
      values.scrollTop = next;
    },
  });
}

const baseTurn: AgentTurn = {
  id: "t1",
  ordinal: 1,
  userText: "Hi",
  agentText: "Hello",
  state: "completed",
  errorCode: null,
  effort: null,
  sources: [],
};

const baseEvents: AgentEvent[] = [
  {
    id: "01900000-0000-7000-8000-000000000020",
    sessionId: "01900000-0000-7000-8000-000000000010",
    turnId: null,
    sequence: 0,
    kind: "session_created",
    createdAtUnixMs: 1_700_000_000_000,
  },
  {
    id: "01900000-0000-7000-8000-000000000021",
    sessionId: "01900000-0000-7000-8000-000000000010",
    turnId: "t1",
    sequence: 1,
    kind: "turn_pending",
    createdAtUnixMs: 1_700_000_000_001,
  },
  {
    id: "01900000-0000-7000-8000-000000000022",
    sessionId: "01900000-0000-7000-8000-000000000010",
    turnId: "t1",
    sequence: 2,
    kind: "turn_completed",
    createdAtUnixMs: 1_700_000_000_002,
  },
];

describe("AgentWorkspace", () => {
  it("blocks composer when disconnected with a message-only prompt", () => {
    render(
      <AgentWorkspace
        title="New session"
        projectId={null}
        projects={[]}
        modelLabel="GPT-5.5"
        modelOptions={[{ id: "gpt-5.5", displayName: "GPT-5.5" }]}
        selectedModelId="gpt-5.5"
        modelLocked={false}
        effortAvailable={false}
        effortValues={[]}
        selectedEffort={null}
        onEffortChange={() => undefined}
        onModelChange={() => undefined}
        turns={[]}
        events={[]}
        draft=""
        pendingAttachment={null}
        connected={false}
        sending={false}
        sendBlocked={false}
        cancelRequested={false}
        activeTurnId={null}
        errorMessage={null}
        onDraftChange={vi.fn()}
        onSend={vi.fn()}
        onCancel={vi.fn()}
        onAttach={vi.fn()}
        onAttachFolder={vi.fn()}
        onAttachLink={vi.fn()}
        onRemoveAttachment={vi.fn()}
        onProjectChange={vi.fn()}
      />,
    );

    expect(screen.getByRole("img", { name: "TULE" })).toBeInTheDocument();
    expect(screen.getByText("Add a Provider to get started.")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Open Settings" })).not.toBeInTheDocument();
    expect(screen.getByText("Provider")).toBeInTheDocument();
  });

  it("renders a collapsible Activity list keyed by event id with turn association", () => {
    render(
      <AgentWorkspace
        title="Hello"
        projectId={null}
        projects={[]}
        modelLabel="GPT-5.5"
        modelOptions={[{ id: "gpt-5.5", displayName: "GPT-5.5" }]}
        selectedModelId="gpt-5.5"
        modelLocked={false}
        effortAvailable={false}
        effortValues={[]}
        selectedEffort={null}
        onEffortChange={() => undefined}
        onModelChange={() => undefined}
        turns={[baseTurn]}
        events={baseEvents}
        draft=""
        pendingAttachment={null}
        connected
        sending={false}
        sendBlocked={false}
        cancelRequested={false}
        activeTurnId={null}
        errorMessage={null}
        onDraftChange={vi.fn()}
        onSend={vi.fn()}
        onCancel={vi.fn()}
        onAttach={vi.fn()}
        onAttachFolder={vi.fn()}
        onAttachLink={vi.fn()}
        onRemoveAttachment={vi.fn()}
        onProjectChange={vi.fn()}
      />,
    );

    expect(screen.getByText("Activity")).toBeInTheDocument();
    expect(screen.getByText("Session created")).toBeInTheDocument();
    expect(screen.getByText("Turn completed")).toBeInTheDocument();
    expect(screen.getAllByText("Turn 1").length).toBeGreaterThan(0);
    expect(screen.getByText("Hi")).toBeInTheDocument();
  });

  it("offers save on completed turns and a collapsible Artifacts panel with detail", () => {
    const onSaveArtifact = vi.fn();
    const onOpenArtifact = vi.fn();
    const onCloseArtifact = vi.fn();
    render(
      <AgentWorkspace
        title="Hello"
        projectId={null}
        projects={[]}
        modelLabel="GPT-5.5"
        modelOptions={[{ id: "gpt-5.5", displayName: "GPT-5.5" }]}
        selectedModelId="gpt-5.5"
        modelLocked={false}
        effortAvailable={false}
        effortValues={[]}
        selectedEffort={null}
        onEffortChange={() => undefined}
        onModelChange={() => undefined}
        turns={[
          baseTurn,
          {
            ...baseTurn,
            id: "t2",
            ordinal: 2,
            agentText: "",
            state: "failed",
            errorCode: "provider_unavailable",
          },
          {
            ...baseTurn,
            id: "t3",
            ordinal: 3,
            agentText: "Still streaming",
            state: "streaming",
          },
        ]}
        events={[]}
        artifacts={[
          {
            id: "01900000-0000-7000-8000-000000000030",
            title: "Saved conclusion",
            kind: "conclusion",
            projectId: null,
            createdAtUnixMs: 1_700_000_000_000,
            latestVersionId: "01900000-0000-7000-8000-000000000031",
            latestVersionOrdinal: 1,
          },
        ]}
        selectedArtifactDetail={{
          artifact: {
            id: "01900000-0000-7000-8000-000000000030",
            title: "Saved conclusion",
            kind: "conclusion",
            projectId: null,
            createdAtUnixMs: 1_700_000_000_000,
          },
          versions: [
            {
              id: "01900000-0000-7000-8000-000000000031",
              artifactId: "01900000-0000-7000-8000-000000000030",
              versionOrdinal: 1,
              content: "Hello",
              contentSha256: "a".repeat(64),
              provenance: {
                sourceSessionId: "01900000-0000-7000-8000-000000000010",
                sourceTurnId: "01900000-0000-7000-8000-000000000011",
                providerProfileId: "xai-subscription-oauth",
                modelId: "grok-3",
                promptVersion: "tule-direct-agent-v2",
                projectId: null,
                providerRequestId: "01900000-0000-7000-8000-000000000012",
              },
              createdAtUnixMs: 1_700_000_000_000,
            },
          ],
        }}
        draft=""
        pendingAttachment={null}
        connected
        sending={false}
        sendBlocked={false}
        cancelRequested={false}
        activeTurnId={null}
        errorMessage={null}
        onDraftChange={vi.fn()}
        onSend={vi.fn()}
        onCancel={vi.fn()}
        onAttach={vi.fn()}
        onAttachFolder={vi.fn()}
        onAttachLink={vi.fn()}
        onRemoveAttachment={vi.fn()}
        onProjectChange={vi.fn()}
        onSaveArtifact={onSaveArtifact}
        onOpenArtifact={onOpenArtifact}
        onCloseArtifact={onCloseArtifact}
      />,
    );

    expect(screen.getByRole("button", { name: "Save as artifact" })).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: "Save as artifact" })).toHaveLength(1);
    fireEvent.click(screen.getByRole("button", { name: "Save as artifact" }));
    expect(onSaveArtifact).toHaveBeenCalledWith("t1");

    expect(screen.getByText("Artifacts")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Saved conclusion/ }));
    expect(onOpenArtifact).toHaveBeenCalledWith("01900000-0000-7000-8000-000000000030");
    expect(screen.getByRole("region", { name: "Artifact detail" })).toHaveTextContent("Hello");
    expect(screen.getByText("grok-3")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(onCloseArtifact).toHaveBeenCalled();
  });

  it("exposes attachment controls and transcript metadata accessibly", () => {
    const onAttach = vi.fn();
    const onRemoveAttachment = vi.fn();
    render(
      <AgentWorkspace
        title="Hello"
        projectId={null}
        projects={[]}
        modelLabel="GPT-5.5"
        modelOptions={[{ id: "gpt-5.5", displayName: "GPT-5.5" }]}
        selectedModelId="gpt-5.5"
        modelLocked={false}
        effortAvailable={false}
        effortValues={[]}
        selectedEffort={null}
        onEffortChange={() => undefined}
        onModelChange={() => undefined}
        turns={[
          {
            ...baseTurn,
            sources: [
              {
                id: "src1",
                originKind: "local_text_file",
                displayName: "notes.txt",
                byteCount: 12,
                contentSha256: "a".repeat(64),
                memberCount: 1,
                canonicalUrl: null,
              },
            ],
          },
        ]}
        events={[]}
        draft="Ask"
        pendingAttachment={{
          draftHandle: "handle",
          displayName: "draft.txt",
          byteCount: 4,
          originKind: "local_text_file",
          memberCount: 1,
          canonicalUrl: null,
        }}
        connected
        sending={false}
        sendBlocked={false}
        cancelRequested={false}
        activeTurnId={null}
        errorMessage={null}
        onDraftChange={vi.fn()}
        onSend={vi.fn()}
        onCancel={vi.fn()}
        onAttach={onAttach}
        onAttachFolder={vi.fn()}
        onAttachLink={vi.fn()}
        onRemoveAttachment={onRemoveAttachment}
        onProjectChange={vi.fn()}
      />,
    );

    expect(screen.getByText(/Attached file snapshot: notes.txt/)).toBeInTheDocument();
    expect(screen.getByText(/Captured file snapshot: draft.txt/)).toBeInTheDocument();

    const replaceFile = screen.getByRole("button", { name: "Replace file" });
    expect(replaceFile.textContent?.trim()).toBe("");
    fireEvent.mouseEnter(replaceFile.parentElement!);
    expect(within(replaceFile.parentElement!).getByRole("tooltip")).toHaveTextContent(
      "Replace file",
    );
    fireEvent.mouseLeave(replaceFile.parentElement!);
    fireEvent.click(replaceFile);
    expect(onAttach).toHaveBeenCalled();

    const remove = screen.getByRole("button", { name: "Remove attachment draft.txt" });
    expect(remove.textContent?.trim()).toBe("");
    fireEvent.mouseEnter(remove.parentElement!);
    expect(within(remove.parentElement!).getByRole("tooltip")).toHaveTextContent("Remove");
    fireEvent.mouseLeave(remove.parentElement!);
    fireEvent.click(remove);
    expect(onRemoveAttachment).toHaveBeenCalled();
  });

  it("exposes idle composer icon actions with accessible names, tooltips, and callbacks", () => {
    const onAttach = vi.fn();
    const onAttachFolder = vi.fn();
    const onSend = vi.fn();
    render(
      <AgentWorkspace
        title="Hello"
        projectId={null}
        projects={[]}
        modelLabel="GPT-5.5"
        modelOptions={[{ id: "gpt-5.5", displayName: "GPT-5.5" }]}
        selectedModelId="gpt-5.5"
        modelLocked={false}
        effortAvailable={false}
        effortValues={[]}
        selectedEffort={null}
        onEffortChange={() => undefined}
        onModelChange={() => undefined}
        turns={[]}
        events={[]}
        draft="Ask"
        pendingAttachment={null}
        connected
        sending={false}
        sendBlocked={false}
        cancelRequested={false}
        activeTurnId={null}
        errorMessage={null}
        onDraftChange={vi.fn()}
        onSend={onSend}
        onCancel={vi.fn()}
        onAttach={onAttach}
        onAttachFolder={onAttachFolder}
        onAttachLink={vi.fn()}
        onRemoveAttachment={vi.fn()}
        onProjectChange={vi.fn()}
      />,
    );

    const attachFile = screen.getByRole("button", { name: "Attach file" });
    expect(attachFile.textContent?.trim()).toBe("");
    fireEvent.mouseEnter(attachFile.parentElement!);
    expect(within(attachFile.parentElement!).getByRole("tooltip")).toHaveTextContent("Attach file");
    fireEvent.mouseLeave(attachFile.parentElement!);
    fireEvent.click(attachFile);
    expect(onAttach).toHaveBeenCalled();

    const attachFolder = screen.getByRole("button", { name: "Attach folder" });
    fireEvent.mouseEnter(attachFolder.parentElement!);
    expect(within(attachFolder.parentElement!).getByRole("tooltip")).toHaveTextContent(
      "Attach folder",
    );
    fireEvent.mouseLeave(attachFolder.parentElement!);
    fireEvent.click(attachFolder);
    expect(onAttachFolder).toHaveBeenCalled();

    const send = screen.getByRole("button", { name: "Send" });
    expect(send.textContent?.trim()).toBe("");
    fireEvent.mouseEnter(send.parentElement!);
    expect(within(send.parentElement!).getByRole("tooltip")).toHaveTextContent("Send");
    fireEvent.mouseLeave(send.parentElement!);
    fireEvent.click(send);
    expect(onSend).toHaveBeenCalled();

    expect(screen.queryByRole("button", { name: "Open Settings" })).not.toBeInTheDocument();
  });

  it("attaches a link from the composer entry without submitting the send form", () => {
    const onAttachLink = vi.fn();
    const onSend = vi.fn();
    render(
      <AgentWorkspace
        title="Hello"
        projectId={null}
        projects={[]}
        modelLabel="GPT-5.5"
        modelOptions={[{ id: "gpt-5.5", displayName: "GPT-5.5" }]}
        selectedModelId="gpt-5.5"
        modelLocked={false}
        effortAvailable={false}
        effortValues={[]}
        selectedEffort={null}
        onEffortChange={() => undefined}
        onModelChange={() => undefined}
        turns={[]}
        events={[]}
        draft="Ask with context"
        pendingAttachment={null}
        connected
        sending={false}
        sendBlocked={false}
        cancelRequested={false}
        activeTurnId={null}
        errorMessage={null}
        onDraftChange={vi.fn()}
        onSend={onSend}
        onCancel={vi.fn()}
        onAttach={vi.fn()}
        onAttachFolder={vi.fn()}
        onAttachLink={onAttachLink}
        onRemoveAttachment={vi.fn()}
        onProjectChange={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Attach link" }));
    const linkField = screen.getByLabelText("HTTPS link to attach");
    fireEvent.change(linkField, { target: { value: "https://example.com/readme.txt" } });
    fireEvent.click(screen.getByRole("button", { name: "Fetch" }));

    expect(onAttachLink).toHaveBeenCalledWith("https://example.com/readme.txt");
    expect(onSend).not.toHaveBeenCalled();
    expect(screen.queryByLabelText("HTTPS link to attach")).not.toBeInTheDocument();
  });

  it("keeps Cancel and Sending as visible text while streaming", () => {
    render(
      <AgentWorkspace
        title="Hello"
        projectId={null}
        projects={[]}
        modelLabel="GPT-5.5"
        modelOptions={[{ id: "gpt-5.5", displayName: "GPT-5.5" }]}
        selectedModelId="gpt-5.5"
        modelLocked={false}
        effortAvailable={false}
        effortValues={[]}
        selectedEffort={null}
        onEffortChange={() => undefined}
        onModelChange={() => undefined}
        turns={[baseTurn]}
        events={[]}
        draft="Ask"
        pendingAttachment={null}
        connected
        sending
        sendBlocked={false}
        cancelRequested={false}
        activeTurnId="t1"
        errorMessage={null}
        onDraftChange={vi.fn()}
        onSend={vi.fn()}
        onCancel={vi.fn()}
        onAttach={vi.fn()}
        onAttachFolder={vi.fn()}
        onAttachLink={vi.fn()}
        onRemoveAttachment={vi.fn()}
        onProjectChange={vi.fn()}
      />,
    );

    expect(screen.getByRole("button", { name: "Cancel" })).toHaveTextContent("Cancel");
    expect(screen.getByRole("button", { name: "Sending…" })).toHaveTextContent("Sending…");
  });

  it("locks attachment actions while a send is in flight", () => {
    render(
      <AgentWorkspace
        title="Hello"
        projectId={null}
        projects={[]}
        modelLabel="GPT-5.5"
        modelOptions={[{ id: "gpt-5.5", displayName: "GPT-5.5" }]}
        selectedModelId="gpt-5.5"
        modelLocked={false}
        effortAvailable={false}
        effortValues={[]}
        selectedEffort={null}
        onEffortChange={() => undefined}
        onModelChange={() => undefined}
        turns={[baseTurn]}
        events={[]}
        draft="Ask"
        pendingAttachment={{
          draftHandle: "handle",
          displayName: "draft.txt",
          byteCount: 4,
          originKind: "local_text_file",
          memberCount: 1,
          canonicalUrl: null,
        }}
        connected
        sending
        sendBlocked={false}
        cancelRequested={false}
        activeTurnId="t1"
        errorMessage={null}
        onDraftChange={vi.fn()}
        onSend={vi.fn()}
        onCancel={vi.fn()}
        onAttach={vi.fn()}
        onAttachFolder={vi.fn()}
        onAttachLink={vi.fn()}
        onRemoveAttachment={vi.fn()}
        onProjectChange={vi.fn()}
      />,
    );

    expect(screen.getByRole("button", { name: "Replace file" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Replace with folder" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Remove attachment draft.txt" })).toBeDisabled();
  });

  it("labels cross-kind replace actions when a pending attachment already exists", () => {
    render(
      <AgentWorkspace
        title="Hello"
        projectId={null}
        projects={[]}
        modelLabel="GPT-5.5"
        modelOptions={[{ id: "gpt-5.5", displayName: "GPT-5.5" }]}
        selectedModelId="gpt-5.5"
        modelLocked={false}
        effortAvailable={false}
        effortValues={[]}
        selectedEffort={null}
        onEffortChange={() => undefined}
        onModelChange={() => undefined}
        turns={[]}
        events={[]}
        draft="Ask"
        pendingAttachment={{
          draftHandle: "handle",
          displayName: "draft.txt",
          byteCount: 4,
          originKind: "local_text_file",
          memberCount: 1,
          canonicalUrl: null,
        }}
        connected
        sending={false}
        sendBlocked={false}
        cancelRequested={false}
        activeTurnId={null}
        errorMessage={null}
        onDraftChange={vi.fn()}
        onSend={vi.fn()}
        onCancel={vi.fn()}
        onAttach={vi.fn()}
        onAttachFolder={vi.fn()}
        onAttachLink={vi.fn()}
        onRemoveAttachment={vi.fn()}
        onProjectChange={vi.fn()}
      />,
    );

    expect(screen.getByRole("button", { name: "Replace file" })).toBeInTheDocument();
    const replaceWithFolder = screen.getByRole("button", { name: "Replace with folder" });
    fireEvent.mouseEnter(replaceWithFolder.parentElement!);
    expect(within(replaceWithFolder.parentElement!).getByRole("tooltip")).toHaveTextContent(
      "Replace with folder",
    );
  });

  it("hides the empty-session wordmark once a turn exists", () => {
    render(
      <AgentWorkspace
        title="New session"
        projectId={null}
        projects={[]}
        modelLabel="GPT-5.5"
        modelOptions={[{ id: "gpt-5.5", displayName: "GPT-5.5" }]}
        selectedModelId="gpt-5.5"
        modelLocked={false}
        effortAvailable={false}
        effortValues={[]}
        selectedEffort={null}
        onEffortChange={() => undefined}
        onModelChange={() => undefined}
        turns={[baseTurn]}
        events={[]}
        draft=""
        pendingAttachment={null}
        connected
        sending={false}
        sendBlocked={false}
        cancelRequested={false}
        activeTurnId={null}
        errorMessage={null}
        onDraftChange={vi.fn()}
        onSend={vi.fn()}
        onCancel={vi.fn()}
        onAttach={vi.fn()}
        onAttachFolder={vi.fn()}
        onAttachLink={vi.fn()}
        onRemoveAttachment={vi.fn()}
        onProjectChange={vi.fn()}
      />,
    );

    expect(screen.queryByRole("img", { name: "TULE" })).not.toBeInTheDocument();
  });

  it("sends on Enter and cancels while streaming", () => {
    const onSend = vi.fn();
    const onCancel = vi.fn();
    const { rerender } = render(
      <AgentWorkspace
        title="Hello"
        projectId={null}
        projects={[{ id: "p1", displayName: "Research" }]}
        modelLabel="GPT-5.5"
        modelOptions={[{ id: "gpt-5.5", displayName: "GPT-5.5" }]}
        selectedModelId="gpt-5.5"
        modelLocked={false}
        effortAvailable={false}
        effortValues={[]}
        selectedEffort={null}
        onEffortChange={() => undefined}
        onModelChange={() => undefined}
        turns={[{ ...baseTurn, state: "streaming", agentText: "Hel" }]}
        events={[]}
        draft="Next"
        pendingAttachment={null}
        connected
        sending={false}
        sendBlocked={false}
        cancelRequested={false}
        activeTurnId={null}
        errorMessage={null}
        onDraftChange={vi.fn()}
        onSend={onSend}
        onCancel={onCancel}
        onAttach={vi.fn()}
        onAttachFolder={vi.fn()}
        onAttachLink={vi.fn()}
        onRemoveAttachment={vi.fn()}
        onProjectChange={vi.fn()}
      />,
    );

    fireEvent.keyDown(screen.getByLabelText("Message the Agent"), {
      key: "Enter",
      shiftKey: false,
    });
    expect(onSend).toHaveBeenCalled();

    rerender(
      <AgentWorkspace
        title="Hello"
        projectId="p1"
        projects={[{ id: "p1", displayName: "Research" }]}
        modelLabel="GPT-5.5"
        modelOptions={[{ id: "gpt-5.5", displayName: "GPT-5.5" }]}
        selectedModelId="gpt-5.5"
        modelLocked={false}
        effortAvailable={false}
        effortValues={[]}
        selectedEffort={null}
        onEffortChange={() => undefined}
        onModelChange={() => undefined}
        turns={[{ ...baseTurn, state: "streaming", agentText: "Hello" }]}
        events={[]}
        draft=""
        pendingAttachment={null}
        connected
        sending
        sendBlocked={false}
        cancelRequested={false}
        activeTurnId="t1"
        errorMessage={null}
        onDraftChange={vi.fn()}
        onSend={onSend}
        onCancel={onCancel}
        onAttach={vi.fn()}
        onAttachFolder={vi.fn()}
        onAttachLink={vi.fn()}
        onRemoveAttachment={vi.fn()}
        onProjectChange={vi.fn()}
      />,
    );

    expect(screen.getByText("Research")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onCancel).toHaveBeenCalled();
  });

  it("documents composer height bounds and grows the textarea within them", () => {
    expect(COMPOSER_MIN_HEIGHT_PX).toBe(56);
    expect(COMPOSER_MAX_HEIGHT_PX).toBe(160);

    render(
      <AgentWorkspace
        title="Hello"
        projectId={null}
        projects={[]}
        modelLabel="GPT-5.5"
        modelOptions={[{ id: "gpt-5.5", displayName: "GPT-5.5" }]}
        selectedModelId="gpt-5.5"
        modelLocked={false}
        effortAvailable={false}
        effortValues={[]}
        selectedEffort={null}
        onEffortChange={() => undefined}
        onModelChange={() => undefined}
        turns={[]}
        events={[]}
        draft={"line\n".repeat(20)}
        pendingAttachment={null}
        connected
        sending={false}
        sendBlocked={false}
        cancelRequested={false}
        activeTurnId={null}
        errorMessage={null}
        onDraftChange={vi.fn()}
        onSend={vi.fn()}
        onCancel={vi.fn()}
        onAttach={vi.fn()}
        onAttachFolder={vi.fn()}
        onAttachLink={vi.fn()}
        onRemoveAttachment={vi.fn()}
        onProjectChange={vi.fn()}
      />,
    );

    const composer = screen.getByRole("textbox", { name: "Message the Agent" });
    expect(Number.parseInt(composer.style.height, 10)).toBeLessThanOrEqual(COMPOSER_MAX_HEIGHT_PX);
    expect(Number.parseInt(composer.style.height, 10)).toBeGreaterThanOrEqual(
      COMPOSER_MIN_HEIGHT_PX,
    );
  });

  it("follows the bottom while streaming, pauses after upward scroll, jumps on request, and preserves position on completion", () => {
    const metrics = { scrollHeight: 1000, clientHeight: 200, scrollTop: 800 };
    const { rerender } = render(
      <AgentWorkspace
        title="Hello"
        projectId={null}
        projects={[]}
        modelLabel="GPT-5.5"
        modelOptions={[{ id: "gpt-5.5", displayName: "GPT-5.5" }]}
        selectedModelId="gpt-5.5"
        modelLocked={false}
        effortAvailable={false}
        effortValues={[]}
        selectedEffort={null}
        onEffortChange={() => undefined}
        onModelChange={() => undefined}
        turns={[{ ...baseTurn, state: "streaming", agentText: "Hello" }]}
        events={[]}
        draft=""
        pendingAttachment={null}
        connected
        sending
        sendBlocked={false}
        cancelRequested={false}
        activeTurnId="t1"
        errorMessage={null}
        onDraftChange={vi.fn()}
        onSend={vi.fn()}
        onCancel={vi.fn()}
        onAttach={vi.fn()}
        onAttachFolder={vi.fn()}
        onAttachLink={vi.fn()}
        onRemoveAttachment={vi.fn()}
        onProjectChange={vi.fn()}
      />,
    );

    const transcript = document.querySelector(".transcript");
    expect(transcript).not.toBeNull();
    stubTranscriptMetrics(transcript as HTMLElement, metrics);

    // Bottom-follow while streaming continues.
    metrics.scrollHeight = 1200;
    rerender(
      <AgentWorkspace
        title="Hello"
        projectId={null}
        projects={[]}
        modelLabel="GPT-5.5"
        modelOptions={[{ id: "gpt-5.5", displayName: "GPT-5.5" }]}
        selectedModelId="gpt-5.5"
        modelLocked={false}
        effortAvailable={false}
        effortValues={[]}
        selectedEffort={null}
        onEffortChange={() => undefined}
        onModelChange={() => undefined}
        turns={[{ ...baseTurn, state: "streaming", agentText: "Hello there" }]}
        events={[]}
        draft=""
        pendingAttachment={null}
        connected
        sending
        sendBlocked={false}
        cancelRequested={false}
        activeTurnId="t1"
        errorMessage={null}
        onDraftChange={vi.fn()}
        onSend={vi.fn()}
        onCancel={vi.fn()}
        onAttach={vi.fn()}
        onAttachFolder={vi.fn()}
        onAttachLink={vi.fn()}
        onRemoveAttachment={vi.fn()}
        onProjectChange={vi.fn()}
      />,
    );
    expect(metrics.scrollTop).toBe(1200);

    // Deliberate upward scroll pauses follow and exposes Jump to latest.
    metrics.scrollTop = 100;
    fireEvent.scroll(transcript!);
    const jump = screen.getByRole("button", { name: "Jump to latest" });
    expect(jump).toBeInTheDocument();
    expect(jump.textContent?.trim()).toBe("");
    fireEvent.mouseEnter(jump.parentElement!);
    expect(screen.getByRole("tooltip")).toHaveTextContent("Jump to latest");

    metrics.scrollHeight = 1400;
    rerender(
      <AgentWorkspace
        title="Hello"
        projectId={null}
        projects={[]}
        modelLabel="GPT-5.5"
        modelOptions={[{ id: "gpt-5.5", displayName: "GPT-5.5" }]}
        selectedModelId="gpt-5.5"
        modelLocked={false}
        effortAvailable={false}
        effortValues={[]}
        selectedEffort={null}
        onEffortChange={() => undefined}
        onModelChange={() => undefined}
        turns={[{ ...baseTurn, state: "streaming", agentText: "Hello there friend" }]}
        events={[]}
        draft=""
        pendingAttachment={null}
        connected
        sending
        sendBlocked={false}
        cancelRequested={false}
        activeTurnId="t1"
        errorMessage={null}
        onDraftChange={vi.fn()}
        onSend={vi.fn()}
        onCancel={vi.fn()}
        onAttach={vi.fn()}
        onAttachFolder={vi.fn()}
        onAttachLink={vi.fn()}
        onRemoveAttachment={vi.fn()}
        onProjectChange={vi.fn()}
      />,
    );
    expect(metrics.scrollTop).toBe(100);

    // Jump to latest restores follow.
    fireEvent.click(screen.getByRole("button", { name: "Jump to latest" }));
    expect(metrics.scrollTop).toBe(1400);
    expect(screen.queryByRole("button", { name: "Jump to latest" })).not.toBeInTheDocument();

    // After another upward scroll, completion must not force the reading position.
    metrics.scrollTop = 220;
    fireEvent.scroll(transcript!);
    expect(screen.getByRole("button", { name: "Jump to latest" })).toBeInTheDocument();

    rerender(
      <AgentWorkspace
        title="Hello"
        projectId={null}
        projects={[]}
        modelLabel="GPT-5.5"
        modelOptions={[{ id: "gpt-5.5", displayName: "GPT-5.5" }]}
        selectedModelId="gpt-5.5"
        modelLocked={false}
        effortAvailable={false}
        effortValues={[]}
        selectedEffort={null}
        onEffortChange={() => undefined}
        onModelChange={() => undefined}
        turns={[{ ...baseTurn, state: "completed", agentText: "Hello there friend" }]}
        events={[]}
        draft=""
        pendingAttachment={null}
        connected
        sending={false}
        sendBlocked={false}
        cancelRequested={false}
        activeTurnId={null}
        errorMessage={null}
        onDraftChange={vi.fn()}
        onSend={vi.fn()}
        onCancel={vi.fn()}
        onAttach={vi.fn()}
        onAttachFolder={vi.fn()}
        onAttachLink={vi.fn()}
        onRemoveAttachment={vi.fn()}
        onProjectChange={vi.fn()}
      />,
    );
    expect(metrics.scrollTop).toBe(220);
    expect(screen.getByRole("button", { name: "Jump to latest" })).toBeInTheDocument();
  });

  it("changes optional Project context and renders only safe failure copy", () => {
    const onProjectChange = vi.fn();
    render(
      <AgentWorkspace
        title="Hello"
        projectId="p1"
        projects={[
          { id: "p1", displayName: "Research" },
          { id: "p2", displayName: "Atlas" },
        ]}
        modelLabel="GPT-5.5"
        modelOptions={[{ id: "gpt-5.5", displayName: "GPT-5.5" }]}
        selectedModelId="gpt-5.5"
        modelLocked={false}
        effortAvailable={false}
        effortValues={[]}
        selectedEffort={null}
        onEffortChange={() => undefined}
        onModelChange={() => undefined}
        turns={[
          {
            id: "t1",
            ordinal: 1,
            userText: "Hi",
            agentText: "",
            state: "failed",
            errorCode: "provider_unavailable",
            effort: null,
            sources: [],
          },
        ]}
        events={[]}
        draft=""
        pendingAttachment={null}
        connected
        sending={false}
        sendBlocked={false}
        cancelRequested={false}
        activeTurnId={null}
        errorMessage={null}
        onDraftChange={vi.fn()}
        onSend={vi.fn()}
        onCancel={vi.fn()}
        onAttach={vi.fn()}
        onAttachFolder={vi.fn()}
        onAttachLink={vi.fn()}
        onRemoveAttachment={vi.fn()}
        onProjectChange={onProjectChange}
      />,
    );

    fireEvent.change(screen.getByRole("combobox", { name: "Project context" }), {
      target: { value: "p2" },
    });
    expect(onProjectChange).toHaveBeenCalledWith("p2");
    expect(screen.getByText("The provider is unavailable. Try again.")).toBeInTheDocument();
    expect(screen.queryByText("provider_unavailable")).not.toBeInTheDocument();
  });
});
