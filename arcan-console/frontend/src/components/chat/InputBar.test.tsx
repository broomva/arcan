import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { InputBar } from "./InputBar";

describe("InputBar", () => {
  it("renders with placeholder text", () => {
    render(<InputBar onSubmit={vi.fn()} placeholder="Type here..." />);
    expect(screen.getByPlaceholderText("Type here...")).toBeInTheDocument();
  });

  it("calls onSubmit with trimmed text on Enter", () => {
    const onSubmit = vi.fn();
    render(<InputBar onSubmit={onSubmit} />);

    const textarea = screen.getByRole("textbox");
    fireEvent.change(textarea, { target: { value: "  hello world  " } });
    fireEvent.keyDown(textarea, { key: "Enter", shiftKey: false });

    expect(onSubmit).toHaveBeenCalledWith("hello world");
  });

  it("does not submit on Shift+Enter (allows newline)", () => {
    const onSubmit = vi.fn();
    render(<InputBar onSubmit={onSubmit} />);

    const textarea = screen.getByRole("textbox");
    fireEvent.change(textarea, { target: { value: "line 1" } });
    fireEvent.keyDown(textarea, { key: "Enter", shiftKey: true });

    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("does not submit empty or whitespace-only input", () => {
    const onSubmit = vi.fn();
    render(<InputBar onSubmit={onSubmit} />);

    const textarea = screen.getByRole("textbox");
    fireEvent.change(textarea, { target: { value: "   " } });
    fireEvent.keyDown(textarea, { key: "Enter", shiftKey: false });

    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("clears input after successful submit", () => {
    const onSubmit = vi.fn();
    render(<InputBar onSubmit={onSubmit} />);

    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    fireEvent.change(textarea, { target: { value: "hello" } });
    fireEvent.keyDown(textarea, { key: "Enter", shiftKey: false });

    expect(textarea.value).toBe("");
  });

  it("disables textarea when disabled prop is true", () => {
    render(<InputBar onSubmit={vi.fn()} disabled />);
    expect(screen.getByRole("textbox")).toBeDisabled();
  });

  it("does not submit when disabled", () => {
    const onSubmit = vi.fn();
    render(<InputBar onSubmit={onSubmit} disabled />);

    const textarea = screen.getByRole("textbox");
    fireEvent.change(textarea, { target: { value: "hello" } });
    fireEvent.keyDown(textarea, { key: "Enter", shiftKey: false });

    expect(onSubmit).not.toHaveBeenCalled();
  });
});
