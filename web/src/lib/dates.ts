// Calendar dates as the API carries them: `YYYY-MM-DD` strings, no time, no zone.
//
// Arithmetic goes through a day number (days since 1970-01-01, via Date.UTC) so
// that no local-time-zone or DST quirk can shift a date. The QBD-specific parts
// (date-field keys, report date macros) are ports of `crates/lb-core/src/period.rs`
// and are tested against the same cases.

import type { DateMacro } from '~/api/generated/DateMacro';
import type { DateRange } from '~/api/generated/DateRange';

export type IsoDate = string;

interface Ymd {
  y: number;
  m: number; // 1-12
  d: number; // 1-31
}

const MS_PER_DAY = 86_400_000;

export function isIsoDate(s: unknown): s is IsoDate {
  if (typeof s !== 'string' || !/^\d{4}-\d{2}-\d{2}$/.test(s)) return false;
  const { y, m, d } = split(s);
  return m >= 1 && m <= 12 && d >= 1 && d <= daysInMonth(y, m);
}

function split(iso: IsoDate): Ymd {
  return { y: Number(iso.slice(0, 4)), m: Number(iso.slice(5, 7)), d: Number(iso.slice(8, 10)) };
}

export function ymd(y: number, m: number, d: number): IsoDate {
  return `${String(y).padStart(4, '0')}-${String(m).padStart(2, '0')}-${String(d).padStart(2, '0')}`;
}

export function daysInMonth(y: number, m: number): number {
  return new Date(Date.UTC(y, m, 0)).getUTCDate();
}

function toDayNumber(iso: IsoDate): number {
  const { y, m, d } = split(iso);
  return Math.round(Date.UTC(y, m - 1, d) / MS_PER_DAY);
}

function fromDayNumber(n: number): IsoDate {
  const dt = new Date(n * MS_PER_DAY);
  return ymd(dt.getUTCFullYear(), dt.getUTCMonth() + 1, dt.getUTCDate());
}

/** Today in the browser's local zone (the user's "today", as QBD's `T` key). */
export function todayIso(now: Date = new Date()): IsoDate {
  return ymd(now.getFullYear(), now.getMonth() + 1, now.getDate());
}

export function addDays(iso: IsoDate, n: number): IsoDate {
  return fromDayNumber(toDayNumber(iso) + n);
}

/** Add months, clamping the day to the target month's end (as jiff does). */
export function addMonths(iso: IsoDate, n: number): IsoDate {
  const { y, m, d } = split(iso);
  const total = y * 12 + (m - 1) + n;
  const ny = Math.floor(total / 12);
  const nm = (total % 12) + 1;
  return ymd(ny, nm, Math.min(d, daysInMonth(ny, nm)));
}

export function addYears(iso: IsoDate, n: number): IsoDate {
  return addMonths(iso, n * 12);
}

export function daysBetween(from: IsoDate, to: IsoDate): number {
  return toDayNumber(to) - toDayNumber(from);
}

/** 0 = Sunday. */
export function weekday(iso: IsoDate): number {
  return new Date(toDayNumber(iso) * MS_PER_DAY).getUTCDay();
}

/** Sunday on or before `d` (US QBD weeks start Sunday). */
export function weekStart(iso: IsoDate): IsoDate {
  return addDays(iso, -weekday(iso));
}

export function firstOfMonth(iso: IsoDate): IsoDate {
  const { y, m } = split(iso);
  return ymd(y, m, 1);
}

export function lastOfMonth(iso: IsoDate): IsoDate {
  const { y, m } = split(iso);
  return ymd(y, m, daysInMonth(y, m));
}

export function minDate(a: IsoDate, b: IsoDate): IsoDate {
  return a < b ? a : b;
}

/**
 * QBD's date-field keys: `+`/`-` a day, `T` today, `W`/`K` start/end of week,
 * `M`/`H` start/end of month, `Y`/`R` start/end of year, `[`/`]` a week back or
 * forward. Null for keys that are not shortcuts. Port of `period::date_key`.
 */
