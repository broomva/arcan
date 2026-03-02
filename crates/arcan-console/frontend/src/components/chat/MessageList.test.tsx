import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { MessageList } from "./MessageList";
import type { ChatMessage } from "../../hooks/useArcanSession";

describe("MessageList", () => {
  it("shows empty state when no messages and not running", () => {
    render(<MessageList messages={[]} streamingText="" isRunning={false} />);
    expect(screen.getByText("Arcan Agent Runtime")).toBeInTheDocument();
  });

  it("renders user messages", () => {
    const messages: ChatMessage[] = [
      { id: "1", role: "user", content: "Hello agent", timestamp: new Date().toISOString() },
    ];
    render(<MessageList messages={messages} streamingText="" isRunning={false} />);
    expect(screen.getByText("Hello agent")).toBeInTheDocument();
    expect(screen.getByText("You")).toBeInTheDocument();
  });

  it("renders assistant messages", () => {
    const messages: ChatMessage[] = [
      { id: "1", role: "assistant", content: "I can help!", timestamp: new Date().toISOString() },
    ];
    render(<MessageList messages={messages} streamingText="" isRunning={false} />);
    expect(screen.getByText("I can help!")).toBeInTheDocument();
  });

  it("shows streaming indicator when running", () => {
    render(<MessageList messages={[]} streamingText="" isRunning={true} />);
    expect(screen.getByText("Thinking...")).toBeInTheDocument();
  });

  it("shows streaming text when available", () => {
    render(
      <MessageList messages={[]} streamingText="Working on it..." isRunning={true} />,
    );
    expect(screen.getByText("Working on it...")).toBeInTheDocument();
  });

  it("renders multiple messages in order", () => {
    const messages: ChatMessage[] = [
      { id: "1", role: "user", content: "First message", timestamp: "2026-01-01T00:00:00Z" },
      { id: "2", role: "assistant", content: "Response here", timestamp: "2026-01-01T00:00:01Z" },
      { id: "3", role: "user", content: "Follow up", timestamp: "2026-01-01T00:00:02Z" },
    ];
    render(<MessageList messages={messages} streamingText="" isRunning={false} />);
    expect(screen.getByText("First message")).toBeInTheDocument();
    expect(screen.getByText("Response here")).toBeInTheDocument();
    expect(screen.getByText("Follow up")).toBeInTheDocument();
  });
});
