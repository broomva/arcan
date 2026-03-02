import { cn } from "../../lib/cn";
import type { HTMLAttributes } from "react";

interface BadgeProps extends HTMLAttributes<HTMLSpanElement> {
  variant?: "default" | "success" | "warning" | "error" | "info";
}

const variants = {
  default: "bg-surface text-text-secondary border-border",
  success: "bg-success-green/15 text-success-green border-success-green/30",
  warning: "bg-amber-500/15 text-amber-400 border-amber-500/30",
  error: "bg-alert-red/15 text-alert-red border-alert-red/30",
  info: "bg-ai-blue/15 text-ai-blue border-ai-blue/30",
};

export function Badge({ variant = "default", className, ...props }: BadgeProps) {
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1 rounded-full px-2.5 py-0.5",
        "text-xs font-medium border",
        variants[variant],
        className,
      )}
      {...props}
    />
  );
}