export function dateKey(current: IsoDate, today: IsoDate, key: string): IsoDate | null {
  const { y } = split(current);
  switch (key.toLowerCase()) {
    case '+':
    case '=':
      return addDays(current, 1);
    case '-':
    case '_':
      return addDays(current, -1);
    case 't':
      return today;
    case 'w':
      return weekStart(current);
    case 'k':
      return addDays(weekStart(current), 6);
    case 'm':
      return firstOfMonth(current);
    case 'h':
      return lastOfMonth(current);
    case 'y':
      return ymd(y, 1, 1);
    case 'r':
      return ymd(y, 12, 31);
    case '[':
      return addDays(current, -7);
    case ']':
      return addDays(current, 7);
    default:
      return null;
  }
}

/** The keys `dateKey` understands, for the shortcut overlay. */
export const DATE_KEYS: ReadonlyArray<[string, string]> = [
  ['+ / -', 'Next / previous day'],
  ['T', 'Today'],
  ['W / K', 'First / last day of the week'],
  ['M / H', 'First / last day of the month'],
  ['Y / R', 'First / last day of the year'],
  ['[ / ]', 'Back / forward a week'],
];

/** First and last day of the fiscal year containing `d`, shifted by `offset` years. */
export function fiscalYear(iso: IsoDate, startMonth: number, offset: number): [IsoDate, IsoDate] {
  const sm = Math.min(12, Math.max(1, startMonth));
  const { y, m } = split(iso);
  const year = (m >= sm ? y : y - 1) + offset;
  const from = ymd(year, sm, 1);
  return [from, addDays(addYears(from, 1), -1)];
}

/** First and last day of the fiscal quarter containing `d`, shifted by `offset`. */
export function fiscalQuarter(iso: IsoDate, startMonth: number, offset: number): [IsoDate, IsoDate] {
  const [fyStart] = fiscalYear(iso, startMonth, 0);
  const a = split(iso);
  const b = split(fyStart);
  const monthsIn = (a.y - b.y) * 12 + a.m - b.m;
  const q = Math.floor(monthsIn / 3) + offset;
  const from = addMonths(fyStart, q * 3);
  return [from, addDays(addMonths(from, 3), -1)];
}

/** The range a report date macro means, for `today`. Port of `DateMacro::range`. */
export function macroRange(macro: DateMacro, today: IsoDate, fiscalStartMonth = 1): DateRange {
  const r = ([from, to]: [IsoDate, IsoDate]): DateRange => ({ from, to });
  const toDate = ([from]: [IsoDate, IsoDate]): DateRange => ({ from, to: today });
  const week = (offset: number): [IsoDate, IsoDate] => {
    const start = addDays(weekStart(today), offset * 7);
    return [start, addDays(start, 6)];
  };
  const month = (offset: number): [IsoDate, IsoDate] => {
    const start = addMonths(firstOfMonth(today), offset);
    return [start, lastOfMonth(start)];
  };
  // "Last X to date": from the start of last X to the same point within it.
  const lastToDate = ([from, to]: [IsoDate, IsoDate], thisFrom: IsoDate): DateRange => {
    const elapsed = daysBetween(thisFrom, today);
    return { from, to: minDate(addDays(from, elapsed), to) };
  };
  const fy = (o: number) => fiscalYear(today, fiscalStartMonth, o);
  const fq = (o: number) => fiscalQuarter(today, fiscalStartMonth, o);
  const cq = (o: number) => fiscalQuarter(today, 1, o);
  switch (macro) {
    case 'All':
      return { from: null, to: null };
    case 'Today':
      return { from: today, to: today };
    case 'Yesterday': {
      const y = addDays(today, -1);
      return { from: y, to: y };
    }
    case 'ThisWeek':
      return r(week(0));
    case 'ThisWeekToDate':
      return toDate(week(0));
    case 'LastWeek':
      return r(week(-1));
    case 'LastWeekToDate':
      return lastToDate(week(-1), week(0)[0]);
    case 'NextWeek':
      return r(week(1));
    case 'NextFourWeeks':
      return { from: week(1)[0], to: week(4)[1] };
    case 'ThisMonth':
      return r(month(0));
    case 'ThisMonthToDate':
      return toDate(month(0));
    case 'LastMonth':
      return r(month(-1));
    case 'LastMonthToDate':
      return lastToDate(month(-1), month(0)[0]);
    case 'NextMonth':
      return r(month(1));
    case 'ThisFiscalQuarter':
      return r(fq(0));
    case 'ThisFiscalQuarterToDate':
      return toDate(fq(0));
    case 'LastFiscalQuarter':
      return r(fq(-1));
    case 'LastFiscalQuarterToDate':
      return lastToDate(fq(-1), fq(0)[0]);
    case 'NextFiscalQuarter':
      return r(fq(1));
    case 'ThisFiscalYear':
      return r(fy(0));
    case 'ThisFiscalYearToDate':
      return toDate(fy(0));
    case 'LastFiscalYear':
      return r(fy(-1));
    case 'LastFiscalYearToDate':
      return lastToDate(fy(-1), fy(0)[0]);
    case 'NextFiscalYear':
      return r(fy(1));
    case 'ThisCalendarQuarter':
      return r(cq(0));
    case 'ThisCalendarQuarterToDate':
      return toDate(cq(0));
    case 'LastCalendarQuarter':
      return r(cq(-1));
    case 'ThisCalendarYear':
      return r(fiscalYear(today, 1, 0));
    case 'ThisCalendarYearToDate':
      return toDate(fiscalYear(today, 1, 0));
    case 'LastCalendarYear':
      return r(fiscalYear(today, 1, -1));
  }
}

