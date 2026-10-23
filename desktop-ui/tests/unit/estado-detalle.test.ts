import { describe, expect, it } from "vitest";
import { isPositiveDetail } from "../../src/utils/estado-detalle";

// Ported from webui-src/src/rips/estadoDetalle.test.js (same cases and expectations).
describe("isPositiveDetail", () => {
  it("empty or undefined detail is not positive", () => {
    expect(isPositiveDetail("")).toBe(false);
    expect(isPositiveDetail(undefined)).toBe(false);
    expect(isPositiveDetail(null)).toBe(false);
  });

  it("confirmado=true is positive", () => {
    expect(isPositiveDetail("confirmado=true")).toBe(true);
  });

  it("RADICADO is positive", () => {
    expect(isPositiveDetail("RADICADO")).toBe(true);
  });

  it("confirmado=false is negative even with other words", () => {
    expect(isPositiveDetail("idCargue=1 status=ok confirmado=false mensaje=x")).toBe(false);
    expect(isPositiveDetail("radicado confirmado=false")).toBe(false);
  });

  it("confirmado = FALSE tolerates spaces and case", () => {
    expect(isPositiveDetail("confirmado = FALSE")).toBe(false);
  });

  it("unrelated text is not positive", () => {
    expect(isPositiveDetail("sin novedad")).toBe(false);
  });
});
