// Importing a bank file: detect it first (POST /feeds/detect), show what was
// found (the OFX statements, or a CSV mapping guess with a preview to check and
// fix), choose the register, then import (POST /feeds/import) and show what the
// pipeline did. Also the Amazon panel: an order-history export matched to the
// card's downloads, applied as memos and links.

import { Alert, Button, Checkbox, FileInput, Group, Modal, NumberInput, Select, Stack, Table, Text } from '@mantine/core';
import { notifications } from '@mantine/notifications';
import { useEffect, useState } from 'react';
import { useApi, useCompanyId } from '~/api/context';
import { describeError } from '~/api/errors';
import { useAccountIndex, useAccounts, useFeedConnections, useFeedInvalidate } from '~/api/hooks';
import type { AmazonPreview, CsvColumn, CsvMapping, FeedDetection, IngestReport } from '~/api/types';
import { formatDateShort } from '~/lib/dates';
import { formatMoney } from '~/lib/money';
import classes from '../screens.module.css';

const FEED_ACCOUNT_TYPES = ['Bank', 'CreditCard', 'OtherCurrentAsset', 'OtherCurrentLiability', 'LongTermLiability'] as const;

export function ImportModal({ opened, accountId, onClose }: { opened: boolean; accountId: string | null; onClose(): void }) {
  const api = useApi();
  const co = useCompanyId();
  const refresh = useFeedInvalidate();
  const { data: accountList = [] } = useAccounts();
  const connections = useFeedConnections();
  const [file, setFile] = useState<File | null>(null);
  const [detection, setDetection] = useState<FeedDetection | null>(null);
  const [mapping, setMapping] = useState<CsvMapping | null>(null);
  const [statement, setStatement] = useState(0);
  const [account, setAccount] = useState<string | null>(accountId);
  const [report, setReport] = useState<IngestReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const feedAccounts = accountList.filter((a) => a.is_active && (FEED_ACCOUNT_TYPES as readonly string[]).includes(a.account_type));

  useEffect(() => {
    if (opened) {
      setFile(null);
      setDetection(null);
      setMapping(null);
      setReport(null);
      setError(null);
      setAccount(accountId);
    }
  }, [opened, accountId]);

  const detect = async (f: File | null) => {
    setFile(f);
    setDetection(null);
    setReport(null);
    setError(null);
    if (!f) return;
    setBusy(true);
    try {
      const d = await api.feedDetect(co, f);
      setDetection(d);
      setMapping(d.csv?.mapping ?? null);
      // A file from a known bank account lands in its connection's register.
      const key = d.statements?.[0]?.account.account_key;
      const conn = key ? connections.data?.find((c) => c.account_key === key) : undefined;
      if (conn?.account_id && !account) setAccount(conn.account_id);
    } catch (e) {
      setError(describeError(e));
    } finally {
      setBusy(false);
    }
  };
  const doImport = async () => {
    if (!file) return;
    setBusy(true);
    setError(null);
    try {
      const r = await api.feedImport(co, { file, account_id: account, mapping: detection?.kind === 'Csv' ? mapping : null, statement: detection?.kind === 'Ofx' ? statement : null });
      setReport(r);
      refresh();
    } catch (e) {
      setError(describeError(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal opened={opened} onClose={onClose} title="Import a bank file" size="xl" centered>
      <Stack gap="sm">
        <FileInput label="File from your bank (.qbo, .qfx, .ofx or .csv)" placeholder="Choose a file" accept=".qbo,.qfx,.ofx,.csv,text/csv,application/x-ofx" value={file} onChange={(f) => void detect(f)} clearable data-testid="feed-file" />
        {busy && !detection && <Text size="sm" c="dimmed">Reading…</Text>}
        {detection?.kind === 'Ofx' && detection.statements && (
          <Stack gap="xs">
            {detection.statements.length > 1 && (
              <Select label="Statement" data={detection.statements.map((s, i) => ({ value: String(i), label: `${s.account.institution ?? ''} ${s.account.account_id_masked} (${s.transactions.length})` }))} value={String(statement)} onChange={(v) => setStatement(Number(v))} allowDeselect={false} />
            )}
            <StatementSummary s={detection.statements[statement] ?? detection.statements[0]!} />
          </Stack>
        )}
        {detection?.kind === 'Csv' && detection.csv && mapping && (
          <Stack gap="xs">
            {!detection.csv.confident && <Alert color="orange">The columns were guessed; check the mapping and the preview before importing.</Alert>}
            {detection.csv.notes.map((n) => (
              <Text key={n} size="xs" c="dimmed">
                {n}
              </Text>
            ))}
            <MappingEditor mapping={mapping} headers={detection.csv.headers} onChange={setMapping} />
            <StatementSummary s={detection.csv.preview} />
          </Stack>
        )}
        {detection && (
          <Select label="Into which register" data={feedAccounts.map((a) => ({ value: a.id, label: a.full_name }))} value={account} onChange={setAccount} searchable aria-label="Register" />
        )}
        {error && <Alert color="red">{error}</Alert>}
        {report && (
          <Alert color="teal" title="Imported" data-testid="feed-import-report">
            {report.import.imported.length} new, {report.import.duplicates} already there, {report.import.before_start} before the start date · {report.matched.length} matched, {report.auto_added.length} added by rules
            {report.auto_add_failures.length ? ` (${report.auto_add_failures.length} failed)` : ''} · {report.waiting} waiting for you.
            {report.import.warnings.length > 0 && <div>{report.import.warnings.join(' ')}</div>}
          </Alert>
        )}
        <Group justify="flex-end">
          <Button variant="default" onClick={onClose}>
            {report ? 'Done' : 'Cancel'}
          </Button>
          {!report && (
            <Button loading={busy} onClick={() => void doImport()} disabled={!detection || !account}>
              Import
            </Button>
          )}
        </Group>
      </Stack>
    </Modal>
  );
}

function StatementSummary({ s }: { s: FeedDetection['statements'] extends (infer T)[] | undefined ? T : never }) {
  return (
    <Stack gap={4}>
      <Text size="sm">
        {s.account.institution ?? 'Statement'} {s.account.account_id_masked} · {s.transactions.length} transactions
        {s.start ? ` from ${formatDateShort(s.start)}` : ''}
        {s.end ? ` to ${formatDateShort(s.end)}` : ''}
        {s.ledger_balance !== null ? ` · balance ${formatMoney(s.ledger_balance)}` : ''}
      </Text>
      {s.warnings.map((w) => (
        <Text key={w} size="xs" c="orange">
          {w}
        </Text>
      ))}
      <Table fz="xs" withTableBorder>
        <Table.Thead>
          <Table.Tr>
            <Table.Th>Date</Table.Th>
            <Table.Th>Payee</Table.Th>
            <Table.Th>Memo</Table.Th>
            <Table.Th>Chk</Table.Th>
            <Table.Th ta="right">Amount</Table.Th>
          </Table.Tr>
        </Table.Thead>
        <Table.Tbody>
          {s.transactions.slice(0, 8).map((t) => (
            <Table.Tr key={t.fitid}>
              <Table.Td>{formatDateShort(t.posted)}</Table.Td>
              <Table.Td>{t.payee}</Table.Td>
              <Table.Td>{t.memo}</Table.Td>
              <Table.Td>{t.check_number}</Table.Td>
              <Table.Td ta="right" className={classes.num}>
                {formatMoney(t.amount)}
              </Table.Td>
            </Table.Tr>
          ))}
        </Table.Tbody>
      </Table>
      {s.transactions.length > 8 && (
        <Text size="xs" c="dimmed">
          …and {s.transactions.length - 8} more.
        </Text>
      )}
    </Stack>
  );
}

/** Which column is which, by header name or position. */
function MappingEditor({ mapping, headers, onChange }: { mapping: CsvMapping; headers: string[]; onChange(m: CsvMapping): void }) {
  const opts = headers.length ? headers.map((h, i) => ({ value: `n:${h}`, label: h || `(column ${i + 1})` })) : Array.from({ length: 8 }, (_, i) => ({ value: `i:${i}`, label: `Column ${i + 1}` }));
  const toValue = (c: CsvColumn | null): string | null => (c === null ? null : 'Name' in c ? `n:${c.Name}` : `i:${c.Index}`);
  const fromValue = (v: string | null): CsvColumn | null => (v === null ? null : v.startsWith('n:') ? { Name: v.slice(2) } : { Index: Number(v.slice(2)) });
  const col = (label: string, value: CsvColumn | null, set: (c: CsvColumn | null) => void, required = false) => (
    <Select size="xs" label={label} data={opts} value={toValue(value)} onChange={(v) => set(fromValue(v))} clearable={!required} allowDeselect={!required} searchable />
  );
  const a = mapping.amount;
  const amountMode = 'Signed' in a ? 'signed' : 'DebitCredit' in a ? 'debitcredit' : 'type';
  return (
    <Stack gap="xs">
      <Group gap="xs" align="flex-end" wrap="wrap">
        <Checkbox size="xs" label="First row is a header" checked={mapping.has_header} onChange={(e) => onChange({ ...mapping, has_header: e.currentTarget.checked })} mb={6} />
        <NumberInput size="xs" label="Skip rows at the top" value={mapping.skip_rows} onChange={(v) => onChange({ ...mapping, skip_rows: Number(v) || 0 })} min={0} w={130} />
        <Select size="xs" label="Delimiter" data={[{ value: ',', label: 'comma' }, { value: ';', label: 'semicolon' }, { value: '\t', label: 'tab' }, { value: '|', label: 'pipe' }]} value={mapping.delimiter} onChange={(v) => v && onChange({ ...mapping, delimiter: v })} allowDeselect={false} w={110} />
        <Select size="xs" label="Date order" data={[{ value: 'Mdy', label: 'M/D/Y' }, { value: 'Dmy', label: 'D/M/Y' }, { value: 'Ymd', label: 'Y-M-D' }]} value={mapping.date_order} onChange={(v) => v && onChange({ ...mapping, date_order: v as CsvMapping['date_order'] })} allowDeselect={false} w={110} />
      </Group>
      <Group gap="xs" align="flex-end" wrap="wrap">
        {col('Date', mapping.date, (c) => onChange({ ...mapping, date: c ?? mapping.date }), true)}
        {col('Payee', mapping.payee, (c) => onChange({ ...mapping, payee: c }))}
        {col('Memo', mapping.memo, (c) => onChange({ ...mapping, memo: c }))}
        {col('Check number', mapping.check_number, (c) => onChange({ ...mapping, check_number: c }))}
        {col("Bank's id", mapping.fitid, (c) => onChange({ ...mapping, fitid: c }))}
      </Group>
      <Group gap="xs" align="flex-end" wrap="wrap">
        <Select
          size="xs"
          label="Amount is"
          data={[{ value: 'signed', label: 'one signed column' }, { value: 'debitcredit', label: 'debit and credit columns' }, { value: 'type', label: 'amount plus a type column' }]}
          value={amountMode}
          onChange={(v) => {
            if (v === 'signed') onChange({ ...mapping, amount: { Signed: { column: { Index: 2 }, positive_is: 'In' } } });
            else if (v === 'debitcredit') onChange({ ...mapping, amount: { DebitCredit: { debit: { Index: 2 }, credit: { Index: 3 } } } });
            else onChange({ ...mapping, amount: { AmountAndType: { amount: { Index: 2 }, type_column: { Index: 3 }, out_values: ['debit'] } } });
          }}
          allowDeselect={false}
          w={200}
        />
        {'Signed' in a && (
          <>
            {col('Amount', a.Signed.column, (c) => onChange({ ...mapping, amount: { Signed: { ...a.Signed, column: c ?? a.Signed.column } } }), true)}
            <Select size="xs" label="Positive means" data={[{ value: 'In', label: 'money in' }, { value: 'Out', label: 'money out' }]} value={a.Signed.positive_is} onChange={(v) => v && onChange({ ...mapping, amount: { Signed: { ...a.Signed, positive_is: v as 'In' | 'Out' } } })} allowDeselect={false} w={130} />
          </>
        )}
        {'DebitCredit' in a && (
          <>
            {col('Debit', a.DebitCredit.debit, (c) => onChange({ ...mapping, amount: { DebitCredit: { ...a.DebitCredit, debit: c ?? a.DebitCredit.debit } } }), true)}
            {col('Credit', a.DebitCredit.credit, (c) => onChange({ ...mapping, amount: { DebitCredit: { ...a.DebitCredit, credit: c ?? a.DebitCredit.credit } } }), true)}
          </>
        )}
        {'AmountAndType' in a && (
          <>
            {col('Amount', a.AmountAndType.amount, (c) => onChange({ ...mapping, amount: { AmountAndType: { ...a.AmountAndType, amount: c ?? a.AmountAndType.amount } } }), true)}
            {col('Type', a.AmountAndType.type_column, (c) => onChange({ ...mapping, amount: { AmountAndType: { ...a.AmountAndType, type_column: c ?? a.AmountAndType.type_column } } }), true)}
          </>
        )}
      </Group>
    </Stack>
  );
}

/** Amazon: match an order-history export to the card's downloads, then write the orders onto them. */
export function AmazonPanel() {
  const api = useApi();
  const co = useCompanyId();
  const refresh = useFeedInvalidate();
  const accounts = useAccountIndex();
  const { data: accountList = [] } = useAccounts();
  const [account, setAccount] = useState<string | null>(null);
  const [file, setFile] = useState<File | null>(null);
  const [preview, setPreview] = useState<AmazonPreview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const cards = accountList.filter((a) => a.is_active && (a.account_type === 'CreditCard' || a.account_type === 'Bank'));
  const load = async (f: File | null) => {
    setFile(f);
    setPreview(null);
    setError(null);
    if (!f) return;
    setBusy(true);
    try {
      setPreview(await api.amazonOrders(co, f, account));
    } catch (e) {
      setError(describeError(e));
    } finally {
      setBusy(false);
    }
  };
  const apply = async () => {
    if (!preview) return;
    setBusy(true);
    try {
      const n = await api.amazonApply(co, preview);
      refresh();
      notifications.show({ message: `${n} download(s) now carry their Amazon order.`, color: 'teal' });
      setPreview(null);
      setFile(null);
    } catch (e) {
      setError(describeError(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Stack gap="sm" maw={900}>
      <Text size="sm" c="dimmed">
        Amazon charges your card per shipment, so the bank line says only "AMAZON". Export your orders (Amazon → Your Account → Request your data → Your Orders) and match the charges to them: each download gets the order number and what was in it as its memo.
      </Text>
      <Group align="flex-end" gap="sm" wrap="wrap">
        <Select label="Card account" data={cards.map((a) => ({ value: a.id, label: a.full_name }))} value={account} onChange={setAccount} clearable searchable placeholder="Any" w={260} />
        <FileInput label="Order history CSV" placeholder="Retail.OrderHistory.1.csv" accept=".csv,text/csv" value={file} onChange={(f) => void load(f)} clearable w={300} data-testid="amazon-file" />
      </Group>
      {busy && <Text size="sm" c="dimmed">Matching…</Text>}
      {error && <Alert color="red">{error}</Alert>}
      {preview && (
        <>
          <Text size="sm">
            {preview.orders.length} orders in the file · {preview.matches.length} charge{preview.matches.length === 1 ? '' : 's'} matched.
          </Text>
          <Table fz="sm" withTableBorder>
            <Table.Thead>
              <Table.Tr>
                <Table.Th>Order</Table.Th>
                <Table.Th>Shipped</Table.Th>
                <Table.Th>Download</Table.Th>
                <Table.Th>Memo it will get</Table.Th>
              </Table.Tr>
            </Table.Thead>
            <Table.Tbody>
              {preview.matches.map((m) => (
                <Table.Tr key={m.feed_txn_id}>
                  <Table.Td>
                    <a href={m.link} target="_blank" rel="noreferrer">
                      {m.order_id}
                    </a>
                  </Table.Td>
                  <Table.Td>{m.ship_date ? formatDateShort(m.ship_date) : 'whole order'}</Table.Td>
                  <Table.Td>{accounts.get(preview.orders.find((o) => o.order_id === m.order_id)?.payment ?? '')?.full_name ?? m.feed_txn_id.slice(0, 8)}</Table.Td>
                  <Table.Td>{m.memo}</Table.Td>
                </Table.Tr>
              ))}
            </Table.Tbody>
          </Table>
          <Group justify="flex-end">
            <Button loading={busy} onClick={() => void apply()} disabled={!preview.matches.length}>
              Apply to {preview.matches.length} download{preview.matches.length === 1 ? '' : 's'}
            </Button>
          </Group>
        </>
      )}
    </Stack>
  );
}
