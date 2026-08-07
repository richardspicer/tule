import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  AgentError,
  cancelAgentTurn,
  clearAgentTextSourceDraft,
  createArtifactFromTurn,
  getAgentErrorCode,
  getAgentSession,
  getArtifact,
  getSafeAgentErrorMessage,
  listAgentSessions,
  listArtifacts,
  pickAgentTextSource,
  pickAgentTextFolderSource,
  attachAgentTextLinkSource,
  getModelRequestControls,
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
  canonicalUrl: null,
};

const validSessionDetail = {
  session: {
    id: "01900000-0000-7000-8000-000000000010",
    title: "Hello",
    projectId: null,
    modelId: "gpt-5.5",
  },
  turns: [] as const,
  events: [] as const,
};

const validEvent = {
  id: "01900000-0000-7000-8000-000000000020",
  sessionId: validSessionDetail.session.id,
  turnId: "01900000-0000-7000-8000-000000000011",
  sequence: 2,
  kind: "turn_cancelled" as const,
  createdAtUnixMs: 1_700_000_000_000,
};

const validFolderSource = {
  ...validSource,
  originKind: "local_text_folder" as const,
  displayName: "docs",
  memberCount: 2,
};

const validLinkSource = {
  ...validSource,
  originKind: "remote_text_url" as const,
  displayName: "example.com/readme.txt",
  canonicalUrl: "https://example.com/readme.txt",
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
      { ...validLinkSource, canonicalUrl: null },
      { ...validLinkSource, canonicalUrl: "http://example.com/readme.txt" },
      { ...validLinkSource, canonicalUrl: "https:///readme.txt" },
      { ...validLinkSource, canonicalUrl: "https://example.com/" + "\u{10000}".repeat(513) },
      { ...validSource, canonicalUrl: "https://example.com/readme.txt" },
      { ...validFolderSource, canonicalUrl: "https://example.com/readme.txt" },
      {
        id: validSource.id,
        originKind: validSource.originKind,
        displayName: validSource.displayName,
        byteCount: validSource.byteCount,
        // missing contentSha256
      },
    ];

    for (const turn of [
      { state: "secret-internal-state", errorCode: null, effort: null, sources: [] },
      { state: "failed", errorCode: "raw-provider-error", effort: null, sources: [] },
      { state: "completed", errorCode: null, effort: "xhigh", sources: [] },
      ...hostileSources.map((sources) => ({
        state: "completed" as const,
        errorCode: null,
        effort: null,
        sources: [sources],
      })),
    ]) {
      invokeMock.mockResolvedValueOnce({
        ...validSessionDetail,
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

      await expect(getAgentSession(validSessionDetail.session.id)).rejects.toMatchObject({
        code: "agent_storage_unavailable",
      });
    }
  });

  it("accepts allowlisted source metadata on reopened turns", async () => {
    invokeMock.mockResolvedValue({
      ...validSessionDetail,
      turns: [
        {
          id: "t1",
          ordinal: 1,
          userText: "Hello",
          agentText: "Hi",
          state: "completed",
          errorCode: null,
          effort: null,
          sources: [validSource, validLinkSource],
        },
      ],
    });

    const detail = await getAgentSession(validSessionDetail.session.id);
    expect(detail.turns[0]?.sources).toEqual([validSource, validLinkSource]);
  });

  it("accepts allowlisted session events and rejects unknown kinds or malformed objects", async () => {
    invokeMock.mockResolvedValue({
      ...validSessionDetail,
      events: [
        {
          id: "01900000-0000-7000-8000-000000000021",
          sessionId: validSessionDetail.session.id,
          turnId: null,
          sequence: 0,
          kind: "session_created",
          createdAtUnixMs: 1_700_000_000_000,
        },
        validEvent,
      ],
    });

    const detail = await getAgentSession(validSessionDetail.session.id);
    expect(detail.events).toHaveLength(2);
    expect(detail.events[1]?.kind).toBe("turn_cancelled");

    const hostileEvents = [
      { ...validEvent, kind: "turn_archived" },
      { ...validEvent, id: "not-a-uuid" },
      { ...validEvent, turnId: "01900000-0000-4000-8000-000000000011" },
      { ...validEvent, sequence: -1 },
      { ...validEvent, sequence: 1.5 },
      { ...validEvent, sequence: Number.MAX_SAFE_INTEGER + 1 },
      { ...validEvent, createdAtUnixMs: 1.5 },
      { ...validEvent, createdAtUnixMs: -1 },
      { ...validEvent, createdAtUnixMs: Number.MAX_SAFE_INTEGER + 1 },
      { ...validEvent, createdAtUnixMs: 8_640_000_000_000_001 },
      { ...validEvent, payload: "secret" },
      {
        id: validEvent.id,
        sessionId: validEvent.sessionId,
        turnId: validEvent.turnId,
        sequence: validEvent.sequence,
        kind: validEvent.kind,
      },
    ];

    for (const event of hostileEvents) {
      invokeMock.mockResolvedValueOnce({
        ...validSessionDetail,
        events: [event],
      });
      await expect(getAgentSession(validSessionDetail.session.id)).rejects.toMatchObject({
        code: "agent_storage_unavailable",
      });
    }
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
            effort: null,
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
      effort: null,
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
        effort: null,
        sourceDraftHandle: "abcd",
      }),
    );
  });

  it("validates model request controls and fails closed on speed or unknown effort", async () => {
    invokeMock.mockResolvedValueOnce({
      modelId: "grok-4.5",
      effortAvailable: true,
      effortValues: ["low", "medium", "high"],
      effortDefault: "high",
      speedAvailable: false,
    });
    await expect(getModelRequestControls("grok-4.5")).resolves.toMatchObject({
      effortAvailable: true,
      effortDefault: "high",
      speedAvailable: false,
    });

    invokeMock.mockResolvedValueOnce({
      modelId: "grok-3",
      effortAvailable: false,
      effortValues: [],
      effortDefault: null,
      speedAvailable: false,
    });
    await expect(getModelRequestControls("grok-3")).resolves.toMatchObject({
      effortAvailable: false,
      effortValues: [],
    });

    for (const hostile of [
      {
        modelId: "grok-4.5",
        effortAvailable: true,
        effortValues: ["low", "medium", "high"],
        effortDefault: "high",
        speedAvailable: true,
      },
      {
        modelId: "grok-4.5",
        effortAvailable: true,
        effortValues: ["low", "medium", "high", "xhigh"],
        effortDefault: "high",
        speedAvailable: false,
      },
      {
        modelId: "grok-3",
        effortAvailable: false,
        effortValues: ["low"],
        effortDefault: null,
        speedAvailable: false,
      },
    ]) {
      invokeMock.mockResolvedValueOnce(hostile);
      await expect(getModelRequestControls("grok-4.5")).rejects.toMatchObject({
        code: "agent_storage_unavailable",
      });
    }
  });

  it("validates pick and clear commands without exposing paths", async () => {
    invokeMock.mockResolvedValueOnce({
      status: "selected",
      draftHandle: "deadbeef".repeat(4),
      displayName: "notes.txt",
      byteCount: 12,
      originKind: "local_text_file",
      memberCount: 1,
      canonicalUrl: null,
    });
    await expect(pickAgentTextSource()).resolves.toEqual({
      status: "selected",
      attachment: {
        draftHandle: "deadbeef".repeat(4),
        displayName: "notes.txt",
        byteCount: 12,
        originKind: "local_text_file",
        memberCount: 1,
        canonicalUrl: null,
      },
    });

    invokeMock.mockResolvedValueOnce({
      status: "selected",
      draftHandle: "deadbeef".repeat(4),
      displayName: "docs",
      byteCount: 120,
      originKind: "local_text_folder",
      memberCount: 3,
      canonicalUrl: null,
    });
    await expect(pickAgentTextFolderSource()).resolves.toEqual({
      status: "selected",
      attachment: {
        draftHandle: "deadbeef".repeat(4),
        displayName: "docs",
        byteCount: 120,
        originKind: "local_text_folder",
        memberCount: 3,
        canonicalUrl: null,
      },
    });

    invokeMock.mockResolvedValueOnce({
      status: "selected",
      draftHandle: "deadbeef".repeat(4),
      displayName: "example.com/readme.txt",
      byteCount: 40,
      originKind: "remote_text_url",
      memberCount: 1,
      canonicalUrl: "https://example.com/readme.txt",
    });
    await expect(attachAgentTextLinkSource("https://example.com/readme.txt")).resolves.toEqual({
      status: "selected",
      attachment: {
        draftHandle: "deadbeef".repeat(4),
        displayName: "example.com/readme.txt",
        byteCount: 40,
        originKind: "remote_text_url",
        memberCount: 1,
        canonicalUrl: "https://example.com/readme.txt",
      },
    });

    invokeMock.mockResolvedValueOnce({
      status: "cancelled",
      draftHandle: null,
      displayName: null,
      byteCount: null,
      originKind: null,
      memberCount: null,
      canonicalUrl: null,
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

    const linkPickBase = {
      status: "selected" as const,
      draftHandle: "deadbeef".repeat(4),
      displayName: "example.com/readme.txt",
      byteCount: 40,
      memberCount: 1,
    };
    const hostileLinkPicks = [
      {
        ...linkPickBase,
        originKind: "remote_text_url",
        canonicalUrl: null,
      },
      {
        ...linkPickBase,
        originKind: "remote_text_url",
        canonicalUrl: "http://example.com/readme.txt",
      },
      {
        ...linkPickBase,
        originKind: "remote_text_url",
        canonicalUrl: "https:///readme.txt",
      },
      {
        ...linkPickBase,
        originKind: "remote_text_url",
        canonicalUrl: "https://example.com/" + "\u{10000}".repeat(513),
      },
      {
        ...linkPickBase,
        originKind: "local_text_file",
        displayName: "notes.txt",
        canonicalUrl: "https://example.com/readme.txt",
      },
      {
        ...linkPickBase,
        originKind: "local_text_folder",
        displayName: "docs",
        memberCount: 2,
        canonicalUrl: "https://example.com/readme.txt",
      },
      {
        ...linkPickBase,
        originKind: "remote_text_url",
        canonicalUrl: "https://example.com/readme.txt",
        path: "C:\\\\secret",
      },
      {
        status: "selected",
        draftHandle: "deadbeef".repeat(4),
        displayName: "example.com/readme.txt",
        byteCount: 40,
        originKind: "remote_text_url",
        memberCount: 1,
      },
    ];
    for (const payload of hostileLinkPicks) {
      invokeMock.mockResolvedValueOnce(payload);
      await expect(
        attachAgentTextLinkSource("https://example.com/readme.txt"),
      ).rejects.toMatchObject({
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

  it("validates artifact list, detail, and create payloads and fails closed on unknown kinds", async () => {
    const validSummary = {
      id: "01900000-0000-7000-8000-000000000030",
      title: "Saved conclusion",
      kind: "conclusion",
      projectId: null,
      createdAtUnixMs: 1_700_000_000_000,
      latestVersionId: "01900000-0000-7000-8000-000000000031",
      latestVersionOrdinal: 1,
    };
    const validVersion = {
      id: "01900000-0000-7000-8000-000000000031",
      artifactId: validSummary.id,
      versionOrdinal: 1,
      content: "exact body",
      contentSha256: "b".repeat(64),
      provenance: {
        sourceSessionId: validSessionDetail.session.id,
        sourceTurnId: "01900000-0000-7000-8000-000000000011",
        providerProfileId: "xai-subscription-oauth",
        modelId: "grok-3",
        promptVersion: "tule-direct-agent-v2",
        projectId: null,
        providerRequestId: "01900000-0000-7000-8000-000000000012",
      },
      createdAtUnixMs: 1_700_000_000_000,
    };
    const validArtifact = {
      id: validSummary.id,
      title: validSummary.title,
      kind: "conclusion",
      projectId: null,
      createdAtUnixMs: validSummary.createdAtUnixMs,
    };

    invokeMock.mockResolvedValueOnce([validSummary]);
    await expect(listArtifacts(validSessionDetail.session.id, null)).resolves.toEqual([
      validSummary,
    ]);
    expect(invokeMock).toHaveBeenCalledWith("list_artifacts", {
      sessionId: validSessionDetail.session.id,
      projectId: null,
    });

    invokeMock.mockResolvedValueOnce({
      artifact: validArtifact,
      versions: [validVersion],
    });
    await expect(getArtifact(validSummary.id)).resolves.toEqual({
      artifact: validArtifact,
      versions: [validVersion],
    });

    invokeMock.mockResolvedValueOnce({
      artifact: validArtifact,
      version: validVersion,
    });
    await expect(
      createArtifactFromTurn({ turnId: validVersion.provenance.sourceTurnId }),
    ).resolves.toEqual({
      artifact: validArtifact,
      version: validVersion,
    });
    expect(invokeMock).toHaveBeenCalledWith("create_artifact_from_turn", {
      turnId: validVersion.provenance.sourceTurnId,
      title: null,
      kind: null,
    });

    invokeMock.mockResolvedValueOnce([
      {
        ...validSummary,
        kind: "not_a_kind",
      },
    ]);
    await expect(listArtifacts(validSessionDetail.session.id, null)).rejects.toMatchObject({
      code: "agent_storage_unavailable",
    });

    invokeMock.mockResolvedValueOnce({
      artifact: { ...validArtifact, kind: "Conclusion" },
      versions: [validVersion],
    });
    await expect(getArtifact(validSummary.id)).rejects.toMatchObject({
      code: "agent_storage_unavailable",
    });

    invokeMock.mockResolvedValueOnce({
      artifact: validArtifact,
      version: { ...validVersion, contentSha256: "not-a-hash" },
    });
    await expect(
      createArtifactFromTurn({ turnId: validVersion.provenance.sourceTurnId }),
    ).rejects.toMatchObject({
      code: "agent_storage_unavailable",
    });
  });
});
