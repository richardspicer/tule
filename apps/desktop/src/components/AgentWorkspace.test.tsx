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
        projectLabel="No project"
        modelLabel="GPT-5.5"
        turns={[]}
        draft=""
        connected={false}
        sending={false}
        activeTurnId={null}
        errorMessage={null}
        onDraftChange={vi.fn()}
        onSend={vi.fn()}
        onCancel={vi.fn()}
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
        projectLabel="No project"
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
        activeTurnId={null}
        errorMessage={null}
        onDraftChange={vi.fn()}
        onSend={onSend}
        onCancel={onCancel}
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
        projectLabel="Research"
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
        activeTurnId="t1"
        errorMessage={null}
        onDraftChange={vi.fn()}
        onSend={onSend}
        onCancel={onCancel}
        onOpenSettings={vi.fn()}
        settingsButtonRef={createRef<HTMLButtonElement>()}
      />,
    );

    expect(screen.getByText("Research")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onCancel).toHaveBeenCalled();
  });
});