/** Labels for the report "Dates" picker, in QBD's order. */
export const MACRO_LABELS: ReadonlyArray<[DateMacro, string]> = [
  ['All', 'All'],
  ['Today', 'Today'],
  ['Yesterday', 'Yesterday'],
  ['ThisWeek', 'This Week'],
  ['ThisWeekToDate', 'This Week-to-date'],
  ['LastWeek', 'Last Week'],
  ['LastWeekToDate', 'Last Week-to-date'],
  ['NextWeek', 'Next Week'],
  ['NextFourWeeks', 'Next 4 Weeks'],
  ['ThisMonth', 'This Month'],
  ['ThisMonthToDate', 'This Month-to-date'],
  ['LastMonth', 'Last Month'],
  ['LastMonthToDate', 'Last Month-to-date'],
  ['NextMonth', 'Next Month'],
  ['ThisFiscalQuarter', 'This Fiscal Quarter'],
  ['ThisFiscalQuarterToDate', 'This Fiscal Quarter-to-date'],
  ['LastFiscalQuarter', 'Last Fiscal Quarter'],
  ['LastFiscalQuarterToDate', 'Last Fiscal Quarter-to-date'],
  ['NextFiscalQuarter', 'Next Fiscal Quarter'],
  ['ThisFiscalYear', 'This Fiscal Year'],
  ['ThisFiscalYearToDate', 'This Fiscal Year-to-date'],
  ['LastFiscalYear', 'Last Fiscal Year'],
  ['LastFiscalYearToDate', 'Last Fiscal Year-to-date'],
  ['NextFiscalYear', 'Next Fiscal Year'],
  ['ThisCalendarQuarter', 'This Calendar Quarter'],
  ['ThisCalendarQuarterToDate', 'This Calendar Quarter-to-date'],
  ['LastCalendarQuarter', 'Last Calendar Quarter'],
  ['ThisCalendarYear', 'This Calendar Year'],
  ['ThisCalendarYearToDate', 'This Calendar Year-to-date'],
  ['LastCalendarYear', 'Last Calendar Year'],
];

/** Pick the century for a two-digit year: the one putting it within 50 years of today. */
function expandYear(yy: number, todayYear: number): number {
  const century = Math.floor(todayYear / 100) * 100;
  let y = century + yy;
  if (y > todayYear + 50) y -= 100;
  else if (y < todayYear - 50) y += 100;
  return y;
}

