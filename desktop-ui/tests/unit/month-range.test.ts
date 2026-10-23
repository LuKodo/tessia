import { describe, expect, it } from "vitest";
import { monthOf, monthRange } from "../../src/utils/month-range";

// Ported from webui-src/src/rips/monthRange.test.js (same cases and expectations).
describe("month range", () => {
  it("monthOf takes the local year-month of a date", () => {
    expect(monthOf(new Date(2026, 9, 6))).toBe("2026-10");
    expect(monthOf(new Date(2026, 0, 31))).toBe("2026-01");
  });

  it("monthRange spans the first to the last day of the month", () => {
    expect(monthRange("2026-09")).toEqual({ desde: "2026-09-01", hasta: "2026-09-30" });
    expect(monthRange("2026-02")).toEqual({ desde: "2026-02-01", hasta: "2026-02-28" });
    expect(monthRange("2028-02")).toEqual({ desde: "2028-02-01", hasta: "2028-02-29" });
    expect(monthRange("2026-12")).toEqual({ desde: "2026-12-01", hasta: "2026-12-31" });
  });
});
