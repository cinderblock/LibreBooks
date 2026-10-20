// Per-device user preferences (QBD's "My Preferences"), kept in localStorage.

import { useLocalStorage } from '@mantine/hooks';
import type { EnterBehavior } from '~/inputs/LineGrid';

/**
 * What Enter does in a line grid. QBD's default is to save the transaction;
 * its "Enter key moves between fields" preference makes it act like Tab, which
 * is the faster default for keyboard entry here. Users can switch.
 */
export function useEnterBehavior() {
  return useLocalStorage<EnterBehavior>({ key: 'librebooks.enterBehavior', defaultValue: 'next-cell', getInitialValueInEffect: false });
}
