import { User } from "lucide-react";
import type { ChatMessage } from "../../hooks/useArcanSession";

interface UserMessageProps {
  message: ChatMessage;
}

export function UserMessage({ message }: UserMessageProps) {
  return (
    <div className="flex gap-3 px-4 py-3">
      <div className="w-7 h-7 rounded-full bg-ai-blue/20 border border-ai-blue/30 flex items-center justify-center shrink-0 mt-0.5">
        <User size={14} className="text-ai-blue" />
      </div>
      <div className="flex-1 min-w-0">
        <div className="text-xs font-medium text-ai-blue mb-1">You</div>
        <div className="text-sm text-text-primary whitespace-pre-wrap break-words">
          {message.content}
        </div>
      </div>
    </div>
  );
}
