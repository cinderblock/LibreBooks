// The alpha-software notice's modal and the "Alpha" badge that reopens it.
// The logic (who accepted what, holding changes) is in ./notice.ts.

import { Alert, Badge, Button, Checkbox, Group, List, Modal, Stack, Text, Title } from '@mantine/core';
import { IconAlertTriangle } from '@tabler/icons-react';
import { type ReactNode, useEffect, useState, useSyncExternalStore } from 'react';
import { getAlphaNotice } from '~/api/select';
import { formatDateTime } from '~/lib/dates';
import { isCurrent } from './notice';

function useAlphaNotice() {
  const notice = getAlphaNotice();
  return [useSyncExternalStore(notice.subscribe, notice.getSnapshot), notice] as const;
}

/** When, in local time with its zone (`11/5/2026 2:02 AM PST`). */
function acceptedWhen(iso: string): string {
  const zone = new Intl.DateTimeFormat(undefined, { timeZoneName: 'short' }).formatToParts(new Date(iso)).find((p) => p.type === 'timeZoneName')?.value;
  return zone ? `${formatDateTime(iso)} ${zone}` : formatDateTime(iso);
}

/** Mounted once, at the root: opens whenever a change is waiting on the notice, or the badge asks. */
export function AlphaNoticeModal() {
  const [state, notice] = useAlphaNotice();
  const [understood, setUnderstood] = useState(false);
  const opened = state.prompt !== 'none';
  const required = state.prompt === 'required';
  const accepted = isCurrent(state.accepted) ? state.accepted : null;
  useEffect(() => {
    if (!opened) setUnderstood(false);
  }, [opened]);

  return (
    <Modal
      opened={opened}
      onClose={() => notice.dismiss()}
      // Clicking outside is too easy to do by accident; Escape and the buttons are deliberate.
      closeOnClickOutside={!required}
      withCloseButton={!required}
      title={
        <Group gap="xs">
          <IconAlertTriangle size={20} color="var(--mantine-color-orange-6)" />
          <Text fw={600}>LibreBooks is alpha software</Text>
        </Group>
      }
      size="lg"
      centered
      data-testid="alpha-notice"
    >
      <Stack gap="sm">
        {required && <Text>Before your first change, please read this and accept it.</Text>}
        <Text>LibreBooks is early and unfinished. It has bugs, and it can get things wrong, including the numbers.</Text>
        <List spacing={6} size="sm">
          <List.Item>
            <b>Don't make it your only books yet.</b> Keep QuickBooks, or whatever you use now, as your books of record, and compare.
          </List.Item>
          <List.Item>
            <b>Keep backups.</b> A bug could record something wrong or lose a change.
          </List.Item>
          <List.Item>
            <b>Check before you rely on it.</b> Look over balances and reports before you pay, file or decide anything with them.
          </List.Item>
          <List.Item>
            <b>Expect change.</b> Screens and behaviour will move as it's built.
          </List.Item>
        </List>
        <Text size="sm" c="dimmed">
          It is provided as is, without warranty of any kind (MIT licence). You use it at your own risk.
        </Text>

        {accepted ? (
          <Text size="sm" c="dimmed" data-testid="alpha-notice-accepted">
            You accepted this on {acceptedWhen(accepted.accepted_at)}.
          </Text>
        ) : (
          <>
            <Checkbox
              checked={understood}
              onChange={(e) => setUnderstood(e.currentTarget.checked)}
              label="I understand that LibreBooks is alpha software and I use it at my own risk."
              data-autofocus
            />
            <Text size="xs" c="dimmed">
              You're asked once{state.accepted ? ' more, because the notice has changed' : ''}; LibreBooks remembers your answer on every device.
            </Text>
          </>
        )}
        {state.error && (
          <Alert color="red" title="Couldn't record your answer">
            {state.error}
          </Alert>
        )}
        <Group justify="flex-end" mt="xs">
          {accepted ? (
            <Button onClick={() => notice.dismiss()}>Close</Button>
          ) : (
            <>
              <Button variant="default" onClick={() => notice.dismiss()}>
                {required ? 'Cancel the change' : 'Not now'}
              </Button>
              <Button color="orange" disabled={!understood} loading={state.busy} onClick={() => void notice.accept()}>
                {required ? 'Accept and continue' : 'Accept'}
              </Button>
            </>
          )}
        </Group>
      </Stack>
    </Modal>
  );
}

/** The always-visible marker. A button, not a hover tooltip: it opens the notice (phones have no hover). */
export function AlphaBadge({ size = 'sm' }: { size?: 'xs' | 'sm' | 'md' }) {
  const notice = getAlphaNotice();
  return (
    <Badge
      component="button"
      type="button"
      variant="light"
      color="orange"
      size={size}
      style={{ cursor: 'pointer', flexShrink: 0 }}
      onClick={() => notice.view()}
      aria-label="Alpha software: read the notice"
      data-testid="alpha-badge"
    >
      Alpha
    </Badge>
  );
}

/** A page title with the badge beside it (sign-in, enrolment, the company list). */
export function AlphaTitle({ children }: { children: ReactNode }) {
  return (
    <Group gap="xs" align="center" wrap="nowrap">
      <Title order={2}>{children}</Title>
      <AlphaBadge size="md" />
    </Group>
  );
}
