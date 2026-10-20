// Phone layout: a header with back and search, the active window full screen,
// and a bottom tab bar with a big "+" for creating things. Windows form a stack:
// back closes the current one and returns to the one before (asking first if it
// has unsaved changes).

import { ActionIcon, Drawer, Group, Stack, Text, UnstyledButton, Button } from '@mantine/core';
import { spotlight } from '@mantine/spotlight';
import {
  IconArrowLeft,
  IconBuildingBank,
  IconCash,
  IconChecks,
  IconCreditCard,
  IconDots,
  IconFileInvoice,
  IconHome,
  IconPlus,
  IconReceipt,
  IconSearch,
  IconTruck,
  IconUserPlus,
  IconUsers,
} from '@tabler/icons-react';
import { useState, type ReactNode } from 'react';
import type { LiveStatus } from '~/api/context';
import { useQuickCreate } from '~/inputs/QuickCreate';
import { windowTitle } from '~/windows/catalog';
import { useWindows } from '~/windows/WindowsProvider';
import { paths } from '~/windows/paths';
import { FixtureBadge, LiveIndicator, NAV_ICONS, ThemeToggle } from './chrome';
import { NAV_ITEMS } from './menus';
import { useSignOut } from './UserMenu';
import { WindowHost } from './WindowHost';
import classes from './shell.module.css';

function SheetItem({ icon, label, onClick, todo }: { icon: ReactNode; label: string; onClick(): void; todo?: boolean }) {
  return (
    <UnstyledButton className={classes.sheetItem} onClick={onClick} data-todo={todo || undefined}>
      {icon}
      {label}
    </UnstyledButton>
  );
}

export function MobileShell({ live, onShortcuts }: { live: LiveStatus; onShortcuts(): void }) {
  const windows = useWindows();
  const { state, open } = windows;
  const qc = useQuickCreate();
  const [plus, setPlus] = useState(false);
  const [more, setMore] = useState(false);
  const signOut = useSignOut();
  const active = state.windows.find((w) => w.key === state.active);
  const title = active ? windowTitle(active, state.titles) : 'LibreBooks';
  const go = (p: string) => {
    setPlus(false);
    setMore(false);
    open(p);
  };
  const tab = (kinds: readonly string[]) => (active && kinds.includes(active.kind)) || undefined;

  return (
    <div className={classes.mobile}>
      <header className={classes.mheader}>
        {active && active.kind !== 'home' ? (
          <ActionIcon variant="subtle" size="xl" aria-label="Back" onClick={() => windows.back(active.key)}>
            <IconArrowLeft size={22} />
          </ActionIcon>
        ) : (
          <span style={{ width: 8 }} />
        )}
        <span className={classes.mtitle}>{title}</span>
        <FixtureBadge />
        <LiveIndicator status={live} />
        <ActionIcon variant="subtle" size="xl" aria-label="Search" onClick={() => spotlight.open()}>
          <IconSearch size={22} />
        </ActionIcon>
      </header>
      <main className={classes.mmain}>
        <WindowHost />
      </main>
      <nav className={classes.tabbar} aria-label="Main">
        <UnstyledButton className={classes.tabbarItem} data-active={tab(['home'])} onClick={() => go(paths.home())}>
          <IconHome size={24} stroke={1.6} />
          Home
        </UnstyledButton>
        <UnstyledButton className={classes.tabbarItem} data-active={tab(['customers'])} onClick={() => go(paths.customers())}>
          <IconUsers size={24} stroke={1.6} />
          Customers
        </UnstyledButton>
        <ActionIcon className={classes.plus} size={54} radius="xl" aria-label="Create" onClick={() => setPlus(true)}>
          <IconPlus size={28} />
        </ActionIcon>
        <UnstyledButton className={classes.tabbarItem} data-active={tab(['banking', 'register', 'check'])} onClick={() => go(paths.banking())}>
          <IconBuildingBank size={24} stroke={1.6} />
          Banking
        </UnstyledButton>
        <UnstyledButton className={classes.tabbarItem} data-active={tab(['vendors', 'employees', 'reports', 'report', 'accounts', 'list', 'company', 'find'])} onClick={() => setMore(true)}>
          <IconDots size={24} stroke={1.6} />
          More
        </UnstyledButton>
      </nav>

      <Drawer opened={plus} onClose={() => setPlus(false)} position="bottom" size="auto" title="Create" radius="md">
        <div className={classes.sheetGrid}>
          <SheetItem icon={<IconChecks size={26} />} label="Write Check" onClick={() => go(paths.newCheck())} />
          <SheetItem icon={<IconFileInvoice size={26} />} label="Invoice" onClick={() => go(paths.form('invoice'))} />
          <SheetItem icon={<IconCash size={26} />} label="Receive Payment" onClick={() => go(paths.form('receive-payment'))} todo />
          <SheetItem icon={<IconReceipt size={26} />} label="Enter Bill" onClick={() => go(paths.form('bill'))} />
          <SheetItem icon={<IconBuildingBank size={26} />} label="Deposit" onClick={() => go(paths.form('deposit'))} todo />
          <SheetItem icon={<IconCreditCard size={26} />} label="Card Charge" onClick={() => go(paths.form('cc-charge'))} />
          <SheetItem
            icon={<IconUserPlus size={26} />}
            label="New Customer"
            onClick={() => {
              setPlus(false);
              void qc.createEntity('', ['Customer']);
            }}
          />
          <SheetItem
            icon={<IconTruck size={26} />}
            label="New Vendor"
            onClick={() => {
              setPlus(false);
              void qc.createEntity('', ['Vendor']);
            }}
          />
          <SheetItem
            icon={<IconPlus size={26} />}
            label="New Account"
            onClick={() => {
              setPlus(false);
              void qc.createAccount('');
            }}
          />
        </div>
      </Drawer>

      <Drawer opened={more} onClose={() => setMore(false)} position="bottom" size="auto" title="More" radius="md">
        <Stack gap="md">
          <div className={classes.sheetGrid}>
            {NAV_ITEMS.filter((n) => !['Home', 'Customers', 'Banking'].includes(n.label)).map((n) => (
              <SheetItem key={n.label} icon={NAV_ICONS[n.icon]!(26)} label={n.label === 'Lists' ? 'Accounts' : n.label} onClick={() => go(n.path)} />
            ))}
            <SheetItem icon={<IconSearch size={26} />} label="Find" onClick={() => go(paths.find())} />
          </div>
          {state.windows.length > 1 && (
            <div>
              <Text size="sm" fw={600} mb={4}>
                Open windows
              </Text>
              <Stack gap={2}>
                {state.windows.map((w) => (
                  <Button
                    key={w.key}
                    variant={w.key === state.active ? 'light' : 'subtle'}
                    justify="space-between"
                    onClick={() => {
                      setMore(false);
                      windows.activate(w.key);
                    }}
                    rightSection={state.dirty[w.key] ? <span className={classes.dirtyDot} /> : null}
                  >
                    {windowTitle(w, state.titles)}
                  </Button>
                ))}
              </Stack>
            </div>
          )}
          <Group justify="space-between">
            <Button
              variant="subtle"
              onClick={() => {
                setMore(false);
                onShortcuts();
              }}
            >
              Keyboard shortcuts
            </Button>
            <Group gap="xs">
              {signOut && (
                <Button variant="subtle" color="gray" onClick={signOut}>
                  Sign out
                </Button>
              )}
              <ThemeToggle />
            </Group>
          </Group>
        </Stack>
      </Drawer>
    </div>
  );
}
