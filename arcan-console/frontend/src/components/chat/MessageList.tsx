import { useEffect, useRef } from "react";
import type { ChatMessage } from "../../hooks/useArcanSession";
import { UserMessage } from "./UserMessage";
import { AssistantMessage } from "./AssistantMessage";
import { StreamingIndicator } from "./StreamingIndicator";

interface MessageListProps {
  messages: ChatMessage[];
  streamingText: string;
  isRunning: boolean;
}

export function MessageList({ messages, streamingText, isRunning }: MessageListProps) {
  const bottomRef = useRef<HTMLDivElement>(null);

  // Auto-scroll to bottom on new messages
  useEffect(() => {
    if (typeof bottomRef.current?.scrollIntoView === "function") {
      bottomRef.current.scrollIntoView({ behavior: "smooth" });
    }
  }, [messages.length, streamingText]);

  if (messages.length === 0 && !isRunning) {
    return (
      <div className="flex-1 flex items-center justify-center">
        <div className="text-center max-w-md px-4">
          <div className="w-16 h-16 rounded-2xl bg-gradient-to-br from-ai-blue to-web3-green flex items-center justify-center mx-auto mb-4">
            <span className="text-3xl font-bold text-white">A</span>
          </div>
          <h2 className="text-lg font-semibold text-text-primary mb-2">
            Arcan Agent Runtime
          </h2>
          <p className="text-sm text-text-secondary">
            Send a message to start a conversation with the agent.
            It can execute tools, read and write files, run commands,
            and reason about complex tasks.
          </p>
        </div>
      </div>
    );
  }

  return (
    <div className="flex-1 overflow-y-auto">
      <div className="max-w-4xl mx-auto py-4 space-y-1">
        {messages.map((msg) =>
          msg.role === "user" ? (
            <UserMessage key={msg.id} message={msg} />
          ) : (
            <AssistantMessage key={msg.id} message={msg} />
          ),
        )}
        {isRunning && <StreamingIndicator text={streamingText} />}
        <div ref={bottomRef} />
      </div>
    </div>
  );
}
