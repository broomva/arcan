import { useState, useCallback, useRef, useEffect } from "react";
import { Send } from "lucide-react";
import { cn } from "../../lib/cn";

interface InputBarProps {
  onSubmit: (message: string) => void;
  disabled?: boolean;
  placeholder?: string;
}

export function InputBar({ onSubmit, disabled, placeholder }: InputBarProps) {
  const [value, setValue] = useState("");
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const submit = useCallback(() => {
    const trimmed = value.trim();
    if (!trimmed || disabled) return;
    onSubmit(trimmed);
    setValue("");
    // Reset textarea height
    if (textareaRef.current) {
      textareaRef.current.style.height = "auto";
    }
  }, [value, disabled, onSubmit]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        submit();
      }
    },
    [submit],
  );

  // Auto-resize textarea
  useEffect(() => {
    const textarea = textareaRef.current;
    if (!textarea) return;
    textarea.style.height = "auto";
    textarea.style.height = `${Math.min(textarea.scrollHeight, 200)}px`;
  }, [value]);

  return (
    <div className="border-t border-border bg-bg-dark/50 p-4">
      <div
        className={cn(
          "flex items-end gap-2 rounded-xl border bg-surface",
          "transition-colors duration-200",
          disabled
            ? "border-border/50 opacity-60"
            : "border-border focus-within:border-ai-blue/50",
        )}
      >
        <textarea
          ref={textareaRef}
          value={value}
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={handleKeyDown}
          disabled={disabled}
          placeholder={placeholder ?? "Send a message..."}
          rows={1}
          className={cn(
            "flex-1 resize-none bg-transparent px-4 py-3",
            "text-sm text-text-primary placeholder:text-text-muted",
            "focus:outline-none",
            "min-h-[44px] max-h-[200px]",
          )}
        />
        <button
          onClick={submit}
          disabled={disabled || !value.trim()}
          className={cn(
            "p-3 rounded-xl transition-colors",
            value.trim() && !disabled
              ? "text-ai-blue hover:bg-ai-blue/10"
              : "text-text-muted cursor-not-allowed",
          )}
          title="Send message (Enter)"
        >
          <Send size={18} />
        </button>
      </div>
      <p className="text-[10px] text-text-muted mt-1.5 px-1">
        Press Enter to send, Shift+Enter for new line
      </p>
    </div>
  );
}
