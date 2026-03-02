import { useState, useEffect, useCallback } from "react";
import { Plus, MessageSquare, Wifi, WifiOff, Activity } from "lucide-react";
import { cn } from "../../lib/cn";
import { Button } from "../ui/Button";
import { arcanClient } from "../../api/client";
import { useHealth } from "../../hooks/useHealth";
import type { SessionSummary } from "../../types/api";
import type { SSEStatus } from "../../api/sse";

interface SidebarProps {
  currentSession: string;
  onSelectSession: (id: string) => void;
  connectionStatus: SSEStatus;
}

export function Sidebar({ currentSession, onSelectSession, connectionStatus }: SidebarProps) {
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [collapsed, setCollapsed] = useState(false);
  const health = useHealth();

  const loadSessions = useCallback(async () => {
    try {
      const list = await arcanClient.listSessions();
      setSessions(list);
    } catch {
      // Daemon might not be running yet
    }
  }, []);

  useEffect(() => {
    loadSessions();
    const timer = setInterval(loadSessions, 5000);
    return () => clearInterval(timer);
  }, [loadSessions]);

  const createSession = useCallback(async () => {
    try {
      const session = await arcanClient.createSession();
      await loadSessions();
      onSelectSession(session.session_id);
    } catch {
      // Will show error via health status
    }
  }, [loadSessions, onSelectSession]);

  if (collapsed) {
    return (
      <aside className="w-14 flex flex-col items-center py-4 gap-4 border-r border-border bg-bg-dark/50">
        <button
          onClick={() => setCollapsed(false)}
          className="text-ai-blue hover:text-ai-blue/80 transition-colors"
          title="Expand sidebar"
        >
          <MessageSquare size={20} />
        </button>
        <button
          onClick={createSession}
          className="text-text-secondary hover:text-text-primary transition-colors"
          title="New session"
        >
          <Plus size={18} />
        </button>
        <div className="mt-auto">
          <ConnectionDot status={connectionStatus} />
        </div>
      </aside>
    );
  }

  return (
    <aside className="w-64 flex flex-col border-r border-border bg-bg-dark/50">
      {/* Header */}
      <div className="p-4 border-b border-border">
        <div className="flex items-center gap-2 mb-3">
          <div className="w-8 h-8 rounded-lg bg-gradient-to-br from-ai-blue to-web3-green flex items-center justify-center">
            <span className="text-white font-bold text-sm">A</span>
          </div>
          <div>
            <h1 className="text-sm font-semibold text-text-primary">Arcan</h1>
            <p className="text-xs text-text-muted">Console</p>
          </div>
          <button
            onClick={() => setCollapsed(true)}
            className="ml-auto text-text-muted hover:text-text-secondary transition-colors text-xs"
            title="Collapse sidebar"
          >
            &#x2190;
          </button>
        </div>
        <Button
          variant="secondary"
          size="sm"
          className="w-full justify-center gap-2"
          onClick={createSession}
        >
          <Plus size={14} />
          New Session
        </Button>
      </div>

      {/* Session list */}
      <div className="flex-1 overflow-y-auto py-2">
        {sessions.length === 0 ? (
          <p className="text-text-muted text-xs px-4 py-8 text-center">
            No sessions yet
          </p>
        ) : (
          sessions.map((s) => (
            <button
              key={s.session_id}
              onClick={() => onSelectSession(s.session_id)}
              className={cn(
                "w-full text-left px-4 py-2.5 text-sm transition-colors",
                "hover:bg-surface-hover",
                currentSession === s.session_id
                  ? "bg-surface border-l-2 border-l-ai-blue text-text-primary"
                  : "text-text-secondary",
              )}
            >
              <div className="truncate font-medium">
                {s.session_id}
              </div>
              <div className="text-xs text-text-muted mt-0.5">
                {new Date(s.created_at).toLocaleDateString()}
              </div>
            </button>
          ))
        )}
      </div>

      {/* Footer status */}
      <div className="p-3 border-t border-border space-y-2">
        <div className="flex items-center gap-2 text-xs">
          <ConnectionDot status={connectionStatus} />
          <span className="text-text-muted capitalize">
            {connectionStatus}
          </span>
        </div>
        <div className="flex items-center gap-2 text-xs">
          <Activity size={12} className={cn(
            health.status === "healthy" ? "text-success-green" : "text-alert-red"
          )} />
          <span className="text-text-muted">
            Daemon: {health.status}
          </span>
        </div>
      </div>
    </aside>
  );
}

function ConnectionDot({ status }: { status: SSEStatus }) {
  if (status === "connected") {
    return <Wifi size={12} className="text-success-green" />;
  }
  if (status === "connecting") {
    return <Wifi size={12} className="text-amber-400 animate-pulse" />;
  }
  return <WifiOff size={12} className="text-text-muted" />;
}
