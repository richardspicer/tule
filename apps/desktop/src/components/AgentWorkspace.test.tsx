import { fireEvent, render, screen } from "@testing-library/react";
import { createRef } from "react";
import { describe, expect, it, vi } from "vitest";
import { AgentWorkspace } from "./AgentWorkspace";

describe("AgentWorkspace", () => {
  it("blocks composer when disconnected and opens settings", () => {
    const onOpenSettings = vi.fn();
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
        onOpenSettings={onOpenSettings}
        settingsButtonRef={createRef<HTMLButtonElement>()}
      />,
    );

    expect(
      screen.getByText("Connect ChatGPT in Settings to message the Agent."),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Open Settings" }));
    expect(onOpenSettings).toHaveBeenCalled();
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
        onOpenSettings={vi.fn()}
        settingsButtonRef={createRef<HTMLButtonElement>()}
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
        onOpenSettings={vi.fn()}
        settingsButtonRef={createRef<HTMLButtonElement>()}
      />,
    );

    expect(screen.getByText("Research")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onCancel).toHaveBeenCalled();
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
        onOpenSettings={vi.fn()}
        settingsButtonRef={createRef<HTMLButtonElement>()}
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
