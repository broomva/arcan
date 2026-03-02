import type {
  SessionSummary,
  CreateSessionRequest,
  RunResponse,
  StateResponse,
  EventListResponse,
  ResolveApprovalRequest,
  HealthResponse,
  BranchInfo,
} from "../types/api";

/**
 * Typed REST client for the Arcan daemon API.
 * Base URL defaults to the current origin (works when served from the daemon).
 */
export class ArcanClient {
  private baseUrl: string;

  constructor(baseUrl?: string) {
    this.baseUrl = baseUrl ?? "";
  }

  private async request<T>(path: string, init?: RequestInit): Promise<T> {
    const resp = await fetch(`${this.baseUrl}${path}`, {
      ...init,
      headers: {
        "Content-Type": "application/json",
        ...init?.headers,
      },
    });
    if (!resp.ok) {
      const body = await resp.text();
      throw new Error(`API error ${resp.status}: ${body}`);
    }
    return resp.json() as Promise<T>;
  }

  // ── Health ──────────────────────────────────────────────────────────────

  async health(): Promise<HealthResponse> {
    return this.request("/health");
  }

  // ── Sessions ────────────────────────────────────────────────────────────

  async listSessions(): Promise<SessionSummary[]> {
    return this.request("/sessions");
  }

  async createSession(req?: CreateSessionRequest): Promise<SessionSummary> {
    return this.request("/sessions", {
      method: "POST",
      body: JSON.stringify(req ?? {}),
    });
  }

  // ── Runs ────────────────────────────────────────────────────────────────

  async startRun(sessionId: string, objective: string, branch?: string): Promise<RunResponse> {
    return this.request(`/sessions/${sessionId}/runs`, {
      method: "POST",
      body: JSON.stringify({ objective, branch }),
    });
  }

  // ── State ───────────────────────────────────────────────────────────────

  async getState(sessionId: string, branch?: string): Promise<StateResponse> {
    const params = branch ? `?branch=${encodeURIComponent(branch)}` : "";
    return this.request(`/sessions/${sessionId}/state${params}`);
  }

  // ── Events ──────────────────────────────────────────────────────────────

  async listEvents(
    sessionId: string,
    opts?: { branch?: string; from_sequence?: number; limit?: number },
  ): Promise<EventListResponse> {
    const params = new URLSearchParams();
    if (opts?.branch) params.set("branch", opts.branch);
    if (opts?.from_sequence) params.set("from_sequence", String(opts.from_sequence));
    if (opts?.limit) params.set("limit", String(opts.limit));
    const qs = params.toString();
    return this.request(`/sessions/${sessionId}/events${qs ? `?${qs}` : ""}`);
  }

  // ── Branches ────────────────────────────────────────────────────────────

  async listBranches(sessionId: string): Promise<{ session_id: string; branches: BranchInfo[] }> {
    return this.request(`/sessions/${sessionId}/branches`);
  }

  async createBranch(
    sessionId: string,
    branch: string,
    fromBranch?: string,
  ): Promise<BranchInfo> {
    return this.request(`/sessions/${sessionId}/branches`, {
      method: "POST",
      body: JSON.stringify({ branch, from_branch: fromBranch }),
    });
  }

  // ── Approvals ───────────────────────────────────────────────────────────

  async resolveApproval(
    sessionId: string,
    approvalId: string,
    req: ResolveApprovalRequest,
  ): Promise<void> {
    await this.request(`/sessions/${sessionId}/approvals/${approvalId}`, {
      method: "POST",
      body: JSON.stringify(req),
    });
  }

  // ── SSE stream URL ──────────────────────────────────────────────────────

  streamUrl(sessionId: string, opts?: { branch?: string; cursor?: number; format?: string }): string {
    const params = new URLSearchParams();
    if (opts?.branch) params.set("branch", opts.branch);
    if (opts?.cursor) params.set("cursor", String(opts.cursor));
    if (opts?.format) params.set("format", opts.format);
    const qs = params.toString();
    return `${this.baseUrl}/sessions/${sessionId}/events/stream${qs ? `?${qs}` : ""}`;
  }
}

/** Singleton client instance */
export const arcanClient = new ArcanClient();
