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
  pickAgentTextFolderSource,
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
  memberCount: 1,
};

const validFolderSource = {
  ...validSource,
  originKind: "local_text_folder" as const,
  displayName: "docs",
  memberCount: 2,
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
    const hostileSources = [
      { ...validSource, id: "not-a-uuid" },
      { ...validSource, id: "01900000-0000-4000-8000-000000000001" },
      { ...validSource, id: "01900000-0000-7000-c000-000000000001" },
      { ...validSource, originKind: "folder" },
      { ...validSource, memberCount: 0 },
      { ...validSource, memberCount: 33 },
      { ...validFolderSource, memberCount: 1.5 },
      { ...validSource, displayName: "" },
      { ...validSource, displayName: "bad\nname" },
      { ...validSource, displayName: "bad\u202Ename" },
      { ...validSource, byteCount: -1 },
      { ...validSource, byteCount: 1.5 },
      { ...validSource, byteCount: 64 * 1024 + 1 },
      { ...validSource, contentSha256: "not-a-hash" },
      { ...validSource, contentSha256: "A".repeat(64) },
      { ...validSource, contentSha256: "a".repeat(63) },
      { ...validSource, path: "C:\\\\secret" },
      {
        id: validSource.id,
        originKind: validSource.originKind,
        displayName: validSource.displayName,
        byteCount: validSource.byteCount,
        // missing contentSha256
      },
    ];

    for (const turn of [
      { state: "secret-internal-state", errorCode: null, sources: [] },
      { state: "failed", errorCode: "raw-provider-error", sources: [] },
      ...hostileSources.map((sources) => ({
        state: "completed" as const,
        errorCode: null,
        sources: [sources],
      })),
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
      memberCount: 1,
    });
    await expect(pickAgentTextSource()).resolves.toEqual({
      status: "selected",
      attachment: {
        draftHandle: "deadbeef".repeat(4),
        displayName: "notes.txt",
        byteCount: 12,
        originKind: "local_text_file",
        memberCount: 1,
      },
    });

    invokeMock.mockResolvedValueOnce({
      status: "selected",
      draftHandle: "deadbeef".repeat(4),
      displayName: "docs",
      byteCount: 120,
      originKind: "local_text_folder",
      memberCount: 3,
    });
    await expect(pickAgentTextFolderSource()).resolves.toEqual({
      status: "selected",
      attachment: {
        draftHandle: "deadbeef".repeat(4),
        displayName: "docs",
        byteCount: 120,
        originKind: "local_text_folder",
        memberCount: 3,
      },
    });

    invokeMock.mockResolvedValueOnce({
      status: "cancelled",
      draftHandle: null,
      displayName: null,
      byteCount: null,
      originKind: null,
      memberCount: null,
    });
    await expect(pickAgentTextSource()).resolves.toEqual({ status: "cancelled" });

    const hostilePicks = [
      {
        status: "selected",
        draftHandle: "x",
        displayName: "notes.txt",
        byteCount: 12,
        originKind: "local_text_file",
      },
      {
        status: "selected",
        draftHandle: "DEADBEEF".repeat(4),
        displayName: "notes.txt",
        byteCount: 12,
        originKind: "local_text_file",
      },
      {
        status: "selected",
        draftHandle: "deadbeef".repeat(4),
        displayName: "",
        byteCount: 12,
        originKind: "local_text_file",
      },
      {
        status: "selected",
        draftHandle: "deadbeef".repeat(4),
        displayName: "bad\nname",
        byteCount: 12,
        originKind: "local_text_file",
      },
      {
        status: "selected",
        draftHandle: "deadbeef".repeat(4),
        displayName: "notes.txt",
        byteCount: 12.5,
        originKind: "local_text_file",
      },
      {
        status: "selected",
        draftHandle: "deadbeef".repeat(4),
        displayName: "notes.txt",
        byteCount: 64 * 1024 + 1,
        originKind: "local_text_file",
        memberCount: 1,
      },
      {
        status: "selected",
        draftHandle: "deadbeef".repeat(4),
        displayName: "docs",
        byteCount: 12,
        originKind: "local_text_folder",
        memberCount: 0,
      },
      {
        status: "selected",
        draftHandle: "deadbeef".repeat(4),
        displayName: "notes.txt",
        byteCount: 12,
        originKind: "local_text_file",
        path: "C:\\\\secret",
      },
      {
        status: "cancelled",
        draftHandle: "deadbeef".repeat(4),
        displayName: null,
        byteCount: null,
        originKind: null,
        memberCount: null,
      },
      {
        status: "cancelled",
        draftHandle: null,
        displayName: null,
        byteCount: null,
        originKind: null,
        memberCount: null,
        path: "C:\\\\secret",
      },
      { status: "cancelled" },
      {
        status: "other",
        draftHandle: null,
        displayName: null,
        byteCount: null,
        originKind: null,
        memberCount: null,
      },
    ];
    for (const payload of hostilePicks) {
      invokeMock.mockResolvedValueOnce(payload);
      await expect(pickAgentTextSource()).rejects.toMatchObject({
        code: "agent_storage_unavailable",
      });
    }

    invokeMock.mockResolvedValueOnce(undefined);
    await clearAgentTextSourceDraft("handle");
    expect(invokeMock).toHaveBeenCalledWith("clear_agent_text_source_draft", {
      draftHandle: "handle",
    });
    invokeMock.mockResolvedValueOnce(undefined);
    await setAgentSourceDraftScope("01900000-0000-7000-8000-000000000001");
    expect(invokeMock).toHaveBeenCalledWith("set_agent_source_draft_scope", {
      sessionId: "01900000-0000-7000-8000-000000000001",
    });
    invokeMock.mockResolvedValueOnce(undefined);
    await setAgentSourceDraftScope(null);
    expect(invokeMock).toHaveBeenCalledWith("set_agent_source_draft_scope", {
      sessionId: null,
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
      "That attachment is too large.",
    );
    expect(getAgentErrorCode({ message: "source_draft_expired" })).toBe("source_draft_expired");
  });
});
