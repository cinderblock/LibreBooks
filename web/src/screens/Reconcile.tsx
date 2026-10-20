// Reconcile (Banking → Reconcile): QBD's two steps. Begin Reconciliation takes
// the statement date, ending balance and any service charge or interest; then
// the two-column window (checks and payments left, deposits and credits right)
// with Mark All, the running difference, Reconcile Now (with Enter Adjustment
// when the difference won't go), Leave and Discard. Undo Last Reconciliation
// lives on the begin panel with the history. Phones get one column at a time
// and a sticky difference footer.

import { Alert, Button, Card, Checkbox, Group, Modal, SegmentedControl, SimpleGrid, Skeleton, Stack, Table, Text, Title } from '@mantine/core';
import { notifications } from '@mantine/notifications';
import { useEffect, useMemo, useState } from 'react';
import { describeError } from '~/api/errors';
import { useAccountIndex, useReconcileActions, useReconcileOverview, useReconciliation } from '~/api/hooks';
import type { ReconcileLine, Reconciliation, StatementCharge } from '~/api/types';
import { DateField } from '~/inputs/DateField';
import { MoneyField } from '~/inputs/MoneyField';
import { AccountQuickFill } from '~/inputs/pickers';
import { addDays, firstOfMonth, formatDate, formatDateShort, todayIso } from '~/lib/dates';
import { formatMoney } from '~/lib/money';
import { TXN_REGISTER_CODE } from '~/lib/txnTypes';
import { useIsMobile } from '~/lib/useMobile';
import { useOpenWindow, useWindow } from '~/windows/WindowsProvider';
import { paths } from '~/windows/paths';
import classes from './screens.module.css';
import formClasses from '~/forms/forms.module.css';

export function Reconcile() {
  const win = useWindow();
  const accountId = win.desc.params.accountId ?? null;
  const accounts = useAccountIndex();
  const account = accountId ? accounts.get(accountId) : undefined;
  const overview = useReconcileOverview(accountId);
  useEffect(() => {
    if (account) win.setTitle(`Reconcile ${account.full_name}`);
  }, [account?.full_name]); // eslint-disable-line react-hooks/exhaustive-deps
  if (!accountId) return <Alert m="md">No account chosen.</Alert>;
  if (overview.error) return <Alert m="md" color="red">{describeError(overview.error)}</Alert>;
  if (!account || !overview.data) return <Skeleton m="md" h={300} />;
  if (overview.data.in_progress) return <ReconcileWindow account={account} rec={overview.data.in_progress} />;
  return <Begin account={account} beginning={overview.data.beginning_balance} history={overview.data.history} />;
}

