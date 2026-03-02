import { describe, it, expect, vi, beforeEach } from "vitest";
import { ArcanClient } from "./client";

// Mock fetch globally
const mockFetch = vi.fn();
vi.stubGlobal("fetch", mockFetch);

function jsonResponse(data: unknown, status = 200) {
  return new Response(JSON.stringify(data), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

describe("ArcanClient", () => {
  let client: ArcanClient;

  beforeEach(() => {
    client = new ArcanClient("http://localhost:3000");
    mockFetch.mockReset();
  });

  describe("health", () => {
    it("returns health status", async () => {
      mockFetch.mockResolvedValueOnce(jsonResponse({ status: "ok" }));
      const health = await client.health();
      expect(health.status).toBe("ok");
      expect(mockFetch).toHaveBeenCalledWith(
        "http://localhost:3000/health",
        expect.objectContaining({ headers: expect.objectContaining({ "Content-Type": "application/json" }) }),
      );
    });

    it("throws on HTTP error", async () => {
      mockFetch.mockResolvedValueOnce(new Response("not found", { status: 404 }));
      await expect(client.health()).rejects.toThrow("API error 404");
    });
  });

  describe("listSessions", () => {
    it("returns session list", async () => {
      const sessions = [
        { session_id: "s-1", owner: "arcan", created_at: "2026-01-01T00:00:00Z" },
      ];
      mockFetch.mockResolvedValueOnce(jsonResponse(sessions));
      const result = await client.listSessions();
      expect(result).toEqual(sessions);
    });
  });

  describe("createSession", () => {
    it("sends POST with optional body", async () => {
      mockFetch.mockResolvedValueOnce(
        jsonResponse({ session_id: "new-1", owner: "arcan", created_at: "2026-01-01T00:00:00Z" }),
      );
      const result = await client.createSession({ session_id: "new-1" });
      expect(result.session_id).toBe("new-1");
      expect(mockFetch).toHaveBeenCalledWith(
        "http://localhost:3000/sessions",
        expect.objectContaining({
          method: "POST",
          body: JSON.stringify({ session_id: "new-1" }),
        }),
      );
    });
  });

  describe("startRun", () => {
    it("sends objective to correct endpoint", async () => {
      mockFetch.mockResolvedValueOnce(
        jsonResponse({
          session_id: "s-1",
          mode: "Execute",
          state: {},
          events_emitted: 5,
          last_sequence: 10,
        }),
      );
      await client.startRun("s-1", "read the README");
      expect(mockFetch).toHaveBeenCalledWith(
        "http://localhost:3000/sessions/s-1/runs",
        expect.objectContaining({
          method: "POST",
          body: JSON.stringify({ objective: "read the README" }),
        }),
      );
    });
  });

  describe("getState", () => {
    it("fetches state with default branch", async () => {
      mockFetch.mockResolvedValueOnce(jsonResponse({ session_id: "s-1", branch: "main" }));
      await client.getState("s-1");
      expect(mockFetch).toHaveBeenCalledWith(
        "http://localhost:3000/sessions/s-1/state",
        expect.anything(),
      );
    });

    it("includes branch parameter when specified", async () => {
      mockFetch.mockResolvedValueOnce(jsonResponse({}));
      await client.getState("s-1", "feature");
      expect(mockFetch).toHaveBeenCalledWith(
        "http://localhost:3000/sessions/s-1/state?branch=feature",
        expect.anything(),
      );
    });
  });

  describe("resolveApproval", () => {
    it("sends approval decision", async () => {
      mockFetch.mockResolvedValueOnce(jsonResponse({}));
      await client.resolveApproval("s-1", "apr-1", { approved: true, actor: "console" });
      expect(mockFetch).toHaveBeenCalledWith(
        "http://localhost:3000/sessions/s-1/approvals/apr-1",
        expect.objectContaining({
          method: "POST",
          body: JSON.stringify({ approved: true, actor: "console" }),
        }),
      );
    });
  });

  describe("streamUrl", () => {
    it("builds URL without parameters", () => {
      const url = client.streamUrl("s-1");
      expect(url).toBe("http://localhost:3000/sessions/s-1/events/stream");
    });

    it("builds URL with cursor and branch", () => {
      const url = client.streamUrl("s-1", { cursor: 42, branch: "dev" });
      expect(url).toContain("cursor=42");
      expect(url).toContain("branch=dev");
    });
  });

  describe("listEvents", () => {
    it("sends event list request with parameters", async () => {
      mockFetch.mockResolvedValueOnce(
        jsonResponse({ session_id: "s-1", branch: "main", from_sequence: 1, events: [] }),
      );
      await client.listEvents("s-1", { limit: 50, from_sequence: 10 });
      const calledUrl = mockFetch.mock.calls[0]?.[0] as string;
      expect(calledUrl).toContain("limit=50");
      expect(calledUrl).toContain("from_sequence=10");
    });
  });
});
