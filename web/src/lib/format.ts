// Small pure formatters the administrative screens share: byte sizes, and the
// ISO 8601 durations the server's schedules are written in (`PT6H`, `P1D`).

/** Bytes as people read them. */
export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(0)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

/**
 * An ISO 8601 duration → minutes, or null when it isn't one. Days, hours,
 * minutes and seconds only: that's all the schedules use (no months, which
 * have no fixed length).
 */
export function durationToMinutes(iso: string | null | undefined): number | null {
  if (!iso) return null;
  const m = /^P(?:(\d+)D)?(?:T(?:(\d+)H)?(?:(\d+)M)?(?:(\d+)S)?)?$/.exec(iso.trim());
  if (!m || iso.trim() === 'P' || iso.trim().endsWith('T')) return null;
  const [, d, h, min, s] = m;
  const total = Number(d ?? 0) * 1440 + Number(h ?? 0) * 60 + Number(min ?? 0) + Number(s ?? 0) / 60;
  return Number.isFinite(total) ? total : null;
}

/** Minutes → the shortest ISO 8601 duration for them (`P1D`, `PT6H`, `PT90M`). */
export function minutesToDuration(minutes: number): string {
  const m = Math.max(1, Math.round(minutes));
  if (m % 1440 === 0) return `P${m / 1440}D`;
  if (m % 60 === 0) return `PT${m / 60}H`;
  return `PT${m}M`;
}

/** `PT6H` → "every 6 hours"; something unparseable comes back as it was. */
export function describeDuration(iso: string | null | undefined): string {
  const m = durationToMinutes(iso);
  if (m === null) return iso ?? '';
  if (m % 1440 === 0) return m === 1440 ? 'every day' : `every ${m / 1440} days`;
  if (m % 60 === 0) return m === 60 ? 'every hour' : `every ${m / 60} hours`;
  return `every ${m} minutes`;
}
