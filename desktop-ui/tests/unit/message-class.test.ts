import { describe, expect, it } from "vitest";
import { messageClass } from "../../src/utils/message-class";

describe("messageClass", () => {
  it("colors errors and successes", () => {
    expect(messageClass("error")).toBe("text-error");
    expect(messageClass("ok")).toBe("text-success");
  });

  it("mutes the neutral progress line with the muted token", () => {
    expect(messageClass("")).toBe("text-muted");
  });
});
