import { X, Activity, Shield, Cpu, Gauge } from "lucide-react";
import { cn } from "../../lib/cn";
import { Badge } from "../ui/Badge";
import type { AgentStateVector, OperatingMode } from "../../types/api";

interface StateInspectorProps {
  state: AgentStateVector | null;
  mode: OperatingMode | null;
  onClose: () => void;
}

const modeColors: Record<string, string> = {
  Explore: "text-ai-blue",
  Execute: "text-web3-green",
  Verify: "text-amber-400",
  Recover: "text-alert-red",
  AskHuman: "text-purple-400",
  Sleep: "text-text-muted",
};

export function StateInspector({ state, mode, onClose }: StateInspectorProps) {
  if (!state) {
    return (
      <div className="w-72 border-l border-border bg-bg-dark/50 p-4">
        <div className="flex items-center justify-between mb-4">
          <h3 className="text-sm font-semibold text-text-primary">State Inspector</h3>
          <button onClick={onClose} className="text-text-muted hover:text-text-primary">
            <X size={16} />
          </button>
        </div>
        <p className="text-xs text-text-muted">No state data available yet.</p>
      </div>
    );
  }

  return (
    <div className="w-72 border-l border-border bg-bg-dark/50 overflow-y-auto">
      <div className="sticky top-0 bg-bg-dark/90 backdrop-blur-sm p-4 border-b border-border z-10">
        <div className="flex items-center justify-between">
          <h3 className="text-sm font-semibold text-text-primary">State Inspector</h3>
          <button onClick={onClose} className="text-text-muted hover:text-text-primary">
            <X size={16} />
          </button>
        </div>
      </div>

      <div className="p-4 space-y-4">
        {/* Operating Mode */}
        <Section icon={<Cpu size={14} />} title="Mode">
          <span className={cn("text-sm font-semibold", modeColors[mode ?? ""] ?? "text-text-primary")}>
            {mode ?? "Unknown"}
          </span>
        </Section>

        {/* Progress */}
        <Section icon={<Gauge size={14} />} title="Progress">
          <ProgressBar value={state.progress} color="bg-web3-green" />
          <div className="flex justify-between text-xs text-text-muted mt-1">
            <span>{(state.progress * 100).toFixed(0)}%</span>
            <span>Uncertainty: {(state.uncertainty * 100).toFixed(0)}%</span>
          </div>
        </Section>

        {/* Risk */}
        <Section icon={<Shield size={14} />} title="Risk">
          <Badge variant={riskVariant(state.risk_level)}>{state.risk_level}</Badge>
          <div className="grid grid-cols-2 gap-2 mt-2 text-xs">
            <Metric label="Error streak" value={state.error_streak} />
            <Metric label="Context press." value={`${(state.context_pressure * 100).toFixed(0)}%`} />
            <Metric label="Side-effect" value={`${(state.side_effect_pressure * 100).toFixed(0)}%`} />
            <Metric label="Human dep." value={`${(state.human_dependency * 100).toFixed(0)}%`} />
          </div>
        </Section>

        {/* Budget */}
        <Section icon={<Activity size={14} />} title="Budget">
          <div className="space-y-1.5 text-xs">
            <Metric label="Tokens" value={state.budget.tokens_remaining.toLocaleString()} />
            <Metric label="Time" value={`${(state.budget.time_remaining_ms / 1000).toFixed(0)}s`} />
            <Metric label="Cost" value={`$${state.budget.cost_remaining_usd.toFixed(4)}`} />
            <Metric label="Tool calls" value={state.budget.tool_calls_remaining} />
            <Metric label="Error budget" value={state.budget.error_budget_remaining} />
          </div>
        </Section>
      </div>
    </div>
  );
}

function Section({
  icon,
  title,
  children,
}: {
  icon: React.ReactNode;
  title: string;
  children: React.ReactNode;
}) {
  return (
    <div className="rounded-lg border border-border bg-surface/30 p-3">
      <div className="flex items-center gap-1.5 text-xs font-medium text-text-secondary mb-2">
        {icon}
        {title}
      </div>
      {children}
    </div>
  );
}

function ProgressBar({ value, color }: { value: number; color: string }) {
  return (
    <div className="h-2 rounded-full bg-bg-dark/50 overflow-hidden">
      <div
        className={cn("h-full rounded-full transition-all duration-500", color)}
        style={{ width: `${Math.min(value * 100, 100)}%` }}
      />
    </div>
  );
}

function Metric({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div className="flex justify-between">
      <span className="text-text-muted">{label}</span>
      <span className="text-text-primary font-mono">{value}</span>
    </div>
  );
}

function riskVariant(level: string): "success" | "warning" | "error" | "info" {
  switch (level) {
    case "Low": return "success";
    case "Medium": return "warning";
    case "High":
    case "Critical": return "error";
    default: return "info";
  }
}
