import { useMediaQuery } from '@mantine/hooks';

/** Phone-sized layout (below Mantine's `sm`, 48em = 768px). Read synchronously so there's no desktop flash. */
export function useIsMobile(): boolean {
  return useMediaQuery('(max-width: 48em)', false, { getInitialValueInEffect: false });
}
