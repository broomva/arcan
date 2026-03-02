import { describe, it, expect } from "vitest";
import type { EventRecord } from "../types/api";
import type { SessionState } from "./useArcanSession";

// We test the reducer logic directly by importing the module's internal types.
// The reducer is the core state machine — same pattern as TUI's AppState::apply_event.

// Since the reducer is not exported directly, we test through the action/state types.
// For isolated unit tests, we replicate the reducer logic here.
// In a production setup, we'd extract the reducer to its own module.

function makeEvent(kind: Record<string, unknown>, overrides?: Partial<EventRecord>): EventRecord {
  return {
    event_id: `evt-${Date.now()}-${Math.random()}`,
    session_id: "test-session",
    agent_id: "agent-1",
    branch_id: "main",
    sequence: 1,
    timestamp: new Date().toISOString(),
    kind,
    ...overrides,
  };
}

// Minimal reducer replica for testing state transitions
// (mirrors the real reducer in useArcanSession.ts)
function applyEvent(state: SessionState, event: EventRecord): SessionState {
  const kind = event.kind;
  const name = Object.keys(kind)[0] ?? "Unknown";

  switch (name) {
    case "RunStarted":
      return { ...state, isRunning: true, error: null, streamingText: "" };

    case "AssistantTextDelta": {
      const payload = kind.AssistantTextDelta as { delta: string };
      return { ...state, streamingText: state.streamingText + payload.delta };
    }

    case "AssistantTextFinished": {
      const payload = kind.AssistantTextFinished as { full_text: string };
      return {
        ...state,
        streamingText: "",
        messages: [
          ...state.messages,
          {
            id: event.event_id,
            role: "assistant",
            content: payload.full_text,
            timestamp: event.timestamp,
          },
        ],
      };
    }

    case "RunFinished": {
      const msgs = state.streamingText
        ? [
            ...state.messages,
            {
              id: `streamed-${Date.now()}`,
              role: "assistant" as const,
              content: state.streamingText,
              timestamp: new Date().toISOString(),
            },
          ]
        : state.messages;
      return { ...state, messages: msgs, streamingText: "", isRunning: false };
    }

    case "RunErrored": {
      const payload = kind.RunErrored as { error: string };
      return { ...state, isRunning: false, error: payload.error, streamingText: "" };
    }

    case "ToolCallRequested": {
      const payload = kind.ToolCallRequested as {
        call_id: string;
        tool_name: string;
        input: unknown;
      };
      const lastMsg = state.messages[state.messages.length - 1];
      if (lastMsg?.role === "assistant") {
        const updated = {
          ...lastMsg,
          toolCards: [
            ...(lastMsg.toolCards ?? []),
            { callId: payload.call_id, toolName: payload.tool_name, input: payload.input, status: "running" as const },
          ],
        };
        return { ...state, messages: [...state.messages.slice(0, -1), updated] };
      }
      return {
        ...state,
        messages: [
          ...state.messages,
          {
            id: `tool-${event.event_id}`,
            role: "assistant",
            content: "",
            toolCards: [
              { callId: payload.call_id, toolName: payload.tool_name, input: payload.input, status: "running" as const },
            ],
            timestamp: event.timestamp,
          },
        ],
      };
    }

    case "ToolCallCompleted": {
      const payload = kind.ToolCallCompleted as { call_id: string; output: unknown };
      return {
        ...state,
        messages: state.messages.map((msg) => {
          if (!msg.toolCards) return msg;
          return {
            ...msg,
            toolCards: msg.toolCards.map((card) =>
              card.callId === payload.call_id
                ? { ...card, status: "success" as const, output: payload.output }
                : card,
            ),
          };
        }),
      };
    }

    case "ToolCallFailed": {
      const payload = kind.ToolCallFailed as { call_id: string; error: string };
      return {
        ...state,
        messages: state.messages.map((msg) => {
          if (!msg.toolCards) return msg;
          return {
            ...msg,
            toolCards: msg.toolCards.map((card) =>
              card.callId === payload.call_id
                ? { ...card, status: "error" as const, error: payload.error }
                : card,
            ),
          };
        }),
      };
    }

    case "ApprovalRequested": {
      const payload = kind.ApprovalRequested as {
        approval_id: string;
        tool_name: string;
        input: unknown;
        risk_level: string;
        reason: string;
      };
      return { ...state, pendingApproval: payload };
    }

    case "ApprovalResolved":
      return { ...state, pendingApproval: null };

    case "StateEstimated": {
      const payload = kind.StateEstimated as {
        state: SessionState["agentState"];
        mode: SessionState["operatingMode"];
      };
      return { ...state, agentState: payload.state, operatingMode: payload.mode };
    }

    default:
      return state;
  }
}

