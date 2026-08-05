import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { AgentWorkspace, COMPOSER_MAX_HEIGHT_PX, COMPOSER_MIN_HEIGHT_PX } from "./AgentWorkspace";
import type { AgentTurn } from "../platform/agents";

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
  sources: [],
};

describe("AgentWorkspace", () => {
  it("blocks composer when disconnected and deep-links Providers settings", () => {
    const onOpenProvidersSettings = vi.fn();
    render(
      <AgentWorkspace
        title="New session"
        projectId={null}
        projects={[]}
        modelLabel="GPT-5.5"
        modelOptions={[{ id: "gpt-5.5", displayName: "GPT-5.5" }]}
        selectedModelId="gpt-5.5"
        modelLocked={false}
        onModelChange={() => undefined}
        turns={[]}
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
        onRemoveAttachment={vi.fn()}
        onProjectChange={vi.fn()}
        onOpenProvidersSettings={onOpenProvidersSettings}
      />,
    );

    expect(screen.getByRole("img", { name: "TULE" })).toBeInTheDocument();
    expect(
      screen.getByText("Connect ChatGPT in Settings to message the Agent."),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Open Settings" }));
    expect(onOpenProvidersSettings).toHaveBeenCalled();
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
              },
            ],
          },
        ]}
        draft="Ask"
        pendingAttachment={{
          draftHandle: "handle",
          displayName: "draft.txt",
          byteCount: 4,
          originKind: "local_text_file",
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
        onRemoveAttachment={onRemoveAttachment}
        onProjectChange={vi.fn()}
        onOpenProvidersSettings={vi.fn()}
      />,
    );

    expect(screen.getByText(/Attached snapshot: notes.txt/)).toBeInTheDocument();
    expect(screen.getByText(/Captured snapshot: draft.txt/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Replace the attached text file" }));
    expect(onAttach).toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Remove attachment draft.txt" }));
    expect(onRemoveAttachment).toHaveBeenCalled();
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
        onModelChange={() => undefined}
        turns={[baseTurn]}
        draft="Ask"
        pendingAttachment={{
          draftHandle: "handle",
          displayName: "draft.txt",
          byteCount: 4,
          originKind: "local_text_file",
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
        onRemoveAttachment={vi.fn()}
        onProjectChange={vi.fn()}
        onOpenProvidersSettings={vi.fn()}
      />,
    );

    expect(screen.getByRole("button", { name: "Replace the attached text file" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Remove attachment draft.txt" })).toBeDisabled();
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
        onModelChange={() => undefined}
        turns={[baseTurn]}
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
        onRemoveAttachment={vi.fn()}
        onProjectChange={vi.fn()}
        onOpenProvidersSettings={vi.fn()}
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
        onModelChange={() => undefined}
        turns={[{ ...baseTurn, state: "streaming", agentText: "Hel" }]}
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
        onRemoveAttachment={vi.fn()}
        onProjectChange={vi.fn()}
        onOpenProvidersSettings={vi.fn()}
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
        onModelChange={() => undefined}
        turns={[{ ...baseTurn, state: "streaming", agentText: "Hello" }]}
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
        onRemoveAttachment={vi.fn()}
        onProjectChange={vi.fn()}
        onOpenProvidersSettings={vi.fn()}
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
        onModelChange={() => undefined}
        turns={[]}
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
        onRemoveAttachment={vi.fn()}
        onProjectChange={vi.fn()}
        onOpenProvidersSettings={vi.fn()}
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
        onModelChange={() => undefined}
        turns={[{ ...baseTurn, state: "streaming", agentText: "Hello" }]}
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
        onRemoveAttachment={vi.fn()}
        onProjectChange={vi.fn()}
        onOpenProvidersSettings={vi.fn()}
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
        onModelChange={() => undefined}
        turns={[{ ...baseTurn, state: "streaming", agentText: "Hello there" }]}
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
        onRemoveAttachment={vi.fn()}
        onProjectChange={vi.fn()}
        onOpenProvidersSettings={vi.fn()}
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
        onModelChange={() => undefined}
        turns={[{ ...baseTurn, state: "streaming", agentText: "Hello there friend" }]}
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
        onRemoveAttachment={vi.fn()}
        onProjectChange={vi.fn()}
        onOpenProvidersSettings={vi.fn()}
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
        onModelChange={() => undefined}
        turns={[{ ...baseTurn, state: "completed", agentText: "Hello there friend" }]}
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
        onRemoveAttachment={vi.fn()}
        onProjectChange={vi.fn()}
        onOpenProvidersSettings={vi.fn()}
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
        onModelChange={() => undefined}
        turns={[
          {
            id: "t1",
            ordinal: 1,
            userText: "Hi",
            agentText: "",
            state: "failed",
            errorCode: "provider_unavailable",
            sources: [],
          },
        ]}
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
        onRemoveAttachment={vi.fn()}
        onProjectChange={onProjectChange}
        onOpenProvidersSettings={vi.fn()}
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
