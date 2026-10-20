// The `?` / F1 overlay: every shortcut on this platform, grouped by where it
// works, with the ones the current window handles marked, plus the keys fields
// handle themselves (date keys, the calculator, QuickFill, grids).

import { Badge, Group, Kbd, Modal, SegmentedControl, SimpleGrid, Stack, Table, Text } from '@mantine/core';
import { COMMAND_IDS, COMMANDS, FIELD_KEYS, SCOPE_LABEL, type Scope } from '~/keyboard/commands';
import { useKeyboard, useShortcutCatalog } from '~/keyboard/KeyboardProvider';

const SCOPES: Scope[] = ['global', 'window', 'form', 'grid'];

export function ShortcutOverlay({ opened, onClose }: { opened: boolean; onClose(): void }) {
  const { registry, format, platform } = useShortcutCatalog();
  const { setPlatform } = useKeyboard();
  const active = new Set(registry.activeCommands().map((a) => a.command));
  // A plain string title: Modal's title is already an <h2>, and a <Title> inside it nests headings (React warns).
  return (
    <Modal opened={opened} onClose={onClose} title="Keyboard shortcuts" size="xl">
      <Group justify="space-between" mb="sm">
        <Text size="sm" c="dimmed">
          Highlighted keys work in the current window. QuickBooks Desktop users: the keys are the ones you know
          {platform === 'browser' ? ', except where a browser keeps a key for itself (shown with its alternate).' : '.'}
        </Text>
        <SegmentedControl
          size="xs"
          data={[
            { value: 'browser', label: 'Browser' },
            { value: 'desktop', label: 'Desktop app' },
          ]}
          value={platform}
          onChange={(v) => setPlatform(v as 'browser' | 'desktop')}
          aria-label="Keymap"
        />
      </Group>
      <SimpleGrid cols={{ base: 1, md: 2 }} spacing="lg">
        {SCOPES.map((scope) => {
          const commands = COMMAND_IDS.filter((c) => COMMANDS[c].scope === scope && format(c).length > 0);
          if (!commands.length) return null;
          return (
            <Stack key={scope} gap={4}>
              <Text fw={600}>{SCOPE_LABEL[scope]}</Text>
              <Table fz="sm" verticalSpacing={3}>
                <Table.Tbody>
                  {commands.map((c) => (
                    <Table.Tr key={c} style={{ opacity: active.has(c) ? 1 : 0.55 }}>
                      <Table.Td>
                        {COMMANDS[c].label}
                        {active.has(c) && (
                          <Badge size="xs" variant="light" ml={6}>
                            here
                          </Badge>
                        )}
                      </Table.Td>
                      <Table.Td ta="right">
                        {format(c).map((k) => (
                          <Kbd key={k} size="xs" ml={4}>
                            {k}
                          </Kbd>
                        ))}
                      </Table.Td>
                    </Table.Tr>
                  ))}
                </Table.Tbody>
              </Table>
            </Stack>
          );
        })}
        {FIELD_KEYS.map((group) => (
          <Stack key={group.where} gap={4}>
            <Text fw={600}>{group.where}</Text>
            <Table fz="sm" verticalSpacing={3}>
              <Table.Tbody>
                {group.keys.map(([k, what]) => (
                  <Table.Tr key={k}>
                    <Table.Td>{what}</Table.Td>
                    <Table.Td ta="right">
                      <Kbd size="xs">{k}</Kbd>
                    </Table.Td>
                  </Table.Tr>
                ))}
              </Table.Tbody>
            </Table>
          </Stack>
        ))}
      </SimpleGrid>
    </Modal>
  );
}
