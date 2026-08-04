import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  AgentError,
  cancelAgentTurn,
  clearAgentTextSourceDraft,
  getAgentErrorCode,
  getAgentSession,
  getSafeAgentErrorMessage,
  listAgentSessions,
  pickAgentTextSource,
  sendAgentMessage,
  setAgentSourceDraftScope,
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

const validSource = {
  id: "01900000-0000-7000-8000-000000000001",
  originKind: "local_text_file",
  displayName: "notes.txt",
  byteCount: 5,
  contentSha256: "a".repeat(64),
};

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

  it("rejects non-allowlisted turn states, error codes, and source shapes", async () => {
    for (const turn of [
      { state: "secret-internal-state", errorCode: null, sources: [] },
      { state: "failed", errorCode: "raw-provider-error", sources: [] },
      {
        state: "completed",
        errorCode: null,
        sources: [{ ...validSource, contentSha256: "not-a-hash" }],
      },
      {
        state: "completed",
        errorCode: null,
        sources: [{ ...validSource, originKind: "folder" }],
      },
    ]) {
      invokeMock.mockResolvedValueOnce({
        session: {
          id: "s1",
          title: "Hello",
          projectId: null,
          modelId: "gpt-5.5",
        },
        turns: [
          {
            id: "t1",
            ordinal: 1,
            userText: "Hello",
            agentText: "",
            ...turn,
          },
        ],
      });

      await expect(getAgentSession("s1")).rejects.toMatchObject({
        code: "agent_storage_unavailable",
      });
    }
  });

  it("accepts allowlisted source metadata on reopened turns", async () => {
    invokeMock.mockResolvedValue({
      session: {
        id: "s1",
        title: "Hello",
        projectId: null,
        modelId: "gpt-5.5",
      },
      turns: [
        {
          id: "t1",
          ordinal: 1,
          userText: "Hello",
          agentText: "Hi",
          state: "completed",
          errorCode: null,
          sources: [validSource],
        },
      ],
    });

    const detail = await getAgentSession("s1");
    expect(detail.turns[0]?.sources).toEqual([validSource]);
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
            sources: [validSource],
          },
        });
        return Promise.resolve();
      },
    );

    await sendAgentMessage({
      sessionId: null,
      userText: "Hello",
      projectId: null,
      modelId: "gpt-5.5",
      sourceDraftHandle: "abcd",
      onEvent: (event) => events.push(event.kind),
    });

    expect(events).toEqual(["started", "delta", "terminal"]);
    expect(invokeMock).toHaveBeenCalledWith(
      "send_agent_message",
      expect.objectContaining({
        sessionId: null,
        userText: "Hello",
        projectId: null,
        modelId: "gpt-5.5",
        sourceDraftHandle: "abcd",
      }),
    );
  });

  it("validates pick and clear commands without exposing paths", async () => {
    invokeMock.mockResolvedValueOnce({
      status: "selected",
      draftHandle: "deadbeef".repeat(4),
      displayName: "notes.txt",
      byteCount: 12,
      originKind: "local_text_file",
    });
    await expect(pickAgentTextSource()).resolves.toEqual({
      status: "selected",
      attachment: {
        draftHandle: "deadbeef".repeat(4),
        displayName: "notes.txt",
        byteCount: 12,
        originKind: "local_text_file",
      },
    });

    invokeMock.mockResolvedValueOnce({
      status: "selected",
      draftHandle: "x",
      displayName: "notes.txt",
      byteCount: 12,
      originKind: "local_text_file",
      path: "C:\\\\secret",
    });
    await expect(pickAgentTextSource()).rejects.toMatchObject({
      code: "agent_storage_unavailable",
    });

    invokeMock.mockResolvedValueOnce(undefined);
    await clearAgentTextSourceDraft("handle");
    expect(invokeMock).toHaveBeenCalledWith("clear_agent_text_source_draft", {
      draftHandle: "handle",
    });
    invokeMock.mockResolvedValueOnce(undefined);
    await setAgentSourceDraftScope("session-1");
    expect(invokeMock).toHaveBeenCalledWith("set_agent_source_draft_scope", {
      scopeKey: "session-1",
    });
  });

  it("maps cancel and safe error copy including source failures", async () => {
    invokeMock.mockResolvedValue(undefined);
    await cancelAgentTurn("t1");
    expect(invokeMock).toHaveBeenCalledWith("cancel_agent_turn", { turnId: "t1" });
    expect(getAgentErrorCode(new AgentError("cancelled"))).toBe("cancelled");
    expect(getSafeAgentErrorMessage(new AgentError("cancelled"))).toBe(
      "TULE stopped receiving the response.",
    );
    expect(getSafeAgentErrorMessage(new AgentError("source_too_large"))).toBe(
      "That file is too large to attach.",
    );
    expect(getAgentErrorCode({ message: "source_draft_expired" })).toBe("source_draft_expired");
  });
});