function Begin({ account, beginning, history }: { account: { id: string; full_name: string; account_type: string }; beginning: number; history: Reconciliation[] }) {
  const mobile = useIsMobile();
  const win = useWindow();
  const { start, undo } = useReconcileActions(account.id);
  const [statementDate, setStatementDate] = useState<string | null>(() => addDays(firstOfMonth(todayIso()), -1));
  const [ending, setEnding] = useState<number | null>(null);
  const [charge, setCharge] = useState<StatementCharge>({ amount: 0, date: todayIso(), account_id: '' });
  const [interest, setInterest] = useState<StatementCharge>({ amount: 0, date: todayIso(), account_id: '' });
  const [error, setError] = useState<string | null>(null);
  const last = history[0];
  const submit = () => {
    if (!statementDate) return setError('Enter the statement date.');
    if (ending === null) return setError('Enter the ending balance from the statement.');
    if (charge.amount && !charge.account_id) return setError('Choose the account for the service charge.');
    if (interest.amount && !interest.account_id) return setError('Choose the account for the interest earned.');
    setError(null);
    start.mutate(
      { statement_date: statementDate, ending_balance: ending, service_charge: charge.amount ? charge : null, interest: interest.amount ? interest : null },
      { onError: (e) => setError(describeError(e)) },
    );
  };
  return (
    <Stack p={mobile ? 'sm' : 'md'} gap="md" className={classes.page} data-testid="reconcile-begin">
      <Title order={3}>Begin Reconciliation</Title>
      <Text size="sm" c="dimmed">
        {account.full_name}
        {last ? ` · last reconciled ${formatDate(last.statement_date)}` : ' · never reconciled'}
      </Text>
      <SimpleGrid cols={{ base: 1, sm: 3 }} spacing="md">
        <DateField label="Statement Date" value={statementDate} onChange={setStatementDate} aria-label="Statement date" />
        <div>
          <Text size="sm" fw={500}>
            Beginning Balance
          </Text>
          <Text size="lg" fw={600} className={classes.num} data-testid="beginning-balance">
            {formatMoney(beginning)}
          </Text>
        </div>
        <MoneyField label="Ending Balance" value={ending} onChange={setEnding} allowNegative aria-label="Ending balance" />
      </SimpleGrid>
      <Card withBorder padding="sm">
        <Text size="sm" fw={500} mb="xs">
          Enter any service charge or interest earned that is not already in the register
        </Text>
        <SimpleGrid cols={{ base: 1, sm: 3 }} spacing="sm">
          <MoneyField label="Service Charge" value={charge.amount || null} onChange={(v) => setCharge({ ...charge, amount: v ?? 0 })} aria-label="Service charge" />
          <DateField label="Date" value={charge.date} onChange={(d) => d && setCharge({ ...charge, date: d })} aria-label="Service charge date" />
          <AccountQuickFill label="Account" types={['Expense', 'OtherExpense']} value={charge.account_id || null} onChange={(id) => setCharge({ ...charge, account_id: id ?? '' })} allowAddNew={false} aria-label="Service charge account" />
          <MoneyField label="Interest Earned" value={interest.amount || null} onChange={(v) => setInterest({ ...interest, amount: v ?? 0 })} aria-label="Interest earned" />
          <DateField label="Date" value={interest.date} onChange={(d) => d && setInterest({ ...interest, date: d })} aria-label="Interest date" />
          <AccountQuickFill label="Account" types={['Income', 'OtherIncome']} value={interest.account_id || null} onChange={(id) => setInterest({ ...interest, account_id: id ?? '' })} allowAddNew={false} aria-label="Interest account" />
        </SimpleGrid>
      </Card>
      {error && (
        <Alert color="red" role="alert">
          {error}
        </Alert>
      )}
      <Group justify="space-between" wrap="wrap">
        <Group gap="xs">
          {last && (
            <Button
              variant="default"
              loading={undo.isPending}
              onClick={() => {
                if (!window.confirm(`Undo the reconciliation of ${formatDate(last.statement_date)}? Its ${last.cleared_txn_ids.length} cleared transactions become uncleared again.`)) return;
                undo.mutate(undefined, { onSuccess: () => notifications.show({ message: 'Last reconciliation undone.', color: 'gray' }), onError: (e) => setError(describeError(e)) });
              }}
            >
              Undo Last Reconciliation
            </Button>
          )}
        </Group>
        <Group gap="xs">
          <Button variant="default" onClick={() => win.close()}>
            Cancel
          </Button>
          <Button onClick={submit} loading={start.isPending}>
            Continue
          </Button>
        </Group>
      </Group>
      {history.length > 0 && (
        <Card withBorder padding="sm">
          <Text size="sm" fw={500} mb="xs">
            Previous reconciliations
          </Text>
          <Table fz="sm">
            <Table.Thead>
              <Table.Tr>
                <Table.Th>Statement date</Table.Th>
                <Table.Th ta="right">Beginning</Table.Th>
                <Table.Th ta="right">Ending</Table.Th>
                <Table.Th ta="right">Cleared</Table.Th>
                <Table.Th>Finished</Table.Th>
              </Table.Tr>
            </Table.Thead>
            <Table.Tbody>
              {history.map((r) => (
                <Table.Tr key={r.id}>
                  <Table.Td>{formatDate(r.statement_date)}</Table.Td>
                  <Table.Td ta="right">{formatMoney(r.beginning_balance)}</Table.Td>
                  <Table.Td ta="right">{formatMoney(r.ending_balance)}</Table.Td>
                  <Table.Td ta="right">{r.cleared_txn_ids.length}</Table.Td>
                  <Table.Td>
                    {r.finished_at ? formatDate(r.finished_at.slice(0, 10)) : ''}
                    {r.finished_by ? ` by ${r.finished_by}` : ''}
                    {r.adjustment_txn_id ? ' · adjusted' : ''}
                  </Table.Td>
                </Table.Tr>
              ))}
            </Table.Tbody>
          </Table>
        </Card>
      )}
    </Stack>
  );
}

