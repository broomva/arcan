/** Session summary returned by GET /sessions */
export interface SessionSummary {
  session_id: string;
  owner: string;
  created_at: string;
}

/** Request body for POST /sessions */
export interface CreateSessionRequest {
  session_id?: string;
  owner?: string;
}

/** Request body for POST /sessions/:id/runs */
export interface RunRequest {
  objective: string;
  branch?: string;
  proposed_tool?: {
    tool_name: string;
    input: unknown;
    requested_capabilities?: string[];
  };
}

/** Response from POST /sessions/:id/runs */
export interface RunResponse {
  session_id: string;
  mode: OperatingMode;
  state: AgentStateVector;
  events_emitted: number;
  last_sequence: number;
}

/** Response from GET /sessions/:id/state */
export interface StateResponse {
  session_id: string;
  branch: string;
  mode: OperatingMode;
  state: AgentStateVector;
  version: number;
}

/** Response from GET /sessions/:id/events */
export interface EventListResponse {
  session_id: string;
  branch: string;
  from_sequence: number;
  events: EventRecord[];
}

/** Request body for POST /sessions/:id/approvals/:aid */
export interface ResolveApprovalRequest {
  approved: boolean;
  actor?: string;
}

/** Health response */
export interface HealthResponse {
  status: string;
}

/** Branch info */
export interface BranchInfo {
  branch_id: string;
  parent_branch: string | null;
  fork_sequence: number;
  head_sequence: number;
  merged_into: string | null;
}

/** Agent operating mode */
export type OperatingMode =
  | "Explore"
  | "Execute"
  | "Verify"
  | "Recover"
  | "AskHuman"
  | "Sleep";

/** Budget state */
export interface BudgetState {
  tokens_remaining: number;
  time_remaining_ms: number;
  cost_remaining_usd: number;
  tool_calls_remaining: number;
  error_budget_remaining: number;
}

/** Risk level */
export type RiskLevel = "Low" | "Medium" | "High" | "Critical";

/** Agent state vector */
export interface AgentStateVector {
  progress: number;
  uncertainty: number;
  risk_level: RiskLevel;
  budget: BudgetState;
  error_streak: number;
  context_pressure: number;
  side_effect_pressure: number;
  human_dependency: number;
}

/** Event record from the journal */
export interface EventRecord {
  event_id: string;
  session_id: string;
  agent_id: string;
  branch_id: string;
  sequence: number;
  timestamp: string;
  kind: Record<string, unknown>;
  [key: string]: unknown;
}
