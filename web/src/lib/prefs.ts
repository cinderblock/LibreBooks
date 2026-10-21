// Per-device user preferences (QBD's "My Preferences"), kept in localStorage.

import { useLocalStorage } from '@mantine/hooks';
import type { ExportPaper } from '~/api/client';
import type { ExportFormat } from '~/api/types';
import type { EnterBehavior } from '~/inputs/LineGrid';

/**
 * What Enter does in a line grid. QBD's default is to save the transaction;
 * its "Enter key moves between fields" preference makes it act like Tab, which
 * is the faster default for keyboard entry here. Users can switch.
 */
export function useEnterBehavior() {
  return useLocalStorage<EnterBehavior>({ key: 'librebooks.enterBehavior', defaultValue: 'next-cell', getInitialValueInEffect: false });
}

/** The report export format Ctrl+E repeats: the last one picked from the menu (CSV to begin with). */
export function useExportFormat() {
  return useLocalStorage<ExportFormat>({ key: 'librebooks.reportExportFormat', defaultValue: 'Csv', getInitialValueInEffect: false });
}

/** The PDF export's paper (QBD prints on the printer's default; here it's remembered per device). */
export function useExportPaper() {
  return useLocalStorage<ExportPaper>({ key: 'librebooks.reportPaper', defaultValue: 'letter', getInitialValueInEffect: false });
}