function valid(y: number, m: number, d: number): IsoDate | null {
  if (m < 1 || m > 12 || d < 1 || d > daysInMonth(y, m)) return null;
  return ymd(y, m, d);
}

/**
 * Parse what people type in a date field (US order, as QBD):
 * `10/19`, `10/19/26`, `10/19/2026`, `10-19-26`, `10.19.26`, `1019`, `101926`,
 * `10192026`, a lone day `19` (this month), ISO `2026-10-19`, and `t`/`today`.
 * Missing parts come from `today`. Null if it isn't a date.
 */
export function parseTypedDate(text: string, today: IsoDate): IsoDate | null {
  const s = text.trim().toLowerCase();
  if (s === '') return null;
  if (s === 't' || s === 'today') return today;
  const t = split(today);
  if (/^\d{4}-\d{1,2}-\d{1,2}$/.test(s)) {
    const [y, m, d] = s.split('-').map(Number) as [number, number, number];
    return valid(y, m, d);
  }
  const parts = s.split(/[/\-. ]+/).filter((p) => p !== '');
  if (parts.some((p) => !/^\d+$/.test(p))) return null;
  const yearOf = (p: string | undefined): number | null => {
    if (p === undefined) return t.y;
    if (p.length <= 2) return expandYear(Number(p), t.y);
    if (p.length === 4) return Number(p);
    return null;
  };
  if (parts.length >= 2 && parts.length <= 3) {
    const y = yearOf(parts[2]);
    if (y === null) return null;
    return valid(y, Number(parts[0]), Number(parts[1]));
  }
  if (parts.length !== 1) return null;
  const digits = parts[0]!;
  switch (digits.length) {
    case 1:
    case 2:
      return valid(t.y, t.m, Number(digits));
    case 3: // MDD
      return valid(t.y, Number(digits.slice(0, 1)), Number(digits.slice(1)));
    case 4: // MMDD
      return valid(t.y, Number(digits.slice(0, 2)), Number(digits.slice(2)));
    case 6: // MMDDYY
      return valid(expandYear(Number(digits.slice(4)), t.y), Number(digits.slice(0, 2)), Number(digits.slice(2, 4)));
    case 8: // MMDDYYYY
      return valid(Number(digits.slice(4)), Number(digits.slice(0, 2)), Number(digits.slice(2, 4)));
    default:
      return null;
  }
}

/** `2026-10-19` → `10/19/2026`, QBD's display. */
export function formatDate(iso: IsoDate | null | undefined): string {
  if (!iso) return '';
  const { y, m, d } = split(iso);
  return `${String(m).padStart(2, '0')}/${String(d).padStart(2, '0')}/${y}`;
}

/** `2026-10-19` → `10/19/26`, for narrow columns. */
export function formatDateShort(iso: IsoDate | null | undefined): string {
  if (!iso) return '';
  const { y, m, d } = split(iso);
  return `${String(m).padStart(2, '0')}/${String(d).padStart(2, '0')}/${String(y % 100).padStart(2, '0')}`;
}

/** Local-midnight Date for a calendar widget (which works in local time). */
export function isoToLocalDate(iso: IsoDate): Date {
  const { y, m, d } = split(iso);
  return new Date(y, m - 1, d);
}

/**
 * QBD's "warn if a date is more than N days in the past / future" preference.
 * Returns a warning sentence or null.
 */
export function dateWarning(
  iso: IsoDate,
  today: IsoDate,
  warnDaysPast: number | null | undefined,
  warnDaysFuture: number | null | undefined,
): string | null {
  const diff = daysBetween(today, iso);
  if (warnDaysPast != null && diff < -warnDaysPast) {
    return `This date is more than ${warnDaysPast} days in the past.`;
  }
  if (warnDaysFuture != null && diff > warnDaysFuture) {
    return `This date is more than ${warnDaysFuture} days in the future.`;
  }
  return null;
}
