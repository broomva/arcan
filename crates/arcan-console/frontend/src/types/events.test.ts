import { describe, it, expect } from "vitest";
import { eventKindName, eventKindPayload, type EventKind } from "./events";

describe("eventKindName", () => {
  it("extracts the discriminant from a RunStarted event", () => {
    const kind: EventKind = { RunStarted: { objective: "hello", run_id: "r-1" } };
    expect(eventKindName(kind)).toBe("RunStarted");
  });

  it("extracts the discriminant from a ToolCallRequested event", () => {
    const kind: EventKind = {
      ToolCallRequested: {
        call_id: "tc-1",
        tool_name: "bash",
        input: {},
      },
    };
    expect(eventKindName(kind)).toBe("ToolCallRequested");
  });

  it("returns Unknown for an empty object", () => {
    const kind = {} as EventKind;
    expect(eventKindName(kind)).toBe("Unknown");
  });
});

describe("eventKindPayload", () => {
  it("returns the inner payload", () => {
    const kind: EventKind = { AssistantTextDelta: { delta: "hello" } };
    expect(eventKindPayload(kind)).toEqual({ delta: "hello" });
  });

  it("returns the kind itself if no discriminant found", () => {
    const kind = {} as EventKind;
    expect(eventKindPayload(kind)).toEqual({});
  });
});
