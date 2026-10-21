// The company preferences the menus depend on (a form whose feature is off is
// hidden, as QBD hides it), as one stable object so the menus memoize well.

import { useMemo } from 'react';
import { useCompanyInfo } from '~/api/hooks';
import type { MenuPrefs } from './menus';

export function useMenuPrefs(): MenuPrefs {
  const { data: info } = useCompanyInfo();
  const p = info?.preferences;
  return useMemo(() => (p ? { sales_orders_enabled: p.sales_orders_enabled, estimates_enabled: p.estimates_enabled } : null), [p?.sales_orders_enabled, p?.estimates_enabled]); // eslint-disable-line react-hooks/exhaustive-deps
}
