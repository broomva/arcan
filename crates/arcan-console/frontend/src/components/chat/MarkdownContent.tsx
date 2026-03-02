import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import rehypeHighlight from "rehype-highlight";
import { useCallback, useState } from "react";
import { Check, Copy } from "lucide-react";
import { cn } from "../../lib/cn";

interface MarkdownContentProps {
  content: string;
  className?: string;
}

export function MarkdownContent({ content, className }: MarkdownContentProps) {
  return (
    <div className={cn("prose prose-invert prose-sm max-w-none", className)}>
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        rehypePlugins={[rehypeHighlight]}
        components={{
          pre: PreBlock,
          code: InlineCode,
          a: ExternalLink,
          table: ResponsiveTable,
        }}
      >
        {content}
      </ReactMarkdown>
    </div>
  );
}

function PreBlock({ children, ...props }: React.HTMLAttributes<HTMLPreElement>) {
  return (
    <div className="relative group rounded-lg overflow-hidden my-3">
      <CopyButton getText={() => {
        // Extract text content from code element inside pre
        if (typeof children === "object" && children !== null) {
          const child = children as React.ReactElement<{ children?: React.ReactNode }>;
          return String(child.props?.children ?? "");
        }
        return String(children ?? "");
      }} />
      <pre
        className="bg-bg-dark/80 border border-border rounded-lg p-4 overflow-x-auto text-xs font-mono"
        {...props}
      >
        {children}
      </pre>
    </div>
  );
}

function InlineCode({
  className,
  children,
  ...props
}: React.HTMLAttributes<HTMLElement>) {
  // If it has a className (from highlight), it's a code block inside <pre>
  if (className) {
    return (
      <code className={className} {...props}>
        {children}
      </code>
    );
  }
  return (
    <code
      className="bg-surface px-1.5 py-0.5 rounded text-xs font-mono text-ai-blue"
      {...props}
    >
      {children}
    </code>
  );
}

function ExternalLink({ href, children, ...props }: React.AnchorHTMLAttributes<HTMLAnchorElement>) {
  return (
    <a
      href={href}
      target="_blank"
      rel="noopener noreferrer"
      className="text-ai-blue hover:text-ai-blue/80 underline underline-offset-2"
      {...props}
    >
      {children}
    </a>
  );
}

function ResponsiveTable({ children, ...props }: React.TableHTMLAttributes<HTMLTableElement>) {
  return (
    <div className="overflow-x-auto rounded-lg border border-border my-3">
      <table className="min-w-full text-sm" {...props}>
        {children}
      </table>
    </div>
  );
}

function CopyButton({ getText }: { getText: () => string }) {
  const [copied, setCopied] = useState(false);

  const copy = useCallback(() => {
    navigator.clipboard.writeText(getText());
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  }, [getText]);

  return (
    <button
      onClick={copy}
      className={cn(
        "absolute top-2 right-2 p-1.5 rounded-md",
        "bg-surface/80 border border-border",
        "opacity-0 group-hover:opacity-100 transition-opacity",
        "text-text-muted hover:text-text-primary",
      )}
      title="Copy code"
    >
      {copied ? <Check size={14} /> : <Copy size={14} />}
    </button>
  );
}
