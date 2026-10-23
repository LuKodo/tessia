import type { ReactiveController, ReactiveControllerHost } from "lit";
import type { Hallazgo } from "../types/api";
import type { TessiaApi } from "../utils/tessia-api";
import { errorMessage } from "../utils/error-message";
import {
  buildPayload,
  descriptionError,
  filterHallazgos,
  needsSave,
  normalizeDescription,
  summaryText,
} from "../utils/hallazgos-model";

const SAVED_FLASH_MS = 2000;

export type RowStatus = "idle" | "saving" | "saved" | "error";

/** The slice of the API this controller needs (a fake can implement just these two). */
export type HallazgosApi = Pick<TessiaApi, "hallazgos" | "updateHallazgo">;

/**
 * A catalog entry plus its edit state. `genera_glosa`, `devolucion` and `descripcion` are the
 * edited draft; `saved` holds the last values the server confirmed.
 */
export interface HallazgoRow extends Hallazgo {
  saved: { genera_glosa: boolean; devolucion: boolean; descripcion: string };
  status: RowStatus;
  message: string;
  /** At most one PUT per row is in flight; edits made meanwhile re-run save() after it. */
  inflight: boolean;
  queued: boolean;
}

function toRow(item: Hallazgo): HallazgoRow {
  const genera = Boolean(item.genera_glosa);
  const devolucion = Boolean(item.devolucion); // absent on older backends
  const descripcion = item.descripcion || "";
  return {
    codigo: item.codigo,
    detalle: item.detalle || "",
    genera_glosa: genera,
    devolucion,
    descripcion,
    saved: { genera_glosa: genera, devolucion, descripcion },
    status: "idle",
    message: "",
    inflight: false,
    queued: false,
  };
}

/**
 * State and actions of the Hallazgos page: loads GET /api/hallazgos when the host connects and saves
 * each edited row with PUT /api/hallazgos/{codigo}. Port of `useHallazgos`. Rows are mutated in place
 * (as the Vue refs were) and the host is asked to re-render after every change.
 * `api` may be a getter so the host can swap its API after the controller was created.
 */
export class HallazgosController implements ReactiveController {
  rows: HallazgoRow[] = [];
  loading = true;
  loadError = "";

  private currentQuery = "";
  // Guards state updates after disconnect; also stops the "Guardado" timers.
  private active = false;
  private flashTimers = new Map<string, ReturnType<typeof setTimeout>>();

  constructor(
    private host: ReactiveControllerHost,
    private api: HallazgosApi | (() => HallazgosApi),
  ) {
    host.addController(this);
  }

  hostConnected(): void {
    this.active = true;
    void this.load();
  }

  hostDisconnected(): void {
    this.active = false;
    for (const timer of this.flashTimers.values()) clearTimeout(timer);
    this.flashTimers.clear();
  }

  get query(): string {
    return this.currentQuery;
  }

  set query(value: string) {
    this.currentQuery = value;
    this.host.requestUpdate();
  }

  get shown(): HallazgoRow[] {
    return filterHallazgos(this.rows, this.currentQuery);
  }

  get summary(): string {
    return summaryText(this.shown.length, this.rows.length);
  }

  private get client(): HallazgosApi {
    return typeof this.api === "function" ? this.api() : this.api;
  }

  async load(): Promise<void> {
    this.loading = true;
    this.loadError = "";
    this.host.requestUpdate();
    try {
      const body = await this.client.hallazgos();
      if (!this.active) return;
      this.rows = (body.hallazgos || []).filter((item) => item && item.codigo).map(toRow);
    } catch (err) {
      if (this.active) this.loadError = errorMessage(err);
    } finally {
      if (this.active) {
        this.loading = false;
        this.host.requestUpdate();
      }
    }
  }

  private findRow(codigo: string): HallazgoRow | undefined {
    return this.rows.find((row) => row.codigo === codigo);
  }

  private setStatus(row: HallazgoRow, status: RowStatus, message = ""): void {
    row.status = status;
    row.message = message;
    clearTimeout(this.flashTimers.get(row.codigo));
    this.flashTimers.delete(row.codigo);
    if (status === "saved") {
      this.flashTimers.set(
        row.codigo,
        setTimeout(() => {
          this.flashTimers.delete(row.codigo);
          if (row.status === "saved") {
            row.status = "idle";
            this.host.requestUpdate();
          }
        }, SAVED_FLASH_MS),
      );
    }
    this.host.requestUpdate();
  }

  private async save(row: HallazgoRow | undefined): Promise<void> {
    if (!row || !needsSave(row.saved, row)) return;
    const invalid = descriptionError(row.descripcion);
    if (invalid) {
      // Nothing is sent, so an optimistic checkbox change must not look saved.
      row.genera_glosa = row.saved.genera_glosa;
      row.devolucion = row.saved.devolucion;
      this.setStatus(row, "error", invalid);
      return;
    }
    // Serialize the PUTs of one row: responses can then never arrive out of
    // order, and the follow-up save below sends the latest draft.
    if (row.inflight) {
      row.queued = true;
      return;
    }
    const payload = buildPayload(row);
    row.inflight = true;
    this.setStatus(row, "saving");
    try {
      const updated = await this.client.updateHallazgo(row.codigo, payload);
      if (!this.active) return;
      row.saved = {
        genera_glosa: Boolean(updated.genera_glosa),
        devolucion: Boolean(updated.devolucion),
        descripcion: updated.descripcion || "",
      };
      // Adopt the server's values only for fields the user has not touched since.
      if (row.genera_glosa === payload.genera_glosa) row.genera_glosa = row.saved.genera_glosa;
      if (row.devolucion === payload.devolucion) row.devolucion = row.saved.devolucion;
      if (normalizeDescription(row.descripcion) === payload.descripcion) row.descripcion = row.saved.descripcion;
      if (updated.detalle) row.detalle = updated.detalle;
      this.setStatus(row, "saved");
    } catch (err) {
      if (!this.active) return;
      // Roll the checkboxes back; keep the typed description so the user can retry.
      if (payload.genera_glosa !== row.saved.genera_glosa) row.genera_glosa = row.saved.genera_glosa;
      if (payload.devolucion !== row.saved.devolucion) row.devolucion = row.saved.devolucion;
      this.setStatus(row, "error", errorMessage(err) || "No se pudo guardar.");
    } finally {
      row.inflight = false;
      if (row.queued && this.active) {
        row.queued = false;
        void this.save(row);
      }
    }
  }

  onToggle(codigo: string, value: boolean): void {
    const row = this.findRow(codigo);
    if (!row) return;
    row.genera_glosa = value; // optimistic
    this.host.requestUpdate();
    void this.save(row);
  }

  onToggleDevolucion(codigo: string, value: boolean): void {
    const row = this.findRow(codigo);
    if (!row) return;
    row.devolucion = value; // optimistic
    this.host.requestUpdate();
    void this.save(row);
  }

  onEdit(codigo: string, text: string): void {
    const row = this.findRow(codigo);
    if (!row) return;
    row.descripcion = text;
    this.host.requestUpdate();
  }

  onCommit(codigo: string): void {
    void this.save(this.findRow(codigo));
  }
}
