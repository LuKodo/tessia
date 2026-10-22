import { describe, expect, it } from "vitest";
import { fmtTime } from "../../src/utils/format";
import { errorMessage } from "../../src/utils/error-message";

describe("fmtTime", () => {
  it("is empty for a missing or invalid date", () => {
    expect(fmtTime("")).toBe("");
    expect(fmtTime(undefined)).toBe("");
    expect(fmtTime(null)).toBe("");
    expect(fmtTime("not a date")).toBe("");
  });

  it("formats a valid date in es-CO with a 24 hour clock", () => {
    const text = fmtTime("2026-10-01T15:04:05Z");
    expect(text).toMatch(/2026/);
    expect(text).not.toMatch(/[ap]\.?\s?m\./i);
  });
});

describe("errorMessage", () => {
  it("uses the message of an Error and the string form of anything else", () => {
    expect(errorMessage(new Error("boom"))).toBe("boom");
    expect(errorMessage("plain")).toBe("plain");
    expect(errorMessage(42)).toBe("42");
  });
});
