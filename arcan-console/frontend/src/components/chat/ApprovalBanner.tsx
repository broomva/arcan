import { ShieldAlert, Check, X } from "lucide-react";
import { Button } from "../ui/Button";
import { Badge } from "../ui/Badge";
import type { ApprovalInfo } from "../../types/events";

interface ApprovalBannerProps {
  approval: ApprovalInfo;
  onResolve: (approvalId: string, approved: boolean) => void;
}

export function ApprovalBanner({ approval, onResolve }: ApprovalBannerProps) {
  const riskVariant =
    approval.risk_level === "Critical" || approval.risk_level === "High"
      ? "error"
      : approval.risk_level === "Medium"
        ? "warning"
        : "info";

  return (
    <div className="mx-4 my-2 rounded-lg border border-amber-500/30 bg-amber-500/5 p-4">
      <div className="flex items-start gap-3">
        <ShieldAlert size={20} className="text-amber-400 shrink-0 mt-0.5" />
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2 mb-1">
            <span className="text-sm font-semibold text-text-primary">
              Approval Required
            </span>
            <Badge variant={riskVariant}>{approval.risk_level}</Badge>
          </div>
          <p className="text-sm text-text-secondary mb-2">
            {approval.reason}
          </p>
          <div className="text-xs text-text-muted mb-3">
            Tool: <code className="font-mono text-ai-blue">{approval.tool_name}</code>
          </div>
          {approval.input != null && (
            <pre className="text-xs font-mono bg-bg-dark/50 rounded p-2 mb-3 overflow-x-auto max-h-32 overflow-y-auto text-text-secondary">
              {typeof approval.input === "string"
                ? approval.input
                : JSON.stringify(approval.input, null, 2)}
            </pre>
          )}
          <div className="flex gap-2">
            <Button
              variant="primary"
              size="sm"
              onClick={() => onResolve(approval.approval_id, true)}
            >
              <Check size={14} className="mr-1" />
              Approve
            </Button>
            <Button
              variant="danger"
              size="sm"
              onClick={() => onResolve(approval.approval_id, false)}
            >
              <X size={14} className="mr-1" />
              Deny
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
}
