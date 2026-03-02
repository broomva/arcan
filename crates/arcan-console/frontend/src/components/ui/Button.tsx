import { cn } from "../../lib/cn";
import type { ButtonHTMLAttributes } from "react";

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: "primary" | "secondary" | "danger" | "ghost";
  size?: "sm" | "md" | "lg";
}

const variants = {
  primary:
    "bg-ai-blue hover:bg-ai-blue/90 text-white shadow-card",
  secondary:
    "bg-surface hover:bg-surface-hover text-text-primary border border-border",
  danger:
    "bg-alert-red hover:bg-alert-red/90 text-white",
  ghost:
    "bg-transparent hover:bg-surface-hover text-text-secondary",
};

const sizes = {
  sm: "px-3 py-1.5 text-xs",
  md: "px-4 py-2 text-sm",
  lg: "px-6 py-3 text-base",
};

export function Button({
  variant = "primary",
  size = "md",
  className,
  ...props
}: ButtonProps) {
  return (
    <button
      className={cn(
        "rounded-lg font-medium transition-all duration-300",
        "focus:outline-none focus:ring-2 focus:ring-ai-blue/50",
        "disabled:opacity-50 disabled:cursor-not-allowed",
        variants[variant],
        sizes[size],
        className,
      )}
      {...props}
    />
  );
}
