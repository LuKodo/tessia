import type { ReactiveController, ReactiveControllerHost } from "lit";
import type { ConsultaParams } from "../types/api";
import { KPI_FILTER } from "../utils/rips-constants";
import type { KpiKey, TextFilterKey } from "../utils/rips-constants";
import {
  computeApplied,
  countActive,
  emptyDraft,
  hasCargueFilter,
  isKPIActive,
  isTextActive,
  toConsultaParams,
  toggleAlertDraft,
  togglePipelineDraft,
  toggleTextDraft,
} from "../utils/rips-filter-model";
import type { AppliedFilters, FilterDraft } from "../utils/rips-filter-model";
import { nextSort } from "../utils/table-model";
import type { SortState } from "../utils/table-model";

/**
 * Port of useRipsFilters. Owns both the "draft" form state (what is typed/selected/checked right
 * now) and the "applied" snapshot the server is asked to filter the table with, plus the table
 * sort. `query` is the filter + sort part of the consulta request; `onQueryChange` runs whenever it
 * changes (applied filters or sort), so the owner can restart its paging. Text inputs, date range and
 * alert checkboxes only take effect once committed: Filtrar/Limpiar commit everything; a KPI click
 * commits immediately (pipeline toggle or alert toggle); a select's own "activar" button commits
 * the current draft (selects never filter live on change).
 *
 * The owner element re-renders on every change (it is a reactive controller); child elements that
 * receive the controller as a property follow it through `subscribe` (see FiltersBinding), because
 * the object identity does not change between renders.
 */
export class RipsFiltersController implements ReactiveController {
  /** Immutable: replaced, never mutated. */
  draft: FilterDraft = emptyDraft();

  /** The table sort (applied at once; the server sorts). */
  sort: SortState = { key: null, dir: 1 };

  private applied: AppliedFilters = computeApplied(this.draft);
  private params: ConsultaParams = toConsultaParams(this.applied, this.sort);
  private listeners = new Set<() => void>();

  constructor(
    private host?: ReactiveControllerHost,
    private onQueryChange?: () => void,
  ) {
    host?.addController(this);
  }

  hostConnected(): void {}

  setText(key: TextFilterKey, value: string): void {
    this.draft = { ...this.draft, text: { ...this.draft.text, [key]: value } };
    this.changed();
  }

  setField(key: "estado" | "pipeline" | "desde" | "hasta", value: string): void {
    this.draft = { ...this.draft, [key]: value };
    this.changed();
  }

  commit(): void {
    this.apply(this.draft);
  }

  clear(): void {
    this.replaceAndCommit(emptyDraft());
  }

  togglePipeline(value: string): void {
    this.replaceAndCommit(togglePipelineDraft(this.draft, value));
  }

  toggleAlert(value: string): void {
    this.replaceAndCommit(toggleAlertDraft(this.draft, value));
  }

  /** Commits a text filter straight away (chart bar click); again clears it. */
  toggleText(key: TextFilterKey, value: string): void {
    this.replaceAndCommit(toggleTextDraft(this.draft, key, value));
  }

  clickKPI(key: KpiKey): void {
    const target = KPI_FILTER[key];
    if (!target) return;
    if (target.type === "pipeline") this.togglePipeline(target.value);
    else this.toggleAlert(target.value);
  }

  isKPIActive(key: KpiKey): boolean {
    return isKPIActive(this.draft, key);
  }

  isTextActive(key: TextFilterKey, value: string): boolean {
    return isTextActive(this.draft, key, value);
  }

  get activeCount(): number {
    return countActive(this.applied);
  }

  /** Filters and sort for the consulta route. Keeps its identity until they change. */
  get query(): ConsultaParams {
    return this.params;
  }

  /** A click on a sortable column header: a new column starts descending, the same one flips. */
  sortBy(key: string): void {
    this.sort = nextSort(this.sort, key);
    this.params = toConsultaParams(this.applied, this.sort);
    this.changed();
    this.onQueryChange?.();
  }

  get alertsMulti(): boolean {
    return this.applied.alerts.length > 1;
  }

  get cargueActive(): boolean {
    return hasCargueFilter(this.applied);
  }

  /** Calls `listener` after every change; returns the unsubscribe function. */
  subscribe(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  private replaceAndCommit(next: FilterDraft): void {
    this.draft = next;
    this.apply(next);
  }

  // The query is rebuilt only here and in sortBy, so it keeps its identity until the applied filters change.
  private apply(draft: FilterDraft): void {
    this.applied = computeApplied(draft);
    this.params = toConsultaParams(this.applied, this.sort);
    this.changed();
    this.onQueryChange?.();
  }

  private changed(): void {
    this.host?.requestUpdate();
    for (const listener of [...this.listeners]) listener();
  }
}

/**
 * Lets a child element follow a RipsFiltersController it receives as a property: call `follow`
 * with the current controller from `willUpdate`; the host re-renders whenever the filters change.
 */
export class FiltersBinding implements ReactiveController {
  private current: RipsFiltersController | null = null;
  private off: (() => void) | undefined;

  constructor(private host: ReactiveControllerHost) {
    host.addController(this);
  }

  follow(filters: RipsFiltersController | null | undefined): void {
    const next = filters ?? null;
    if (next === this.current) return;
    this.off?.();
    this.current = next;
    this.off = next?.subscribe(() => this.host.requestUpdate());
  }

  hostConnected(): void {
    // Reconnected after hostDisconnected released the subscription.
    if (this.current && !this.off) this.off = this.current.subscribe(() => this.host.requestUpdate());
  }

  hostDisconnected(): void {
    this.off?.();
    this.off = undefined;
  }
}
