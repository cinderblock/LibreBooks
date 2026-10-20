// Company information (read-only until the settings forms exist) and this
// device's preferences: the keymap (desktop app vs browser) and what Enter does.

import { Card, Group, SegmentedControl, SimpleGrid, Skeleton, Stack, Table, Text, Title } from '@mantine/core';
import { useCompanyInfo } from '~/api/hooks';
import { useApi } from '~/api/context';
import { useKeyboard } from '~/keyboard/KeyboardProvider';
import { differencesFromQbd } from '~/keyboard/keymaps';
import { COMMANDS } from '~/keyboard/commands';
import { formatCombo } from '~/keyboard/combo';
import { useEnterBehavior } from '~/lib/prefs';
import type { EnterBehavior } from '~/inputs/LineGrid';
import classes from './screens.module.css';

export function Company() {
  const { data: info, isLoading } = useCompanyInfo();
  const api = useApi();
  const { platform, setPlatform, os } = useKeyboard();
  const [enter, setEnter] = useEnterBehavior();
  const diffs = differencesFromQbd('browser', os);
  return (
    <Stack p="md" gap="md" className={classes.page}>
      <Title order={3}>Company</Title>
      {isLoading || !info ? (
        <Skeleton h={160} />
      ) : (
        <SimpleGrid cols={{ base: 1, md: 2 }}>
          <Card withBorder>
            <Text fw={600}>{info.name}</Text>
            {info.legal_name && <Text c="dimmed">{info.legal_name}</Text>}
            {(info.address.lines ?? []).map((l) => (
              <Text key={l} size="sm">
                {l}
              </Text>
            ))}
            <Text size="sm">{[info.address.city, info.address.state, info.address.postal_code].filter(Boolean).join(' ')}</Text>
            {info.phone && <Text size="sm">{info.phone}</Text>}
            {info.email && <Text size="sm">{info.email}</Text>}
          </Card>
          <Card withBorder>
            <Table fz="sm">
              <Table.Tbody>
                <Table.Tr>
                  <Table.Td c="dimmed">Fiscal year starts</Table.Td>
                  <Table.Td>{new Date(2000, info.fiscal_year_start_month - 1, 1).toLocaleString(undefined, { month: 'long' })}</Table.Td>
                </Table.Tr>
                <Table.Tr>
                  <Table.Td c="dimmed">Tax form</Table.Td>
                  <Table.Td>{info.tax_form ?? '—'}</Table.Td>
                </Table.Tr>
                <Table.Tr>
                  <Table.Td c="dimmed">Class tracking</Table.Td>
                  <Table.Td>{info.preferences.use_class_tracking ? 'On' : 'Off'}</Table.Td>
                </Table.Tr>
                <Table.Tr>
                  <Table.Td c="dimmed">Data source</Table.Td>
                  <Table.Td>{api.mode === 'fixtures' ? 'Fixture mode (synthetic, in memory)' : 'Server'}</Table.Td>
                </Table.Tr>
              </Table.Tbody>
            </Table>
          </Card>
        </SimpleGrid>
      )}

      <Title order={4}>My preferences (this device)</Title>
      <Card withBorder>
        <Stack gap="md">
          <div>
            <Text fw={500}>Keyboard shortcuts</Text>
            <Text size="sm" c="dimmed" mb="xs">
              The desktop app uses QuickBooks Desktop's keys. A browser keeps some keys for itself (Ctrl+W closes the tab), so the browser keymap uses alternates for those.
            </Text>
            <SegmentedControl
              data={[
                { value: 'browser', label: 'Browser keymap' },
                { value: 'desktop', label: 'Desktop app (QBD) keymap' },
              ]}
              value={platform}
              onChange={(v) => setPlatform(v as 'browser' | 'desktop')}
            />
            {diffs.length > 0 && (
              <Table fz="xs" mt="sm" maw={560}>
                <Table.Thead>
                  <Table.Tr>
                    <Table.Th>Command</Table.Th>
                    <Table.Th>QuickBooks</Table.Th>
                    <Table.Th>In a browser</Table.Th>
                  </Table.Tr>
                </Table.Thead>
                <Table.Tbody>
                  {diffs.map((d) => (
                    <Table.Tr key={d.command}>
                      <Table.Td>{COMMANDS[d.command].label}</Table.Td>
                      <Table.Td>{d.qbd.map((c) => formatCombo(c, os)).join(', ')}</Table.Td>
                      <Table.Td>{d.here.map((c) => formatCombo(c, os)).join(', ')}</Table.Td>
                    </Table.Tr>
                  ))}
                </Table.Tbody>
              </Table>
            )}
          </div>
          <div>
            <Text fw={500}>Enter key in line grids</Text>
            <Group gap="xs" mt="xs">
              <SegmentedControl
                data={[
                  { value: 'next-cell', label: 'Next field' },
                  { value: 'next-row', label: 'Next line' },
                  { value: 'save', label: 'Save the form' },
                ]}
                value={enter}
                onChange={(v) => setEnter(v as EnterBehavior)}
              />
            </Group>
          </div>
        </Stack>
      </Card>
    </Stack>
  );
}
