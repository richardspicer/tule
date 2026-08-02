import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  AgentError,
  cancelAgentTurn,
  getAgentErrorCode,
  getSafeAgentErrorMessage,
  listAgentSessions,
  sendAgentMessage,
} from "./agents";

const { channelCtor, invokeMock } = vi.hoisted(() => {
  const invokeMock = vi.fn();
  const channelCtor = vi.fn(function Channel(this: { onmessage?: unknown }, onmessage?: unknown) {
    this.onmessage = onmessage;
  });
  return { channelCtor, invokeMock };
});

vi.mock("@tauri-apps/api/core", () => ({
  Channel: channelCtor,
  invoke: invokeMock,
}));

describe("agents platform", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    channelCtor.mockClear();
  });

  it("validates session list payloads", async () => {
    invokeMock.mockResolvedValue([
      {
        id: "s1",
        title: "Hello",
        projectId: null,
        modelId: "gpt-5.5",
      },
    ]);

    await expect(listAgentSessions()).resolves.toHaveLength(1);
  });

  it("rejects malformed sessions", async () => {
    invokeMock.mockResolvedValue([{ id: 1 }]);
    await expect(listAgentSessions()).rejects.toMatchObject({
      code: "agent_storage_unavailable",
    });
  });

  it("sends through a typed channel and preserves event order", async () => {
    const events: string[] = [];
    invokeMock.mockImplementation(
      (_command: string, args: { channel: { onmessage: (v: unknown) => void } }) => {
        args.channel.onmessage({
          kind: "started",
          session_id: "s1",
          turn_id: "t1",
        });
        args.channel.onmessage({ kind: "delta", turn_id: "t1", text: "Hi" });
        args.channel.onmessage({
          kind: "terminal",
          turn: {
            id: "t1",
            ordinal: 1,
            userText: "Hello",
            agentText: "Hi",
            state: "completed",
            errorCode: null,
          },
        });
        return Promise.resolve();
      },
    );

    await sendAgentMessage({
      sessionId: null,
      userText: "Hello",
      projectId: null,
      onEvent: (event) => events.push(event.kind),
    });

    expect(events).toEqual(["started", "delta", "terminal"]);
    expect(invokeMock).toHaveBeenCalledWith(
      "send_agent_message",
      expect.objectContaining({
        sessionId: null,
        userText: "Hello",
        projectId: null,
      }),
    );
  });

  it("maps cancel and safe error copy", async () => {
    invokeMock.mockResolvedValue(undefined);
    await cancelAgentTurn("t1");
    expect(invokeMock).toHaveBeenCalledWith("cancel_agent_turn", { turnId: "t1" });
    expect(getAgentErrorCode(new AgentError("cancelled"))).toBe("cancelled");
    expect(getSafeAgentErrorMessage(new AgentError("cancelled"))).toBe(
      "TULE stopped receiving the response.",
    );
  });
});
