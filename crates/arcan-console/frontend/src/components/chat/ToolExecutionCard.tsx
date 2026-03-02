import { useState } from "react";
import {
  ChevronDown,
  ChevronRight,
  CheckCircle,
  XCircle,
  Loader2,
  Terminal,
} from "lucide-react";
import { cn } from "../../lib/cn";
import { Badge } from "../ui/Badge";
import type { ToolCard } from "../../hooks/useArcanSession";

interface ToolExecutionCardProps {
  card: ToolCard;
}

export function ToolExecutionCard({ card }: ToolExecutionCardProps) {
  const [expanded, setExpanded] = useState(false);

  const statusIcon = {
    running: <Loader2 size={14} className="animate-spin text-ai-blue" />,
    success: <CheckCircle size={14} className="text-success-green" />,
    error: <XCircle size={14} className="text-alert-red" />,
  }[card.status];

  const statusBadge = {
    running: "info" as const,
    success: "success" as const,
    error: "error" as const,
  }[card.status];

  return (
    <div
      className={cn(
        "relative my-2 rounded-lg border transition-all duration-300",
        card.status === "running"
          ? "border-ai-blue/30 bg-ai-blue/5"
          : card.status === "error"
            ? "border-alert-red/30 bg-alert-red/5"
            : "border-border bg-surface/50",
      )}
    >
      <button
        onClick={() => setExpanded(!expanded)}
        className="w-full flex items-center gap-2 px-3 py-2 text-left"
      >
        {expanded ? (
          <ChevronDown size={14} className="text-text-muted" />
        ) : (
          <ChevronRight size={14} className="text-text-muted" />
        )}
        <Terminal size={14} className="text-text-secondary" />
        <span className="text-sm font-mono text-text-primary">
          {card.toolName}
        </span>
        <span className="ml-auto flex items-center gap-2">
          {card.durationMs !== undefined && (
            <span className="text-xs text-text-muted">
              {card.durationMs}ms
            </span>
          )}
          {statusIcon}
          <Badge variant={statusBadge}>
            {card.status}
          </Badge>
        </span>
      </button>

      {expanded && (
        <div className="px-3 pb-3 space-y-2 border-t border-border/50 pt-2">
          <div>
            <div className="text-xs font-medium text-text-muted mb-1">Input</div>
            <pre className="text-xs font-mono bg-bg-dark/50 rounded p-2 overflow-x-auto max-h-40 overflow-y-auto text-text-secondary">
              {JSON.stringify(card.input, null, 2)}
            </pre>
          </div>
          {card.output !== undefined && (
            <div>
              <div className="text-xs font-medium text-text-muted mb-1">Output</div>
              <pre className="text-xs font-mono bg-bg-dark/50 rounded p-2 overflow-x-auto max-h-40 overflow-y-auto text-text-secondary">
                {typeof card.output === "string"
                  ? card.output
                  : JSON.stringify(card.output, null, 2)}
              </pre>
            </div>
          )}
          {card.error && (
            <div>
              <div className="text-xs font-medium text-alert-red mb-1">Error</div>
              <pre className="text-xs font-mono bg-alert-red/10 rounded p-2 overflow-x-auto text-alert-red">
                {card.error}
              </pre>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
