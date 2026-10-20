// Forms that aren't built yet open this, so every menu item and shortcut goes
// somewhere honest instead of nowhere.

import { Button, Stack, Text, Title } from '@mantine/core';
import { useEffect } from 'react';
import { FORM_LABELS } from '~/shell/menus';
import { useOpenWindow, useWindow, useWindows } from '~/windows/WindowsProvider';
import { paths } from '~/windows/paths';

export function FormPlaceholder() {
  const win = useWindow();
  const open = useOpenWindow();
  const label = FORM_LABELS[win.desc.params.form ?? ''] ?? 'This form';
  useEffect(() => win.setTitle(label), [label]); // eslint-disable-line react-hooks/exhaustive-deps
  return (
    <Stack p="xl" gap="sm" maw={560}>
      <Title order={3}>{label}</Title>
      <Text>
        {label} isn't built yet. Write Checks is the first complete form and the template the others will follow (same header, line grids,
        keys and error handling).
      </Text>
      <Button variant="light" onClick={() => open(paths.newCheck())} w="fit-content">
        Open Write Checks
      </Button>
    </Stack>
  );
}

export function NotFound() {
  const { notFound } = useWindows();
  const open = useOpenWindow();
  return (
    <Stack p="xl" gap="sm">
      <Title order={3}>Nothing here</Title>
      <Text c="dimmed">There's no window for "{notFound}".</Text>
      <Button variant="light" onClick={() => open(paths.home())} w="fit-content">
        Go Home
      </Button>
    </Stack>
  );
}
