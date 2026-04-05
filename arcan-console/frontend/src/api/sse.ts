import type { EventRecord } from "../types/api";

export type SSEStatus = "connecting" | "connected" | "disconnected" | "error";

export interface EventStreamOptions {
  /** URL to connect to */
  url: string;
  /** Callback for each parsed event */
  onEvent: (event: EventRecord) => void;
  /** Callback for connection status changes */
  onStatusChange?: (status: SSEStatus) => void;
  /** Callback on error */
  onError?: (error: Error) => void;
  /** Auto-reconnect delay in ms (default 2000) */
  reconnectDelay?: number;
}

/**
 * Manages an EventSource connection to the Arcan SSE stream.
 * Supports cursor-based resume on reconnect.
 */
export class EventStreamManager {
  private source: EventSource | null = null;
  private cursor = 0;
  private baseUrl: string;
  private options: EventStreamOptions;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private intentionallyClosed = false;

  constructor(options: EventStreamOptions) {
    this.options = options;
    this.baseUrl = options.url;
  }

  /** Start (or restart) the SSE connection. */
  connect(): void {
    this.intentionallyClosed = false;
    this.cleanup();

    const url = this.cursor > 0
      ? this.appendCursor(this.baseUrl, this.cursor)
      : this.baseUrl;

    this.options.onStatusChange?.("connecting");

    const source = new EventSource(url);
    this.source = source;

    source.onopen = () => {
      this.options.onStatusChange?.("connected");
    };

    source.onmessage = (msg) => {
      try {
        const event = JSON.parse(msg.data) as EventRecord;
        // Update cursor from the SSE event ID.
        const eventId = parseInt(msg.lastEventId, 10);
        if (!isNaN(eventId) && eventId > this.cursor) {
          this.cursor = eventId;
        }
        this.options.onEvent(event);
      } catch {
        // Skip non-JSON messages (keepalive, etc.)
      }
    };

    source.onerror = () => {
      this.options.onStatusChange?.("disconnected");
      source.close();
      this.source = null;

      if (!this.intentionallyClosed) {
        this.scheduleReconnect();
      }
    };
  }

  /** Disconnect and stop reconnecting. */
  disconnect(): void {
    this.intentionallyClosed = true;
    this.cleanup();
    this.options.onStatusChange?.("disconnected");
  }

  /** Reset cursor (for session switching). */
  resetCursor(): void {
    this.cursor = 0;
  }

  private cleanup(): void {
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    if (this.source) {
      this.source.close();
      this.source = null;
    }
  }

  private scheduleReconnect(): void {
    const delay = this.options.reconnectDelay ?? 2000;
    this.reconnectTimer = setTimeout(() => {
      this.connect();
    }, delay);
  }

  private appendCursor(url: string, cursor: number): string {
    const separator = url.includes("?") ? "&" : "?";
    return `${url}${separator}cursor=${cursor}`;
  }
}