function ReconcileWindow({ account, rec }: { account: { id: string; full_name: string }; rec: Reconciliation }) {
  const mobile = useIsMobile();
  const win = useWindow();
  const open = useOpenWindow();
  const view = useReconciliation(rec.id);
  const { mark, finish, discard } = useReconcileActions(account.id);
  const [side, setSide] = useState<'payments' | 'deposits'>('payments');
  const [adjustOpen, setAdjustOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const lines = view.data?.lines ?? [];
  const status = view.data?.status;
  const payments = useMemo(() => lines.filter((l) => l.amount < 0), [lines]);
  const deposits = useMemo(() => lines.filter((l) => l.amount >= 0), [lines]);
  useEffect(() => win.setDirty((status?.marked_count ?? 0) > 0), [status?.marked_count]); // eslint-disable-line react-hooks/exhaustive-deps

  const toggle = (l: ReconcileLine) => mark.mutate({ id: rec.id, txnIds: [l.txn_id], marked: !l.marked });
  const markAll = (list: ReconcileLine[], marked: boolean) => {
    const ids = list.filter((l) => l.marked !== marked).map((l) => l.txn_id);
    if (ids.length) mark.mutate({ id: rec.id, txnIds: ids, marked });
  };
  const reconcileNow = () => {
    if (!status) return;
    if (status.difference !== 0) return setAdjustOpen(true);
    finish.mutate({ id: rec.id, adjust: false }, { onSuccess: done, onError: (e) => setError(describeError(e)) });
  };
  const done = (r: Reconciliation) => {
    notifications.show({ message: `Reconciled ${account.full_name} through ${formatDate(r.statement_date)}: ${r.cleared_txn_ids.length} transactions cleared.`, color: 'teal' });
    setAdjustOpen(false);
  };

  const column = (title: string, list: ReconcileLine[], label: string) => (
    <Card withBorder padding={0} className={classes.reconcileColumn} data-testid={`reconcile-${label}`}>
      <Group justify="space-between" px="sm" py={6} className={classes.reconcileHead}>
        <Text fw={600} size="sm">
          {title}
        </Text>
        <Group gap={4}>
          <Button size="compact-xs" variant="subtle" onClick={() => markAll(list, true)} disabled={!list.length}>
            Mark All
          </Button>
          <Button size="compact-xs" variant="subtle" onClick={() => markAll(list, false)} disabled={!list.some((l) => l.marked)}>
            Unmark All
          </Button>
        </Group>
      </Group>
      <div className={classes.reconcileScroll}>
        <Table fz="sm" highlightOnHover stickyHeader>
          <Table.Thead>
            <Table.Tr>
              <Table.Th w={36}>✓</Table.Th>
              <Table.Th>Date</Table.Th>
              <Table.Th>Chk #</Table.Th>
              <Table.Th>Payee</Table.Th>
              <Table.Th ta="right">Amount</Table.Th>
            </Table.Tr>
          </Table.Thead>
          <Table.Tbody>
            {list.map((l) => (
              <Table.Tr key={l.txn_id} data-marked={l.marked || undefined} onDoubleClick={() => open(paths.txn(l.txn_id))}>
                <Table.Td>
                  <Checkbox size="xs" aria-label={`Mark ${l.payee ?? TXN_REGISTER_CODE[l.txn_type]} ${formatMoney(Math.abs(l.amount))}`} checked={l.marked} onChange={() => toggle(l)} />
                </Table.Td>
                <Table.Td>{formatDateShort(l.date)}</Table.Td>
                <Table.Td>{l.number ?? TXN_REGISTER_CODE[l.txn_type]}</Table.Td>
                <Table.Td>
                  <Text size="sm" truncate maw={mobile ? 120 : 220}>
                    {l.payee ?? l.memo}
                  </Text>
                </Table.Td>
                <Table.Td ta="right" className={classes.num}>
                  {formatMoney(Math.abs(l.amount))}
                </Table.Td>
              </Table.Tr>
            ))}
            {list.length === 0 && (
              <Table.Tr>
                <Table.Td colSpan={5}>
                  <Text size="sm" c="dimmed">
                    Nothing here through {formatDateShort(rec.statement_date)}.
                  </Text>
                </Table.Td>
              </Table.Tr>
            )}
          </Table.Tbody>
        </Table>
      </div>
    </Card>
  );

  const diff = status?.difference ?? 0;
  const summary = status && (
    <div className={formClasses.totals} data-testid="reconcile-summary">
      <span className={formClasses.label}>Beginning Balance</span>
      <span className={formClasses.value}>{formatMoney(status.beginning_balance)}</span>
      <span className={formClasses.label}>{payments.filter((l) => l.marked).length} checks and payments</span>
      <span className={formClasses.value}>{formatMoney(status.marked_decreases)}</span>
      <span className={formClasses.label}>{deposits.filter((l) => l.marked).length} deposits and credits</span>
      <span className={formClasses.value}>{formatMoney(status.marked_increases)}</span>
      <span className={formClasses.label}>Cleared Balance</span>
      <span className={formClasses.value}>{formatMoney(status.cleared_balance)}</span>
      <span className={formClasses.label}>Ending Balance</span>
      <span className={formClasses.value}>{formatMoney(status.ending_balance)}</span>
      <span className={`${formClasses.label} ${formClasses.grand}`}>Difference</span>
      <span className={`${formClasses.value} ${formClasses.grand} ${diff === 0 ? formClasses.balanced : formClasses.unbalanced}`} aria-label="Difference">
        {formatMoney(diff)}
      </span>
    </div>
  );

  return (
    <div className={mobile ? classes.mobileFill : classes.fill} data-testid="reconcile">
      <Group justify="space-between" px={mobile ? 'sm' : 0} pt={mobile ? 'xs' : 0} wrap="wrap">
        <Text size="sm" c="dimmed">
          Statement {formatDate(rec.statement_date)} · ending balance{' '}
          <Text span fw={600} c="var(--mantine-color-text)">
            {formatMoney(rec.ending_balance)}
          </Text>
        </Text>
        {mobile && (
          <SegmentedControl
            size="xs"
            data={[
              { value: 'payments', label: `Payments (${payments.length})` },
              { value: 'deposits', label: `Deposits (${deposits.length})` },
            ]}
            value={side}
            onChange={(v) => setSide(v as 'payments' | 'deposits')}
          />
        )}
      </Group>
      {view.error && <Alert color="red">{describeError(view.error)}</Alert>}
      {!view.data ? (
        <Skeleton h={300} />
      ) : mobile ? (
        side === 'payments' ? column('Checks and Payments', payments, 'payments') : column('Deposits and Other Credits', deposits, 'deposits')
      ) : (
        <div className={classes.reconcileColumns}>
          {column('Checks and Payments', payments, 'payments')}
          {column('Deposits and Other Credits', deposits, 'deposits')}
        </div>
      )}
      {error && (
        <Alert color="red" role="alert">
          {error}
        </Alert>
      )}
      <div className={mobile ? classes.reconcileFooterMobile : classes.reconcileFooter}>
        {summary}
        <Group gap="xs" justify="flex-end" wrap="wrap">
          <Button
            variant="subtle"
            color="red"
            onClick={() => {
              if (window.confirm('Discard this reconciliation? Marks made so far are cleared.')) discard.mutate(rec.id, { onError: (e) => setError(describeError(e)) });
            }}
          >
            Discard
          </Button>
          <Button variant="default" onClick={() => win.close({ force: true })}>
            Leave
          </Button>
          <Button onClick={reconcileNow} loading={finish.isPending} disabled={!status}>
            Reconcile Now
          </Button>
        </Group>
      </div>
      <Modal opened={adjustOpen} onClose={() => setAdjustOpen(false)} title="Reconcile Adjustment" centered>
        <Stack gap="sm">
          <Text size="sm">
            There is a {formatMoney(Math.abs(diff))} difference between the statement's ending balance and the cleared balance. LibreBooks can write it off as a Balance Adjustment to Reconciliation Discrepancies, dated {formatDate(rec.statement_date)}.
          </Text>
          <Text size="xs" c="dimmed">
            Better: Leave, find the transaction that's missing or wrong, and come back. The adjustment can be deleted later.
          </Text>
          <Group justify="flex-end">
            <Button variant="default" onClick={() => setAdjustOpen(false)}>
              Return to Reconcile
            </Button>
            <Button color="orange" loading={finish.isPending} onClick={() => finish.mutate({ id: rec.id, adjust: true }, { onSuccess: done, onError: (e) => setError(describeError(e)) })}>
              Enter Adjustment
            </Button>
          </Group>
        </Stack>
      </Modal>
    </div>
  );
}