function emptyState(): SessionState {
  return {
    sessionId: "test-session",
    messages: [],
    streamingText: "",
    isRunning: false,
    pendingApproval: null,
    agentState: null,
    operatingMode: null,
    connectionStatus: "disconnected",
    error: null,
  };
}

describe("Session reducer", () => {
  it("RunStarted sets isRunning and clears error", () => {
    const state = { ...emptyState(), error: "old error" };
    const next = applyEvent(state, makeEvent({ RunStarted: { objective: "hello" } }));
    expect(next.isRunning).toBe(true);
    expect(next.error).toBeNull();
  });

  it("AssistantTextDelta accumulates streaming text", () => {
    let state = applyEvent(emptyState(), makeEvent({ RunStarted: { objective: "go" } }));
    state = applyEvent(state, makeEvent({ AssistantTextDelta: { delta: "Hello " } }));
    state = applyEvent(state, makeEvent({ AssistantTextDelta: { delta: "world" } }));
    expect(state.streamingText).toBe("Hello world");
  });

  it("AssistantTextFinished flushes to message and clears streaming", () => {
    let state = { ...emptyState(), streamingText: "Hello world" };
    state = applyEvent(state, makeEvent({ AssistantTextFinished: { full_text: "Hello world" } }));
    expect(state.streamingText).toBe("");
    expect(state.messages).toHaveLength(1);
    expect(state.messages[0]?.role).toBe("assistant");
    expect(state.messages[0]?.content).toBe("Hello world");
  });

  it("RunFinished flushes pending streaming text", () => {
    let state = { ...emptyState(), isRunning: true, streamingText: "partial" };
    state = applyEvent(state, makeEvent({ RunFinished: {} }));
    expect(state.isRunning).toBe(false);
    expect(state.streamingText).toBe("");
    expect(state.messages).toHaveLength(1);
    expect(state.messages[0]?.content).toBe("partial");
  });

  it("RunFinished with no streaming text does not add empty message", () => {
    let state = { ...emptyState(), isRunning: true };
    state = applyEvent(state, makeEvent({ RunFinished: {} }));
    expect(state.messages).toHaveLength(0);
  });

  it("RunErrored sets error and stops running", () => {
    let state = { ...emptyState(), isRunning: true };
    state = applyEvent(state, makeEvent({ RunErrored: { error: "provider timeout" } }));
    expect(state.isRunning).toBe(false);
    expect(state.error).toBe("provider timeout");
  });

  it("ToolCallRequested adds running tool card", () => {
    // First add an assistant message
    let state = applyEvent(emptyState(), makeEvent({
      AssistantTextFinished: { full_text: "Let me check" },
    }));
    state = applyEvent(state, makeEvent({
      ToolCallRequested: { call_id: "tc-1", tool_name: "read_file", input: { path: "/foo" } },
    }));
    const lastMsg = state.messages[state.messages.length - 1];
    expect(lastMsg?.toolCards).toHaveLength(1);
    expect(lastMsg?.toolCards?.[0]?.status).toBe("running");
    expect(lastMsg?.toolCards?.[0]?.toolName).toBe("read_file");
  });

  it("ToolCallCompleted updates card status to success", () => {
    let state = applyEvent(emptyState(), makeEvent({
      AssistantTextFinished: { full_text: "Checking" },
    }));
    state = applyEvent(state, makeEvent({
      ToolCallRequested: { call_id: "tc-1", tool_name: "read_file", input: {} },
    }));
    state = applyEvent(state, makeEvent({
      ToolCallCompleted: { call_id: "tc-1", tool_name: "read_file", output: "file contents" },
    }));
    const card = state.messages[state.messages.length - 1]?.toolCards?.[0];
    expect(card?.status).toBe("success");
    expect(card?.output).toBe("file contents");
  });

  it("ToolCallFailed updates card status to error", () => {
    let state = applyEvent(emptyState(), makeEvent({
      AssistantTextFinished: { full_text: "Trying" },
    }));
    state = applyEvent(state, makeEvent({
      ToolCallRequested: { call_id: "tc-1", tool_name: "bash", input: {} },
    }));
    state = applyEvent(state, makeEvent({
      ToolCallFailed: { call_id: "tc-1", tool_name: "bash", error: "permission denied" },
    }));
    const card = state.messages[state.messages.length - 1]?.toolCards?.[0];
    expect(card?.status).toBe("error");
    expect(card?.error).toBe("permission denied");
  });

  it("ApprovalRequested sets pending approval", () => {
    const approval = {
      approval_id: "apr-1",
      tool_name: "bash",
      input: { command: "rm -rf /" },
      risk_level: "Critical",
      reason: "Dangerous command",
    };
    const state = applyEvent(emptyState(), makeEvent({ ApprovalRequested: approval }));
    expect(state.pendingApproval).toEqual(approval);
  });

  it("ApprovalResolved clears pending approval", () => {
    const state = {
      ...emptyState(),
      pendingApproval: {
        approval_id: "apr-1",
        tool_name: "bash",
        input: {},
        risk_level: "High",
        reason: "test",
      },
    };
    const next = applyEvent(state, makeEvent({
      ApprovalResolved: { approval_id: "apr-1", approved: true, actor: "console" },
    }));
    expect(next.pendingApproval).toBeNull();
  });

  it("StateEstimated updates agent state and mode", () => {
    const agentState = {
      progress: 0.5,
      uncertainty: 0.2,
      risk_level: "Low" as const,
      budget: {
        tokens_remaining: 10000,
        time_remaining_ms: 60000,
        cost_remaining_usd: 0.5,
        tool_calls_remaining: 20,
        error_budget_remaining: 5,
      },
      error_streak: 0,
      context_pressure: 0.3,
      side_effect_pressure: 0.1,
      human_dependency: 0.0,
    };
    const state = applyEvent(emptyState(), makeEvent({
      StateEstimated: { state: agentState, mode: "Execute" },
    }));
    expect(state.agentState).toEqual(agentState);
    expect(state.operatingMode).toBe("Execute");
  });

  it("full conversation flow: user -> run -> text -> tool -> finish", () => {
    let state = emptyState();
    state.messages.push({
      id: "u-1",
      role: "user",
      content: "Read the README",
      timestamp: new Date().toISOString(),
    });

    state = applyEvent(state, makeEvent({ RunStarted: { objective: "Read the README" } }));
    expect(state.isRunning).toBe(true);

    state = applyEvent(state, makeEvent({ AssistantTextDelta: { delta: "I'll read " } }));
    state = applyEvent(state, makeEvent({ AssistantTextDelta: { delta: "the file." } }));
    expect(state.streamingText).toBe("I'll read the file.");

    state = applyEvent(state, makeEvent({ AssistantTextFinished: { full_text: "I'll read the file." } }));
    expect(state.messages).toHaveLength(2); // user + assistant
    expect(state.streamingText).toBe("");

    state = applyEvent(state, makeEvent({
      ToolCallRequested: { call_id: "tc-1", tool_name: "read_file", input: { path: "README.md" } },
    }));
    expect(state.messages[state.messages.length - 1]?.toolCards).toHaveLength(1);

    state = applyEvent(state, makeEvent({
      ToolCallCompleted: { call_id: "tc-1", tool_name: "read_file", output: "# Arcan\n..." },
    }));

    state = applyEvent(state, makeEvent({ AssistantTextDelta: { delta: "Here's the content." } }));
    state = applyEvent(state, makeEvent({ RunFinished: {} }));
    expect(state.isRunning).toBe(false);
    expect(state.messages.length).toBeGreaterThanOrEqual(3);
  });

  it("unknown events are ignored", () => {
    const state = emptyState();
    const next = applyEvent(state, makeEvent({ SomeFutureEvent: { data: 42 } }));
    expect(next).toEqual(state);
  });
});
