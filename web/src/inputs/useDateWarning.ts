// QBD's "Warn if transactions are N days in the past / future" preference,
// as a hook for DateField's `warning`.

import { useCompanyInfo } from '~/api/hooks';
import { dateWarning, todayIso, type IsoDate } from '~/lib/dates';

export function useDateWarning(date: IsoDate | null): string | null {
  const { data: info } = useCompanyInfo();
  if (!date || !info) return null;
  return dateWarning(date, todayIso(), info.preferences.warn_days_past, info.preferences.warn_days_future);
}
