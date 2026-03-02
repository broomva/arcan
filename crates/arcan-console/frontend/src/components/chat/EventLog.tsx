import { useState, useEffect, useCallback } from "react";
import { List, RefreshCw, ChevronDown, ChevronRight } from "lucide-react";
import { cn } from "../../lib/cn";
import { Badge } from "../ui/Badge";
import { arcanClient } from "../../api/client";
import { eventKindName } from "../../types/events";
import type { EventRecord } from "../../types/api";
import type { EventKind } from "../../types/events";

interface EventLogProps {
  sessionId: string;
  onClose: () => void;
}

export function EventLog({ sessionId, onClose }: EventLogProps) {
  const [events, setEvents] = useState<EventRecord[]>([]);
  const [loading, setLoading] = useState(false);

  const loadEvents = useCallback(async () => {
    if (!sessionId) return;
    setLoading(true);
    try {
      const resp = await arcanClient.listEvents(sessionId, { limit: 200 });
      setEvents(resp.events);
    } catch {
      // Ignore
    }
    setLoading(false);
  }, [sessionId]);

  useEffect(() => {
    loadEvents();
  }, [loadEvents]);

  return (
    <div className="flex flex-col h-full border-l border-border bg-bg-dark/50 w-96">
      <div className="flex items-center justify-between px-4 py-3 border-b border-border">
        <div className="flex items-center gap-2">
          <List size={16} className="text-text-secondary" />
          <h3 className="text-sm font-semibold text-text-primary">Event Log</h3>
          <Badge variant="default">{events.length}</Badge>
        </div>
        <div className="flex items-center gap-2">
          <button
            onClick={loadEvents}
            className={cn(
              "text-text-muted hover:text-text-primary transition-colors p-1",
              loading && "animate-spin",
            )}
            title="Refresh"
          >
            <RefreshCw size={14} />
          </button>
          <button
            onClick={onClose}
            className="text-text-muted hover:text-text-primary text-xs"
          >
            Close
          </button>
        </div>
      </div>

      <div className="flex-1 overflow-y-auto">
        {events.map((event) => (
          <EventRow key={event.event_id} event={event} />
        ))}
        {events.length === 0 && !loading && (
          <p className="text-xs text-text-muted text-center py-8">No events recorded.</p>
        )}
      </div>
    </div>
  );
}

function EventRow({ event }: { event: EventRecord }) {
  const [expanded, setExpanded] = useState(false);
  const name = eventKindName(event.kind as EventKind);

  const kindColor = getKindColor(name);

  return (
    <div className="border-b border-border/50">
      <button
        onClick={() => setExpanded(!expanded)}
        className="w-full flex items-center gap-2 px-4 py-2 text-left hover:bg-surface/30 transition-colors"
      >
        {expanded ? (
          <ChevronDown size={12} className="text-text-muted shrink-0" />
        ) : (
          <ChevronRight size={12} className="text-text-muted shrink-0" />
        )}
        <span className="text-[10px] font-mono text-text-muted w-8 shrink-0">
          #{event.sequence}
        </span>
        <span className={cn("text-xs font-medium", kindColor)}>{name}</span>
        <span className="ml-auto text-[10px] text-text-muted font-mono">
          {new Date(event.timestamp).toLocaleTimeString()}
        </span>
      </button>
      {expanded && (
        <div className="px-4 pb-2">
          <pre className="text-[10px] font-mono bg-bg-dark/50 rounded p-2 overflow-x-auto max-h-48 overflow-y-auto text-text-secondary">
            {JSON.stringify(event.kind, null, 2)}
          </pre>
        </div>
      )}
    </div>
  );
}

function getKindColor(name: string): string {
  if (name.includes("Error") || name.includes("Failed")) return "text-alert-red";
  if (name.includes("Tool")) return "text-ai-blue";
  if (name.includes("User")) return "text-ai-blue";
  if (name.includes("Assistant") || name.includes("Text")) return "text-web3-green";
  if (name.includes("Approval")) return "text-amber-400";
  if (name.includes("State")) return "text-purple-400";
  if (name.includes("Run")) return "text-text-primary";
  return "text-text-secondary";
}
