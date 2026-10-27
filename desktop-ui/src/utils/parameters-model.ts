// Pure helpers for the "Parameters" form. Port of webui-src/src/parametersModel.js.

// Must stay equal to DefaultAutomaticSubplans in tessia/internal/convenio/prepare.go.
export const DEFAULT_AUTOMATIC_SUBPLANS: readonly string[] = [
  "EVENTO POS",
  "EVENTO NO PBS",
  "TRANSFERENCIA DE PENDIENTES CON AUTORIZACION",
  "EVENTO SAS SIN AUTORIZACION",
];

// Must stay equal to convenio.DefaultWaveSize / convenio.MaxWaveSize.
export const DEFAULT_WAVE_SIZE = 20;
export const MAX_WAVE_SIZE = 500;

// wave_size is still stored and sent, but the pipeline no longer gates admission with it
// (rolling semaphore: every phase starts at its worker ceiling).
export const WAVE_SIZE_LABEL = "Wave size (proceso run) - heredado";
export const WAVE_SIZE_HINT =
  "Heredado: ya no limita la admisión de facturas; cada fase arranca en su tope de workers. El valor se sigue guardando.";

// Must stay equal to convenio.DefaultDerechosUnavailablePercent /
// DefaultDerechosMinSample / MaxDerechosMinSample.
export const DEFAULT_DERECHOS_UNAVAILABLE_PERCENT = 50;
export const DEFAULT_DERECHOS_MIN_SAMPLE = 4;
export const MAX_DERECHOS_MIN_SAMPLE = 1000;

// Must stay equal to convenio.DefaultCUVTokenFailurePercent /
// DefaultCUVMinSample / MaxCUVMinSample.
export const DEFAULT_CUV_TOKEN_FAILURE_PERCENT = 50;
export const DEFAULT_CUV_MIN_SAMPLE = 4;
export const MAX_CUV_MIN_SAMPLE = 1000;

/** A form field: text inputs hold strings, type="number" inputs may hold numbers. */
type FieldValue = string | number | null | undefined;

function normalizeSubplan(value: unknown): string {
  return String(value || "").normalize("NFD").replace(/\p{Diacritic}/gu, "").toUpperCase().replace(/\s+/g, " ").trim();
}

/** Defaults first, then any saved subplan not already present (saved spelling kept). */
export function subplanOptions(saved: readonly string[] | null | undefined): string[] {
  const options = [...DEFAULT_AUTOMATIC_SUBPLANS];
  const keys = new Set(options.map(normalizeSubplan));
  for (const entry of saved || []) {
    const key = normalizeSubplan(entry);
    if (!key || keys.has(key)) continue;
    keys.add(key);
    options.push(String(entry).trim());
  }
  return options;
}

/** Options (in option order) whose normalized key matches a saved entry. */
export function selectedSubplans(options: readonly string[], saved: readonly string[] | null | undefined): string[] {
  const savedKeys = new Set((saved || []).map(normalizeSubplan));
  return options.filter((option) => savedKeys.has(normalizeSubplan(option)));
}

function isWholeNumberAtMost(value: FieldValue, max: number): boolean {
  return /^[0-9]+$/.test(String(value).trim()) && Number(value) <= max;
}

/** First validation message (English, as in the old UI), or null when the form is valid. */
export function validateParameterForm(
  amount: FieldValue,
  selected: readonly string[] | null | undefined,
  waveSize: FieldValue = DEFAULT_WAVE_SIZE,
  derechosPercent: FieldValue = DEFAULT_DERECHOS_UNAVAILABLE_PERCENT,
  derechosMinSample: FieldValue = DEFAULT_DERECHOS_MIN_SAMPLE,
  cuvPercent: FieldValue = DEFAULT_CUV_TOKEN_FAILURE_PERCENT,
  cuvMinSample: FieldValue = DEFAULT_CUV_MIN_SAMPLE,
): string | null {
  const cap = String(amount || "").trim();
  if (!/^[0-9]{1,12}(?:\.[0-9]{1,2})?$/.test(cap) || Number(cap) <= 0) {
    return "Enter a positive maximum amount using up to two decimals.";
  }
  if (!selected || selected.length === 0) return "Select at least one automatic subplan.";
  if (!isWholeNumberAtMost(waveSize, MAX_WAVE_SIZE)) {
    return `Enter a wave size between 0 (disabled) and ${MAX_WAVE_SIZE}.`;
  }
  if (!isWholeNumberAtMost(derechosPercent, 100)) {
    return "Enter a derechos unavailable percent between 0 (disabled) and 100.";
  }
  if (!isWholeNumberAtMost(derechosMinSample, MAX_DERECHOS_MIN_SAMPLE)) {
    return `Enter a derechos minimum sample between 0 and ${MAX_DERECHOS_MIN_SAMPLE}.`;
  }
  if (!isWholeNumberAtMost(cuvPercent, 100)) {
    return "Enter a CUV token failure percent between 0 (disabled) and 100.";
  }
  if (!isWholeNumberAtMost(cuvMinSample, MAX_CUV_MIN_SAMPLE)) {
    return `Enter a CUV minimum sample between 0 and ${MAX_CUV_MIN_SAMPLE}.`;
  }
  return null;
}
