import { describe, expect, it } from "vitest";
import { DEFAULT_API_BASE, resolveApiBase } from "../../src/config";

describe("resolveApiBase", () => {
  it("defaults to the local service when unset", () => {
    expect(resolveApiBase(undefined)).toBe(DEFAULT_API_BASE);
  });

  it("keeps an explicit empty string, meaning same origin", () => {
    expect(resolveApiBase("")).toBe("");
  });

  it("uses the configured value", () => {
    expect(resolveApiBase("http://api.test:9000")).toBe("http://api.test:9000");
  });
});
