import type { ReactiveController, ReactiveControllerHost } from "lit";
import type { PreparationParametersResponse } from "../types/api";
import { errorMessage } from "../utils/error-message";
import {
  DEFAULT_CUV_MIN_SAMPLE,
  DEFAULT_CUV_TOKEN_FAILURE_PERCENT,
  DEFAULT_DERECHOS_MIN_SAMPLE,
  DEFAULT_DERECHOS_UNAVAILABLE_PERCENT,
  DEFAULT_WAVE_SIZE,
  selectedSubplans,
  subplanOptions,
  validateParameterForm,
} from "../utils/parameters-model";
import type { TessiaApi } from "../utils/tessia-api";

/** Fields the page edits; the numeric ones hold strings (typed text) or numbers (loaded values). */
export interface ParametersFormFields {
  amount: string | number;
  waveSize: string | number;
  derechosPercent: string | number;
  derechosMinSample: string | number;
  cuvPercent: string | number;
  cuvMinSample: string | number;
  selected: string[];
}

/**
 * State and actions of the Parameters page: loads GET /api/parameters when the host connects and saves
 * with PUT /api/parameters. Port of `useParametersForm`; the host re-renders after every change and
 * answers arriving after it disconnected are ignored.
 */
export class ParametersFormController implements ReactiveController, ParametersFormFields {
  amount: string | number = "";
  waveSize: string | number = DEFAULT_WAVE_SIZE;
  derechosPercent: string | number = DEFAULT_DERECHOS_UNAVAILABLE_PERCENT;
  derechosMinSample: string | number = DEFAULT_DERECHOS_MIN_SAMPLE;
  cuvPercent: string | number = DEFAULT_CUV_TOKEN_FAILURE_PERCENT;
  cuvMinSample: string | number = DEFAULT_CUV_MIN_SAMPLE;
  options: string[] = [];
  selected: string[] = [];
  configured = false;
  loading = true;
  saving = false;
  error = "";
  message = "";

  private active = false;

  constructor(
    private host: ReactiveControllerHost,
    private api: Pick<TessiaApi, "parameters" | "saveParameters">,
  ) {
    host.addController(this);
  }

  hostConnected(): void {
    this.active = true;
    void this.load();
  }

  hostDisconnected(): void {
    this.active = false;
  }

  /** Sets one edited field and re-renders the host. */
  set<K extends keyof ParametersFormFields>(field: K, value: ParametersFormFields[K]): void {
    (this as ParametersFormFields)[field] = value;
    this.host.requestUpdate();
  }

  private apply(settings: PreparationParametersResponse): void {
    this.amount = settings.max_invoice_amount || "";
    this.waveSize = settings.wave_size ?? DEFAULT_WAVE_SIZE;
    this.derechosPercent = settings.derechos_unavailable_percent ?? DEFAULT_DERECHOS_UNAVAILABLE_PERCENT;
    this.derechosMinSample = settings.derechos_min_sample ?? DEFAULT_DERECHOS_MIN_SAMPLE;
    this.cuvPercent = settings.cuv_token_failure_percent ?? DEFAULT_CUV_TOKEN_FAILURE_PERCENT;
    this.cuvMinSample = settings.cuv_min_sample ?? DEFAULT_CUV_MIN_SAMPLE;
    const saved = settings.automatic_subplans || [];
    this.options = subplanOptions(saved);
    this.selected = selectedSubplans(this.options, saved);
    this.configured = Boolean(settings.configured);
  }

  private async load(): Promise<void> {
    try {
      const settings = await this.api.parameters();
      if (!this.active) return;
      this.apply(settings);
      this.error = "";
    } catch (err) {
      if (this.active) this.error = errorMessage(err);
    } finally {
      if (this.active) {
        this.loading = false;
        this.host.requestUpdate();
      }
    }
  }

  async save(): Promise<void> {
    this.error = "";
    this.message = "";
    const validationError = validateParameterForm(
      this.amount,
      this.selected,
      this.waveSize,
      this.derechosPercent,
      this.derechosMinSample,
      this.cuvPercent,
      this.cuvMinSample,
    );
    if (validationError) {
      this.error = validationError;
      this.host.requestUpdate();
      return;
    }
    this.saving = true;
    this.host.requestUpdate();
    try {
      const settings = await this.api.saveParameters({
        max_invoice_amount: String(this.amount).trim(),
        automatic_subplans: this.options.filter((option) => this.selected.includes(option)),
        wave_size: Number(this.waveSize),
        derechos_unavailable_percent: Number(this.derechosPercent),
        derechos_min_sample: Number(this.derechosMinSample),
        cuv_token_failure_percent: Number(this.cuvPercent),
        cuv_min_sample: Number(this.cuvMinSample),
      });
      this.configured = Boolean(settings.configured);
      this.message = "Global preparation parameters saved.";
    } catch (err) {
      this.error = errorMessage(err);
    } finally {
      this.saving = false;
      this.host.requestUpdate();
    }
  }
}
