import { Bot } from "lucide-react";
import { MarkdownContent } from "./MarkdownContent";

interface StreamingIndicatorProps {
  text: string;
}

export function StreamingIndicator({ text }: StreamingIndicatorProps) {
  return (
    <div className="flex gap-3 px-4 py-3">
      <div className="w-7 h-7 rounded-full bg-web3-green/20 border border-web3-green/30 flex items-center justify-center shrink-0 mt-0.5">
        <Bot size={14} className="text-web3-green" />
      </div>
      <div className="flex-1 min-w-0">
        <div className="text-xs font-medium text-web3-green mb-1">Arcan</div>
        {text ? (
          <div className="streaming-cursor">
            <MarkdownContent content={text} />
          </div>
        ) : (
          <div className="flex items-center gap-2 text-sm text-text-muted">
            <span className="flex gap-1">
              <span className="w-1.5 h-1.5 rounded-full bg-ai-blue animate-bounce [animation-delay:0ms]" />
              <span className="w-1.5 h-1.5 rounded-full bg-ai-blue animate-bounce [animation-delay:150ms]" />
              <span className="w-1.5 h-1.5 rounded-full bg-ai-blue animate-bounce [animation-delay:300ms]" />
            </span>
            Thinking...
          </div>
        )}
      </div>
    </div>
  );
}
