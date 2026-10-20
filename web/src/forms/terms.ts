// Payment terms → dates, as QBD computes them. Pure.
//
// Standard terms count days from the transaction date ("Net 30", "2% 10 Net
// 30"). Date-driven terms name a day of the month: the bill is due on that day
// of the *next* month when the transaction falls within `cutoff_days` of it,
// else that day of this month.

import type { Terms } from '~/api/types';
import { addDays, addMonths, daysInMonth, type IsoDate } from '~/lib/dates';
import { percentOf, type Cents } from '~/lib/money';

function dayOfMonthOnOrAfter(date: IsoDate, day: number, cutoffDays: number): IsoDate {
  const y = Number(date.slice(0, 4));
  const m = Number(date.slice(5, 7));
  const d = Number(date.slice(8, 10));
  const clamp = (yy: number, mm: number) => `${String(yy).padStart(4, '0')}-${String(mm).padStart(2, '0')}-${String(Math.min(day, daysInMonth(yy, mm))).padStart(2, '0')}`;
  const thisMonth = clamp(y, m);
  // Within the cutoff of this month's day (or past it): next month.
  if (d + cutoffDays > Math.min(day, daysInMonth(y, m)) || thisMonth < date) return clamp(...splitYm(addMonths(`${date.slice(0, 7)}-01`, 1)));
  return thisMonth;
}

function splitYm(iso: IsoDate): [number, number] {
  return [Number(iso.slice(0, 4)), Number(iso.slice(5, 7))];
}

export function dueDate(terms: Terms | null | undefined, date: IsoDate): IsoDate | null {
  if (!terms) return null;
  const k = terms.kind;
  if (k.type === 'Standard') return addDays(date, k.due_days);
  return dayOfMonthOnOrAfter(date, k.day_of_month, k.cutoff_days);
}

/** The last day an early-payment discount applies, or null without one. */
export function discountDate(terms: Terms | null | undefined, date: IsoDate): IsoDate | null {
  if (!terms) return null;
  const k = terms.kind;
  if (k.type === 'Standard') return k.discount_days !== null && k.discount_percent ? addDays(date, k.discount_days) : null;
  return k.discount_day_of_month !== null && k.discount_percent ? dayOfMonthOnOrAfter(date, k.discount_day_of_month, k.cutoff_days) : null;
}

/** The discount the terms offer on `amount` when paid by `payDate`, else 0. */
export function termsDiscount(terms: Terms | null | undefined, txnDate: IsoDate, payDate: IsoDate, amount: Cents): Cents {
  if (!terms) return 0;
  const until = discountDate(terms, txnDate);
  if (!until || payDate > until) return 0;
  const pct = terms.kind.discount_percent;
  return pct ? (percentOf(amount, pct) ?? 0) : 0;
}
