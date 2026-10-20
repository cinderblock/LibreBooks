// Small pieces of chrome shared by both shells.

import { ActionIcon, Badge, Tooltip, useComputedColorScheme, useMantineColorScheme } from '@mantine/core';
import {
  IconBuilding,
  IconBuildingBank,
  IconHome,
  IconId,
  IconListDetails,
  IconMoon,
  IconReportAnalytics,
  IconSun,
  IconTruck,
  IconUsers,
} from '@tabler/icons-react';
import type { ReactNode } from 'react';
import { useApi, type LiveStatus } from '~/api/context';
import classes from './shell.module.css';

export const NAV_ICONS: Record<string, (size: number) => ReactNode> = {
  home: (s) => <IconHome size={s} stroke={1.6} />,
  customers: (s) => <IconUsers size={s} stroke={1.6} />,
  vendors: (s) => <IconTruck size={s} stroke={1.6} />,
  employees: (s) => <IconId size={s} stroke={1.6} />,
  banking: (s) => <IconBuildingBank size={s} stroke={1.6} />,
  reports: (s) => <IconReportAnalytics size={s} stroke={1.6} />,
  lists: (s) => <IconListDetails size={s} stroke={1.6} />,
  company: (s) => <IconBuilding size={s} stroke={1.6} />,
};

export function ThemeToggle() {
  const { setColorScheme } = useMantineColorScheme();
  const computed = useComputedColorScheme('light');
  const dark = computed === 'dark';
  return (
    <Tooltip label={dark ? 'Light mode' : 'Dark mode'}>
      <ActionIcon variant="subtle" color="gray" onClick={() => setColorScheme(dark ? 'light' : 'dark')} aria-label="Toggle dark mode">
        {dark ? <IconSun size={18} /> : <IconMoon size={18} />}
      </ActionIcon>
    </Tooltip>
  );
}

const LIVE_LABEL: Record<LiveStatus, string> = {
  open: 'Live: changes by others appear as they happen',
  connecting: 'Connecting to live updates…',
  error: 'Live updates disconnected; retrying',
};

export function LiveIndicator({ status }: { status: LiveStatus }) {
  return (
    <Tooltip label={LIVE_LABEL[status]}>
      <span className={classes.live} data-status={status} role="status" aria-label={LIVE_LABEL[status]} />
    </Tooltip>
  );
}

export function FixtureBadge() {
  const api = useApi();
  if (api.mode !== 'fixtures') return null;
  return (
    <Tooltip label="Synthetic demo books held in memory; a reload starts over.">
      <Badge variant="light" color="orange" size="sm">
        Fixtures
      </Badge>
    </Tooltip>
  );
}
