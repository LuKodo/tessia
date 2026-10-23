import { describe, expect, it } from "vitest";
import { ApiError } from "../../src/utils/api-client";
import { connectivityHint, isNetworkFailure } from "../../src/utils/connectivity-hint";

const BASE = "http://localhost:8090";
const ORIGIN = "http://localhost:3000";

describe("isNetworkFailure", () => {
  it("is true for an ApiError with status 0 and for a fetch TypeError", () => {
    expect(isNetworkFailure(new ApiError(0, "sin conexion"))).toBe(true);
    expect(isNetworkFailure(new TypeError("Failed to fetch"))).toBe(true);
  });

  it("is false for HTTP errors and unrelated values", () => {
    expect(isNetworkFailure(new ApiError(500, "boom"))).toBe(false);
    expect(isNetworkFailure(new Error("x"))).toBe(false);
    expect(isNetworkFailure(null)).toBe(false);
  });
});

describe("connectivityHint", () => {
  it("names the API base and the page origin on a network failure, without CORS flags", () => {
    const hint = connectivityHint(new ApiError(0, "sin conexion"), BASE, ORIGIN);
    expect(hint).toContain(BASE);
    expect(hint).toContain(ORIGIN);
    expect(hint).toContain("reinicie la aplicación");
    expect(hint).not.toContain("allowed-origin");
    expect(hint).not.toContain("CORS");
  });

  it("is empty for an HTTP 500", () => {
    expect(connectivityHint(new ApiError(500, "boom"), BASE, ORIGIN)).toBe("");
  });

  it("is empty when there is no error (ok)", () => {
    expect(connectivityHint(null, BASE, ORIGIN)).toBe("");
  });

  it("drops trailing slashes of the API base", () => {
    const hint = connectivityHint(new TypeError("Failed to fetch"), `${BASE}//`, ORIGIN);
    expect(hint).toContain(`${BASE}.`);
    expect(hint).not.toContain(`${BASE}/`);
  });

  it("falls back to the page origin when the API base is empty (same-origin)", () => {
    const hint = connectivityHint(new ApiError(0, "x"), "", ORIGIN);
    expect(hint).toContain(`${ORIGIN}.`);
  });
});
