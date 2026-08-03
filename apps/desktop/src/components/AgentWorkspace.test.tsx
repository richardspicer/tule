import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { AgentWorkspace, COMPOSER_MAX_HEIGHT_PX, COMPOSER_MIN_HEIGHT_PX } from "./AgentWorkspace";

describe("AgentWorkspace", () => {
  it("blocks composer when disconnected and deep-links Connections settings", () => {
    const onOpenConnectionsSettings = vi.fn();
    render(
      <AgentWorkspace
        title="New session"
        projectId={null}
        projects={[]}
        modelLabel="GPT-5.5"
        turns={[]}
        draft=""
        connected={false}
        sending={false}
        sendBlocked={false}
        cancelRequested={false}
        activeTurnId={null}
        errorMessage={null}
        onDraftChange={vi.fn()}
        onSend={vi.fn()}
        onCancel={vi.fn()}
        onProjectChange={vi.fn()}
        onOpenConnectionsSettings={onOpenConnectionsSettings}
      />,
    );

    expect(screen.getByRole("img", { name: "TULE" })).toBeInTheDocument();
    expect(
      screen.getByText("Connect ChatGPT in Settings to message the Agent."),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Open Settings" }));
    expect(onOpenConnectionsSettings).toHaveBeenCalled();
  });

  it("hides the empty-session wordmark once a turn exists", () => {
    render(
      <AgentWorkspace
        title="New session"
        projectId={null}
        projects={[]}
        modelLabel="GPT-5.5"
        turns={[
          {
            id: "t1",
            ordinal: 1,
            userText: "Hi",
            agentText: "Hello",
            state: "completed",
            errorCode: null,
          },
        ]}
        draft=""
        connected
        sending={false}
        sendBlocked={false}
        cancelRequested={false}
        activeTurnId={null}
        errorMessage={null}
        onDraftChange={vi.fn()}
        onSend={vi.fn()}
        onCancel={vi.fn()}
        onProjectChange={vi.fn()}
        onOpenConnectionsSettings={vi.fn()}
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
        turns={[
          {
            id: "t1",
            ordinal: 1,
            userText: "Hi",
            agentText: "Hel",
            state: "streaming",
            errorCode: null,
          },
        ]}
        draft="Next"
        connected
        sending={false}
        sendBlocked={false}
        cancelRequested={false}
        activeTurnId={null}
        errorMessage={null}
        onDraftChange={vi.fn()}
        onSend={onSend}
        onCancel={onCancel}
        onProjectChange={vi.fn()}
        onOpenConnectionsSettings={vi.fn()}
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
        turns={[
          {
            id: "t1",
            ordinal: 1,
            userText: "Hi",
            agentText: "Hello",
            state: "streaming",
            errorCode: null,
          },
        ]}
        draft=""
        connected
        sending
        sendBlocked={false}
        cancelRequested={false}
        activeTurnId="t1"
        errorMessage={null}
        onDraftChange={vi.fn()}
        onSend={onSend}
        onCancel={onCancel}
        onProjectChange={vi.fn()}
        onOpenConnectionsSettings={vi.fn()}
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
        turns={[]}
        draft={"line\n".repeat(20)}
        connected
        sending={false}
        sendBlocked={false}
        cancelRequested={false}
        activeTurnId={null}
        errorMessage={null}
        onDraftChange={vi.fn()}
        onSend={vi.fn()}
        onCancel={vi.fn()}
        onProjectChange={vi.fn()}
        onOpenConnectionsSettings={vi.fn()}
      />,
    );

    const composer = screen.getByRole("textbox", { name: "Message the Agent" });
    expect(Number.parseInt(composer.style.height, 10)).toBeLessThanOrEqual(COMPOSER_MAX_HEIGHT_PX);
    expect(Number.parseInt(composer.style.height, 10)).toBeGreaterThanOrEqual(
      COMPOSER_MIN_HEIGHT_PX,
    );
  });

  it("shows Jump to latest after deliberate upward scroll and keeps the reading position", () => {
    render(
      <AgentWorkspace
        title="Hello"
        projectId={null}
        projects={[]}
        modelLabel="GPT-5.5"
        turns={[
          {
            id: "t1",
            ordinal: 1,
            userText: "Hi",
            agentText: "Hello",
            state: "completed",
            errorCode: null,
          },
        ]}
        draft=""
        connected
        sending={false}
        sendBlocked={false}
        cancelRequested={false}
        activeTurnId={null}
        errorMessage={null}
        onDraftChange={vi.fn()}
        onSend={vi.fn()}
        onCancel={vi.fn()}
        onProjectChange={vi.fn()}
        onOpenConnectionsSettings={vi.fn()}
      />,
    );

    const transcript = document.querySelector(".transcript");
    expect(transcript).not.toBeNull();
    Object.defineProperty(transcript, "scrollHeight", { configurable: true, value: 1000 });
    Object.defineProperty(transcript, "clientHeight", { configurable: true, value: 200 });
    Object.defineProperty(transcript, "scrollTop", {
      configurable: true,
      writable: true,
      value: 0,
    });
    fireEvent.scroll(transcript!);
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
        turns={[
          {
            id: "t1",
            ordinal: 1,
            userText: "Hi",
            agentText: "",
            state: "failed",
            errorCode: "provider_unavailable",
          },
        ]}
        draft=""
        connected
        sending={false}
        sendBlocked={false}
        cancelRequested={false}
        activeTurnId={null}
        errorMessage={null}
        onDraftChange={vi.fn()}
        onSend={vi.fn()}
        onCancel={vi.fn()}
        onProjectChange={onProjectChange}
        onOpenConnectionsSettings={vi.fn()}
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
