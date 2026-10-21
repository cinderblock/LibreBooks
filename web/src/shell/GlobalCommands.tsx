// Handlers for the global commands (QBD's Ctrl+letter navigation), plus the
// small dialogs they open: Use Register's account picker and the shortcut
// overlay. Mounted once in the company shell.

import { Button, Group, Modal, Stack, Text, useMantineColorScheme } from '@mantine/core';
import { spotlight } from '@mantine/spotlight';
import { useState } from 'react';
import { AccountQuickFill } from '~/inputs/pickers';
import { useShortcut } from '~/keyboard/KeyboardProvider';
import { hasRegister } from '~/lib/accountTypes';
import { useAccountIndex } from '~/api/hooks';
import { ACCOUNT_TYPES } from '~/lib/accountTypes';
import type { CommandId } from '~/keyboard/commands';
import { useOpenWindow } from '~/windows/WindowsProvider';
import { paths } from '~/windows/paths';
import { ShortcutOverlay } from './ShortcutOverlay';

const REGISTER_TYPES = ACCOUNT_TYPES.filter(hasRegister);

/** Run a command from a menu click (same effect as its key). */
export type RunCommand = (c: CommandId) => void;

export function useGlobalCommands(): { run: RunCommand; overlay: boolean; setOverlay(v: boolean): void; registerPicker: boolean; setRegisterPicker(v: boolean): void } {
  const open = useOpenWindow();
  const { toggleColorScheme } = useMantineColorScheme();
  const [overlay, setOverlay] = useState(false);
  const [registerPicker, setRegisterPicker] = useState(false);

  const actions: Partial<Record<CommandId, () => void>> = {
    'palette.open': () => spotlight.open(),
    'help.shortcuts': () => setOverlay(true),
    'nav.home': () => open(paths.home()),
    'form.writeChecks': () => open(paths.newCheck()),
    'form.invoice': () => open(paths.form('invoice')),
    'form.bill': () => open(paths.form('bill')),
    'form.payBills': () => open(paths.form('pay-bills')),
    'form.vendorCredit': () => open(paths.form('vendor-credit')),
    'form.purchaseOrder': () => open(paths.form('purchase-order')),
    'form.ccCharge': () => open(paths.form('cc-charge')),
    'form.salesReceipt': () => open(paths.form('sales-receipt')),
    'form.receivePayment': () => open(paths.form('receive-payment')),
    'form.creditMemo': () => open(paths.form('credit-memo')),
    'form.estimate': () => open(paths.form('estimate')),
    'form.salesOrder': () => open(paths.form('sales-order')),
    'nav.statementCharges': () => open(paths.statementCharges()),
    'form.deposit': () => open(paths.form('deposit')),
    'form.transfer': () => open(paths.form('transfer')),
    'form.journal': () => open(paths.form('journal')),
    'nav.reconcile': () => open(paths.form('reconcile')),
    'nav.bankFeeds': () => open(paths.feeds()),
    'nav.customerCenter': () => open(paths.customers()),
    'nav.vendorCenter': () => open(paths.vendors()),
    'nav.useRegister': () => setRegisterPicker(true),
    'nav.chartOfAccounts': () => open(paths.accounts()),
    'nav.find': () => open(paths.find()),
    'nav.memorizedTxns': () => open(paths.memorized()),
    'nav.reports': () => open(paths.reports()),
    'theme.toggle': () => toggleColorScheme(),
  };
  for (const [command, fn] of Object.entries(actions) as Array<[CommandId, () => void]>) {
    // Hooks in a loop are fine here: the object's keys never change.
    // eslint-disable-next-line react-hooks/rules-of-hooks
    useShortcut(command, fn);
  }
  const run: RunCommand = (c) => actions[c]?.();
  return { run, overlay, setOverlay, registerPicker, setRegisterPicker };
}

export function GlobalDialogs({ overlay, setOverlay, registerPicker, setRegisterPicker }: ReturnType<typeof useGlobalCommands>) {
  return (
    <>
      <ShortcutOverlay opened={overlay} onClose={() => setOverlay(false)} />
      <UseRegisterPicker opened={registerPicker} onClose={() => setRegisterPicker(false)} />
    </>
  );
}

function UseRegisterPicker({ opened, onClose }: { opened: boolean; onClose(): void }) {
  const open = useOpenWindow();
  const accounts = useAccountIndex();
  const [id, setId] = useState<string | null>(null);
  const go = (accountId: string | null) => {
    if (!accountId || !accounts.has(accountId)) return;
    onClose();
    open(paths.register(accountId));
  };
  return (
    <Modal opened={opened} onClose={onClose} title="Use Register" centered>
      <Stack>
        <Text size="sm" c="dimmed">
          Select the account whose register you want to open.
        </Text>
        <AccountQuickFill
          label="Account"
          types={REGISTER_TYPES}
          value={id}
          onChange={(v) => {
            setId(v);
            go(v);
          }}
          allowAddNew={false}
          autoFocus
          aria-label="Register account"
        />
        <Group justify="flex-end">
          <Button variant="default" onClick={onClose}>
            Cancel
          </Button>
          <Button onClick={() => go(id)} disabled={!id}>
            Open
          </Button>
        </Group>
      </Stack>
    </Modal>
  );
}
