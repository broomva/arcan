import { useState } from "react";
import { PanelRightOpen, PanelRightClose, AlertTriangle } from "lucide-react";
import { cn } from "../../lib/cn";
import { MessageList } from "./MessageList";
import { InputBar } from "./InputBar";
import { ApprovalBanner } from "./ApprovalBanner";
import { StateInspector } from "./StateInspector";
import type { SessionState } from "../../hooks/useArcanSession";

interface ChatViewProps {
  session: SessionState;
  onSendMessage: (content: string) => void;
  onResolveApproval: (approvalId: string, approved: boolean) => void;
}

export function ChatView({ session, onSendMessage, onResolveApproval }: ChatViewProps) {
  const [showInspector, setShowInspector] = useState(false);

  return (
    <div className="flex flex-1 overflow-hidden">
      {/* Main chat area */}
      <div className="flex-1 flex flex-col overflow-hidden">
        {/* Header */}
        <div className="flex items-center justify-between px-4 py-3 border-b border-border bg-bg-dark/30">
          <div className="flex items-center gap-2">
            <h2 className="text-sm font-semibold text-text-primary truncate">
              {session.sessionId || "No session"}
            </h2>
            {session.operatingMode && (
              <span className={cn(
                "text-xs px-2 py-0.5 rounded-full",
                modeStyle(session.operatingMode),
              )}>
                {session.operatingMode}
              </span>
            )}
          </div>
          <button
            onClick={() => setShowInspector(!showInspector)}
            className="text-text-muted hover:text-text-primary transition-colors p-1"
            title={showInspector ? "Hide state inspector" : "Show state inspector"}
          >
            {showInspector ? <PanelRightClose size={18} /> : <PanelRightOpen size={18} />}
          </button>
        </div>

        {/* Error banner */}
        {session.error && (
          <div className="mx-4 mt-2 flex items-center gap-2 rounded-lg border border-alert-red/30 bg-alert-red/5 px-3 py-2">
            <AlertTriangle size={14} className="text-alert-red shrink-0" />
            <span className="text-sm text-alert-red">{session.error}</span>
          </div>
        )}

        {/* Messages */}
        <MessageList
          messages={session.messages}
          streamingText={session.streamingText}
          isRunning={session.isRunning}
        />

        {/* Approval banner */}
        {session.pendingApproval && (
          <ApprovalBanner
            approval={session.pendingApproval}
            onResolve={onResolveApproval}
          />
        )}

        {/* Input */}
        <InputBar
          onSubmit={onSendMessage}
          disabled={session.isRunning || !session.sessionId}
          placeholder={
            !session.sessionId
              ? "Create or select a session to start..."
              : session.isRunning
                ? "Agent is working..."
                : "Send a message..."
          }
        />
      </div>

      {/* State inspector panel */}
      {showInspector && (
        <StateInspector
          state={session.agentState}
          mode={session.operatingMode}
          onClose={() => setShowInspector(false)}
        />
      )}
    </div>
  );
}

function modeStyle(mode: string): string {
  switch (mode) {
    case "Explore": return "bg-ai-blue/15 text-ai-blue";
    case "Execute": return "bg-web3-green/15 text-web3-green";
    case "Verify": return "bg-amber-500/15 text-amber-400";
    case "Recover": return "bg-alert-red/15 text-alert-red";
    case "AskHuman": return "bg-purple-500/15 text-purple-400";
    case "Sleep": return "bg-surface text-text-muted";
    default: return "bg-surface text-text-secondary";
  }
}
