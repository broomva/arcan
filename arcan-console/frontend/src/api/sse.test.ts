import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { EventStreamManager, type SSEStatus } from "./sse";

// Mock EventSource
class MockEventSource {
  static instances: MockEventSource[] = [];
  url: string;
  onopen: (() => void) | null = null;
  onmessage: ((event: MessageEvent) => void) | null = null;
  onerror: (() => void) | null = null;
  readyState = 0;
  closed = false;

  constructor(url: string) {
    this.url = url;
    MockEventSource.instances.push(this);
  }

  close() {
    this.closed = true;
    this.readyState = 2;
  }
}

vi.stubGlobal("EventSource", MockEventSource);

describe("EventStreamManager", () => {
  beforeEach(() => {
    MockEventSource.instances = [];
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("connects to the provided URL", () => {
    const onEvent = vi.fn();
    const manager = new EventStreamManager({
      url: "http://localhost:3000/sessions/s-1/events/stream",
      onEvent,
    });
    manager.connect();

    expect(MockEventSource.instances).toHaveLength(1);
    expect(MockEventSource.instances[0]?.url).toBe(
      "http://localhost:3000/sessions/s-1/events/stream",
    );

    manager.disconnect();
  });

  it("fires onStatusChange when connected", () => {
    const statuses: SSEStatus[] = [];
    const manager = new EventStreamManager({
      url: "http://localhost:3000/test",
      onEvent: vi.fn(),
      onStatusChange: (s) => statuses.push(s),
    });

    manager.connect();
    expect(statuses).toContain("connecting");

    // Simulate onopen
    MockEventSource.instances[0]?.onopen?.();
    expect(statuses).toContain("connected");

    manager.disconnect();
    expect(statuses).toContain("disconnected");
  });

  it("parses incoming events and updates cursor", () => {
    const events: unknown[] = [];
    const manager = new EventStreamManager({
      url: "http://localhost:3000/test",
      onEvent: (e) => events.push(e),
    });

    manager.connect();
    const source = MockEventSource.instances[0]!;

    // Simulate a message
    source.onmessage?.({
      data: JSON.stringify({ event_id: "e-1", sequence: 5, kind: { RunStarted: {} } }),
      lastEventId: "5",
    } as MessageEvent);

    expect(events).toHaveLength(1);
    expect((events[0] as { sequence: number }).sequence).toBe(5);

    manager.disconnect();
  });

  it("reconnects with cursor after disconnect", () => {
    const manager = new EventStreamManager({
      url: "http://localhost:3000/test",
      onEvent: vi.fn(),
      reconnectDelay: 1000,
    });

    manager.connect();
    const source = MockEventSource.instances[0]!;

    // Simulate receiving an event to set cursor
    source.onmessage?.({
      data: JSON.stringify({ event_id: "e-1", sequence: 10 }),
      lastEventId: "10",
    } as MessageEvent);

    // Simulate disconnect
    source.onerror?.();

    expect(source.closed).toBe(true);

    // Advance timer to trigger reconnect
    vi.advanceTimersByTime(1000);

    expect(MockEventSource.instances).toHaveLength(2);
    expect(MockEventSource.instances[1]?.url).toContain("cursor=10");

    manager.disconnect();
  });

  it("does not reconnect after intentional disconnect", () => {
    const manager = new EventStreamManager({
      url: "http://localhost:3000/test",
      onEvent: vi.fn(),
      reconnectDelay: 500,
    });

    manager.connect();
    manager.disconnect();

    vi.advanceTimersByTime(1000);
    // Should only have the initial connection
    expect(MockEventSource.instances).toHaveLength(1);
  });

  it("skips non-JSON messages gracefully", () => {
    const events: unknown[] = [];
    const manager = new EventStreamManager({
      url: "http://localhost:3000/test",
      onEvent: (e) => events.push(e),
    });

    manager.connect();
    const source = MockEventSource.instances[0]!;

    // Send a keepalive / non-JSON message
    source.onmessage?.({
      data: ":keepalive",
      lastEventId: "",
    } as MessageEvent);

    expect(events).toHaveLength(0);

    manager.disconnect();
  });

  it("resetCursor resets the resume position", () => {
    const manager = new EventStreamManager({
      url: "http://localhost:3000/test",
      onEvent: vi.fn(),
      reconnectDelay: 500,
    });

    manager.connect();
    const source = MockEventSource.instances[0]!;

    source.onmessage?.({
      data: JSON.stringify({ event_id: "e-1", sequence: 50 }),
      lastEventId: "50",
    } as MessageEvent);

    manager.resetCursor();

    // Force a reconnect
    source.onerror?.();
    vi.advanceTimersByTime(500);

    // The reconnection URL should NOT have cursor=50
    const reconnectUrl = MockEventSource.instances[1]?.url ?? "";
    expect(reconnectUrl).not.toContain("cursor=50");

    manager.disconnect();
  });
});
