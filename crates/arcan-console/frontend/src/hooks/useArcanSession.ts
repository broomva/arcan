import { useReducer, useEffect, useCallback, useRef } from "react";
import type { EventRecord, AgentStateVector, OperatingMode } from "../types/api";
import type { EventKind, ToolCallInfo, ToolResultInfo, ApprovalInfo } from "../types/events";
import { eventKindName } from "../types/events";
import { arcanClient } from "../api/client";
import { EventStreamManager, type SSEStatus } from "../api/sse";

// ─── State ──────────────────────────────────────────────────────────────────

export interface ToolCard {
  callId: string;
  toolName: string;
  input: unknown;
  status: "running" | "success" | "error";
  output?: unknown;
  error?: string;
  durationMs?: number;
}

export interface ChatMessage {
  id: string;
  role: "user" | "assistant" | "system" | "tool";
  content: string;
  toolCards?: ToolCard[];
  timestamp: string;
}

export interface SessionState {
  sessionId: string;
  messages: ChatMessage[];
  streamingText: string;
  isRunning: boolean;
  pendingApproval: ApprovalInfo | null;
  agentState: AgentStateVector | null;
  operatingMode: OperatingMode | null;
  connectionStatus: SSEStatus;
  error: string | null;
}

const initialState: SessionState = {
  sessionId: "",
  messages: [],
  streamingText: "",
  isRunning: false,
  pendingApproval: null,
  agentState: null,
  operatingMode: null,
  connectionStatus: "disconnected",
  error: null,
};

// ─── Actions ────────────────────────────────────────────────────────────────

type Action =
  | { type: "SET_SESSION"; sessionId: string }
  | { type: "RESET" }
  | { type: "SET_CONNECTION"; status: SSEStatus }
  | { type: "SET_ERROR"; error: string | null }
  | { type: "ADD_USER_MESSAGE"; content: string }
  | { type: "APPLY_EVENT"; event: EventRecord };

// ─── Reducer ────────────────────────────────────────────────────────────────

function reducer(state: SessionState, action: Action): SessionState {
  switch (action.type) {
    case "SET_SESSION":
      return { ...initialState, sessionId: action.sessionId };
    case "RESET":
      return { ...initialState, sessionId: state.sessionId };
    case "SET_CONNECTION":
      return { ...state, connectionStatus: action.status };
    case "SET_ERROR":
      return { ...state, error: action.error };
    case "ADD_USER_MESSAGE":
      return {
        ...state,
        messages: [
          ...state.messages,
          {
            id: `user-${Date.now()}`,
            role: "user",
            content: action.content,
            timestamp: new Date().toISOString(),
          },
        ],
        isRunning: true,
        error: null,
      };
    case "APPLY_EVENT":
      return applyEvent(state, action.event);
    default:
      return state;
  }
}

