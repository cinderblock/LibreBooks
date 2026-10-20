// Desktop layout: QBD's menu bar across the top, the icon bar down the left,
// and the open windows as tabs (QBD's Open Window List) over the active window.

import { CloseButton, Kbd, Menu, Menubar, Text, Tooltip, UnstyledButton } from '@mantine/core';
import { spotlight } from '@mantine/spotlight';
import { IconCheck, IconSearch } from '@tabler/icons-react';
import type { MouseEvent } from 'react';
import type { LiveStatus } from '~/api/context';
import { useComboHint, useShortcutCatalog } from '~/keyboard/KeyboardProvider';
import type { CommandId } from '~/keyboard/commands';
import { windowTitle } from '~/windows/catalog';
import { useWindows } from '~/windows/WindowsProvider';
import { FixtureBadge, LiveIndicator, NAV_ICONS, ThemeToggle } from './chrome';
import type { RunCommand } from './GlobalCommands';
import { MENUS, NAV_ITEMS, type MenuItem } from './menus';
import { UserMenu } from './UserMenu';
import { WindowHost } from './WindowHost';
import classes from './shell.module.css';

function Hint({ command }: { command?: CommandId }) {
  const hint = useComboHint(command ?? 'palette.open');
  return command && hint ? (
    <Text size="xs" c="dimmed">
      {hint}
    </Text>
  ) : null;
}

function Item({ item, run }: { item: MenuItem; run: RunCommand }) {
  const { open } = useWindows();
  return (
    <Menu.Item
      rightSection={<Hint command={item.command} />}
      c={item.todo ? 'dimmed' : undefined}
      onClick={() => (item.path ? open(item.path) : item.command ? run(item.command) : undefined)}
    >
      {item.label}
    </Menu.Item>
  );
}

function TopMenu({ run, onShortcuts }: { run: RunCommand; onShortcuts(): void }) {
  const windows = useWindows();
  const { state } = windows;
  const { format } = useShortcutCatalog();
  return (
    <Menubar trigger="click" aria-label="Main menu">
      {MENUS.map((m) => (
        <Menubar.Menu key={m.label}>
          <Menubar.Target>{m.label}</Menubar.Target>
          <Menubar.Dropdown>
            {m.entries.map((e, i) =>
              'divider' in e ? (
                <Menu.Divider key={`d${i}`} />
              ) : 'children' in e ? (
                <Menu.Sub key={e.label}>
                  <Menu.Sub.Target>
                    <Menu.Sub.Item>{e.label}</Menu.Sub.Item>
                  </Menu.Sub.Target>
                  <Menu.Sub.Dropdown>
                    {e.children.map((c) => (
                      <Item key={c.label} item={c} run={run} />
                    ))}
                  </Menu.Sub.Dropdown>
                </Menu.Sub>
              ) : (
                <Item key={e.label} item={e} run={run} />
              ),
            )}
          </Menubar.Dropdown>
        </Menubar.Menu>
      ))}
      <Menubar.Menu>
        <Menubar.Target>Window</Menubar.Target>
        <Menubar.Dropdown>
          <Menu.Item rightSection={<Hint command="window.close" />} disabled={!state.active} onClick={() => state.active && windows.close(state.active)}>
            Close Window
          </Menu.Item>
          <Menu.Item disabled={state.windows.length === 0} onClick={() => state.windows.forEach((w) => windows.close(w.key))}>
            Close All
          </Menu.Item>
          <Menu.Item rightSection={<Hint command="window.next" />} onClick={() => windows.cycle(1)}>
            Next Window
          </Menu.Item>
          <Menu.Item rightSection={<Hint command="window.prev" />} onClick={() => windows.cycle(-1)}>
            Previous Window
          </Menu.Item>
          <Menu.Divider />
          <Menu.Label>Open Window List</Menu.Label>
          {state.windows.map((w) => (
            <Menu.Item key={w.key} leftSection={w.key === state.active ? <IconCheck size={14} /> : <span style={{ width: 14 }} />} onClick={() => windows.activate(w.key)}>
              {windowTitle(w, state.titles)}
              {state.dirty[w.key] ? ' •' : ''}
            </Menu.Item>
          ))}
        </Menubar.Dropdown>
      </Menubar.Menu>
      <Menubar.Menu>
        <Menubar.Target>Help</Menubar.Target>
        <Menubar.Dropdown>
          <Menu.Item rightSection={<Text size="xs" c="dimmed">{format('help.shortcuts').join(' / ')}</Text>} onClick={onShortcuts}>
            Keyboard Shortcuts
          </Menu.Item>
          <Menu.Item onClick={() => windows.open('/company')}>Preferences (keymap, Enter key)</Menu.Item>
          <Menu.Divider />
          <Menu.Label>LibreBooks · MIT licensed</Menu.Label>
        </Menubar.Dropdown>
      </Menubar.Menu>
    </Menubar>
  );
}

function NavRail() {
  const { state, open } = useWindows();
  const active = state.windows.find((w) => w.key === state.active);
  return (
    <nav className={classes.rail} aria-label="Navigation">
      {NAV_ITEMS.map((n) => (
        <UnstyledButton
          key={n.label}
          className={classes.railItem}
          data-active={(active && (n.kinds as readonly string[]).includes(active.kind)) || undefined}
          onClick={() => open(n.path)}
        >
          {NAV_ICONS[n.icon]!(22)}
          {n.label}
        </UnstyledButton>
      ))}
    </nav>
  );
}

function WindowTabs() {
  const windows = useWindows();
  const { state } = windows;
  return (
    <div className={classes.tabs} role="tablist" aria-label="Open windows">
      {state.windows.map((w) => {
        const title = windowTitle(w, state.titles);
        const active = w.key === state.active;
        return (
          <div
            key={w.key}
            role="tab"
            aria-selected={active}
            tabIndex={-1}
            className={classes.tab}
            data-active={active || undefined}
            title={title}
            onClick={() => windows.activate(w.key)}
            onAuxClick={(e: MouseEvent) => {
              if (e.button === 1) windows.close(w.key);
            }}
          >
            {state.dirty[w.key] && <span className={classes.dirtyDot} aria-label="Unsaved changes" />}
            <span className={classes.tabTitle}>{title}</span>
            <CloseButton
              size="xs"
              aria-label={`Close ${title}`}
              onClick={(e) => {
                e.stopPropagation();
                windows.close(w.key);
              }}
            />
          </div>
        );
      })}
    </div>
  );
}

export function DesktopShell({ live, run, onShortcuts }: { live: LiveStatus; run: RunCommand; onShortcuts(): void }) {
  const paletteHint = useComboHint('palette.open');
  return (
    <div className={classes.desktop}>
      <header className={classes.menubar}>
        <span className={classes.brand}>LibreBooks</span>
        <TopMenu run={run} onShortcuts={onShortcuts} />
        <span className={classes.spacer} />
        <Tooltip label="Search and go to anything">
          <UnstyledButton className={classes.searchButton} onClick={() => spotlight.open()} aria-label="Search">
            <IconSearch size={14} />
            <span style={{ flex: 1 }}>Search…</span>
            {paletteHint && <Kbd size="xs">{paletteHint}</Kbd>}
          </UnstyledButton>
        </Tooltip>
        <FixtureBadge />
        <LiveIndicator status={live} />
        <ThemeToggle />
        <UserMenu />
      </header>
      <div className={classes.body}>
        <NavRail />
        <main className={classes.main}>
          <WindowTabs />
          <WindowHost />
        </main>
      </div>
    </div>
  );
}
