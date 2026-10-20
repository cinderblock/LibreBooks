import { describe, expect, test } from 'bun:test';
import { allMenuItems, MENUS, visibleEntries, type MenuEntry } from './menus';

describe('menus', () => {
  test('admin items are hidden from non-admins, along with submenus and dividers left empty', () => {
    const entries: MenuEntry[] = [
      { label: 'Home', path: '/home' },
      { divider: true },
      { label: 'Admin only', path: '/admin/x', admin: true },
      { divider: true },
      { label: 'Administration', children: [{ label: 'Users', path: '/admin/users', admin: true }] },
      { divider: true },
      { label: 'Mixed', children: [{ label: 'Users', path: '/admin/users', admin: true }, { label: 'Find', path: '/find' }] },
      { divider: true },
    ];
    const seen = visibleEntries(entries, false);
    expect(seen).toEqual([{ label: 'Home', path: '/home' }, { divider: true }, { label: 'Mixed', children: [{ label: 'Find', path: '/find' }] }]);
    expect(visibleEntries(entries, true)).toBe(entries);
  });

  test('the palette list drops admin items for non-admins only', () => {
    const all = allMenuItems(true);
    const some = allMenuItems(false);
    expect(all.some((i) => i.admin)).toBe(true);
    expect(some.some((i) => i.admin)).toBe(false);
    expect(all.length - some.length).toBe(all.filter((i) => i.admin).length);
  });

  test('every menu path is company-relative and every item has a destination', () => {
    for (const m of MENUS) {
      for (const e of visibleEntries(m.entries, true)) {
        if ('divider' in e) continue;
        const items = 'children' in e ? e.children : [e];
        for (const i of items) {
          expect(!!(i.path ?? i.command)).toBe(true);
          if (i.path) expect(i.path.startsWith('/')).toBe(true);
        }
      }
    }
  });
});