function applyEvent(state: SessionState, event: EventRecord): SessionState {
  const kind = event.kind as EventKind;
  const name = eventKindName(kind);

  switch (name) {
    case "RunStarted":
      return { ...state, isRunning: true, error: null, streamingText: "" };

    case "UserMessage": {
      const payload = (kind as { UserMessage: { content: string } }).UserMessage;
      // Check if already added optimistically
      const lastMsg = state.messages[state.messages.length - 1];
      if (lastMsg?.role === "user" && lastMsg.content === payload.content) {
        return state;
      }
      return {
        ...state,
        messages: [
          ...state.messages,
          {
            id: event.event_id,
            role: "user",
            content: payload.content,
            timestamp: event.timestamp,
          },
        ],
      };
    }

    case "AssistantTextDelta": {
      const payload = (kind as { AssistantTextDelta: { delta: string } }).AssistantTextDelta;
      return { ...state, streamingText: state.streamingText + payload.delta };
    }

    case "AssistantTextFinished": {
      const payload = (kind as { AssistantTextFinished: { full_text: string } }).AssistantTextFinished;
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

    case "ToolCallRequested": {
      const payload = (kind as { ToolCallRequested: ToolCallInfo }).ToolCallRequested;
      // Flush any pending streaming text
      const msgs = flushStreamingText(state);
      const toolCard: ToolCard = {
        callId: payload.call_id,
        toolName: payload.tool_name,
        input: payload.input,
        status: "running",
      };
      // Add tool card to the last assistant message or create a new one
      const lastMsg = msgs[msgs.length - 1];
      if (lastMsg?.role === "assistant") {
        const updated = {
          ...lastMsg,
          toolCards: [...(lastMsg.toolCards ?? []), toolCard],
        };
        return {
          ...state,
          streamingText: "",
          messages: [...msgs.slice(0, -1), updated],
        };
      }
      return {
        ...state,
        streamingText: "",
        messages: [
          ...msgs,
          {
            id: `tool-group-${event.event_id}`,
            role: "assistant",
            content: "",
            toolCards: [toolCard],
            timestamp: event.timestamp,
          },
        ],
      };
    }

    case "ToolCallCompleted": {
      const payload = (kind as { ToolCallCompleted: ToolResultInfo }).ToolCallCompleted;
      return updateToolCard(state, payload.call_id, {
        status: "success",
        output: payload.output,
        durationMs: payload.duration_ms,
      });
    }

    case "ToolCallFailed": {
      const payload = (kind as { ToolCallFailed: { call_id: string; error: string } }).ToolCallFailed;
      return updateToolCard(state, payload.call_id, {
        status: "error",
        error: payload.error,
      });
    }

    case "ApprovalRequested": {
      const payload = (kind as { ApprovalRequested: ApprovalInfo }).ApprovalRequested;
      return { ...state, pendingApproval: payload };
    }

    case "ApprovalResolved":
      return { ...state, pendingApproval: null };

    case "RunFinished": {
      const msgs = flushStreamingText(state);
      return {
        ...state,
        messages: msgs,
        streamingText: "",
        isRunning: false,
      };
    }

    case "RunErrored": {
      const payload = (kind as { RunErrored: { error: string } }).RunErrored;
      const msgs = flushStreamingText(state);
      return {
        ...state,
        messages: msgs,
        streamingText: "",
        isRunning: false,
        error: payload.error,
      };
    }

    case "StateEstimated": {
      const payload = (kind as {
        StateEstimated: {
          state: AgentStateVector;
          mode: OperatingMode;
        };
      }).StateEstimated;
      return {
        ...state,
        agentState: payload.state,
        operatingMode: payload.mode,
      };
    }

    default:
      return state;
  }
}

function flushStreamingText(state: SessionState): ChatMessage[] {
  if (!state.streamingText) return state.messages;
  return [
    ...state.messages,
    {
      id: `streamed-${Date.now()}`,
      role: "assistant",
      content: state.streamingText,
      timestamp: new Date().toISOString(),
    },
  ];
}

function updateToolCard(
  state: SessionState,
  callId: string,
  update: Partial<ToolCard>,
): SessionState {
  const messages = state.messages.map((msg) => {
    if (!msg.toolCards) return msg;
    const cards = msg.toolCards.map((card) =>
      card.callId === callId ? { ...card, ...update } : card,
    );
    return { ...msg, toolCards: cards };
  });
  return { ...state, messages };
}

// ─── Hook ───────────────────────────────────────────────────────────────────

export function useArcanSession(sessionId: string) {
  const [state, dispatch] = useReducer(reducer, {
    ...initialState,
    sessionId,
  });
  const streamRef = useRef<EventStreamManager | null>(null);

  // Connect to SSE stream when session changes
  useEffect(() => {
    if (!sessionId) return;

    dispatch({ type: "SET_SESSION", sessionId });

    const manager = new EventStreamManager({
      url: arcanClient.streamUrl(sessionId),
      onEvent: (event) => dispatch({ type: "APPLY_EVENT", event }),
      onStatusChange: (status) => dispatch({ type: "SET_CONNECTION", status }),
      onError: (err) => dispatch({ type: "SET_ERROR", error: err.message }),
    });

    streamRef.current = manager;
    manager.connect();

    return () => {
      manager.disconnect();
      streamRef.current = null;
    };
  }, [sessionId]);

  // Send a message
  const sendMessage = useCallback(
    async (content: string) => {
      if (!sessionId || !content.trim()) return;
      dispatch({ type: "ADD_USER_MESSAGE", content });
      try {
        await arcanClient.startRun(sessionId, content);
      } catch (err) {
        dispatch({
          type: "SET_ERROR",
          error: err instanceof Error ? err.message : "Failed to send message",
        });
      }
    },
    [sessionId],
  );

  // Resolve an approval
  const resolveApproval = useCallback(
    async (approvalId: string, approved: boolean) => {
      if (!sessionId) return;
      try {
        await arcanClient.resolveApproval(sessionId, approvalId, {
          approved,
          actor: "console",
        });
      } catch (err) {
        dispatch({
          type: "SET_ERROR",
          error: err instanceof Error ? err.message : "Failed to resolve approval",
        });
      }
    },
    [sessionId],
  );

  return {
    ...state,
    sendMessage,
    resolveApproval,
  };
}
