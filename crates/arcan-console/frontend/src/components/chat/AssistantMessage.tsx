import { Bot } from "lucide-react";
import type { ChatMessage } from "../../hooks/useArcanSession";
import { MarkdownContent } from "./MarkdownContent";
import { ToolExecutionCard } from "./ToolExecutionCard";

interface AssistantMessageProps {
  message: ChatMessage;
}

export function AssistantMessage({ message }: AssistantMessageProps) {
  return (
    <div className="flex gap-3 px-4 py-3">
      <div className="w-7 h-7 rounded-full bg-web3-green/20 border border-web3-green/30 flex items-center justify-center shrink-0 mt-0.5">
        <Bot size={14} className="text-web3-green" />
      </div>
      <div className="flex-1 min-w-0">
        <div className="text-xs font-medium text-web3-green mb-1">Arcan</div>
        {message.content && (
          <MarkdownContent content={message.content} />
        )}
        {message.toolCards?.map((card) => (
          <ToolExecutionCard key={card.callId} card={card} />
        ))}
      </div>
    </div>
  );
}
