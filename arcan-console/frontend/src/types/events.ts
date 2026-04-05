/**
 * Discriminated union of all EventKind variants emitted by the Arcan runtime.
 * Matches aios-protocol::EventKind.
 */
export type EventKind =
  | { SessionCreated: { owner: string } }
  | { RunStarted: { objective: string; run_id?: string } }
  | { RunFinished: { summary?: string } }
  | { RunErrored: { error: string } }
  | { UserMessage: { content: string } }
  | { AssistantTextDelta: { delta: string } }
  | { AssistantTextFinished: { full_text: string } }
  | { ToolCallRequested: ToolCallInfo }
  | { ToolCallCompleted: ToolResultInfo }
  | { ToolCallFailed: { call_id: string; tool_name: string; error: string } }
  | { ApprovalRequested: ApprovalInfo }
  | { ApprovalResolved: { approval_id: string; approved: boolean; actor: string } }
  | { StateEstimated: { state: import("./api").AgentStateVector; mode: import("./api").OperatingMode } }
  | { ContextCompiled: { block_count: number; total_tokens: number } }
  | { MemoryProposed: { key: string; value: string } }
  | { MemoryCommitted: { key: string } }
  | { MemoryRecalled: { key: string; value: string } }
  | { Observation: { source: string; content: string } }
  | { BranchCreated: { branch_id: string; parent_branch?: string } }
  | { BranchMerged: { source: string; target: string } }
  | Record<string, unknown>;

export interface ToolCallInfo {
  call_id: string;
  tool_name: string;
  input: unknown;
  capabilities?: string[];
}

export interface ToolResultInfo {
  call_id: string;
  tool_name: string;
  output: unknown;
  duration_ms?: number;
}

export interface ApprovalInfo {
  approval_id: string;
  tool_name: string;
  input: unknown;
  risk_level: string;
  reason: string;
}

/** Extract the event kind discriminant name */
export function eventKindName(kind: EventKind): string {
  const keys = Object.keys(kind);
  return keys[0] ?? "Unknown";
}

/** Extract the event kind payload */
export function eventKindPayload(kind: EventKind): unknown {
  const keys = Object.keys(kind);
  if (keys[0]) {
    return (kind as Record<string, unknown>)[keys[0]];
  }
  return kind;
}
