// FIXTURE ONLY: the administrative side of the server in memory — users and
// their company access, enrolment links, my passkeys/devices/sessions, the
// server settings document with its secrets, updater, tunnel and backup
// scheduler, per-company backups, attachments, and the QuickBooks import
// (settings, a sync that runs for a few seconds with progress, a last report
// and a parity report). Shapes follow plans/api.md and plans/server-ops.md;
// the real server is the truth.

import { ApiError } from '../errors';
import { addDays, type IsoDate } from '~/lib/dates';
import type {
  Account,
  AccountBalance,
  AttachmentMeta,
  AttachmentQuery,
  AttachmentRecord,
  BackupList,
  BackupManifest,
  BackupRecord,
  CompanyBackupsStatus,
  CompanyId,
  DeviceKeyInfo,
  EnrolmentInfo,
  ImportSettings,
  ImportStatus,
  JobKind,
  MintedEnrolmentLink,
  NewEnrolment,
  ParityBody,
  ParityReport,
  ParityRow,
  ParityStatus,
  PasskeyInfo,
  RestoreRequest,
  RunningJob,
  Restored,
  Role,
  SecretInfo,
  ServerBackupsStatus,
  ServerSettings,
  ServerView,
  SessionInfo,
  SyncBody,
  SyncProgress,
  SyncReport,
  TunnelUpdate,
  TunnelView,
  UpdateInstalled,
  UpdateView,
  User,
  UserDetail,
  UserUpdate,
  VerifyReport,
} from '../types';

export interface AdminHost {
  today: IsoDate;
  companyId: CompanyId;
  companyName(): string;
  accounts(): Account[];
  balances(): AccountBalance[];
  emit(kind: string, id: string, action: string): void;
}

/** The signed-in demo user (matches `FixtureApiClient.session()`). */
export const FIXTURE_USER_ID = 'fixture-user';
const FIXTURE_SESSION_ID = 'session-this-browser';
const VERSION = '0.4.1';

const hoursAgo = (h: number) => new Date(Date.now() - h * 3_600_000).toISOString();
const daysAgo = (d: number) => hoursAgo(d * 24);

interface StoredAttachment {
  meta: AttachmentMeta;
  blob: Blob;
  url?: string;
}

export class FixtureAdmin {
  private users: User[] = [];
  private access = new Map<string, Role>(); // `${userId}:${companyId}`
  private passkeys = new Map<string, PasskeyInfo[]>();
  private devices = new Map<string, DeviceKeyInfo[]>();
  private sessions: SessionInfo[] = [];
  private enrollments: EnrolmentInfo[] = [];
  private settings: ServerSettings;
  private secretList: SecretInfo[] = [];
  private updateView: UpdateView;
  private tunnelView: TunnelView;
  private backupRecords: BackupRecord[] = [];
  private lastBackup: { started_at: string; finished_at: string; record: BackupRecord | null; error: string | null } | null = null;
  private attachmentStore = new Map<string, StoredAttachment>();
  private qbd: ImportSettings;
  /** The Web Connector password is a secret: only whether it is set. */
  private qbwcPasswordSet = false;
  /** The one job (sync or parity) running for this company, or none. */
  private qbdJob: RunningJob | null = null;
  private lastSync: SyncReport | null;
  private parity: ParityReport | null = null;
  private qbdTimers: ReturnType<typeof setTimeout>[] = [];
  /** Long-poll waiters: the status ones wake on a progress step too, the parity ones only when the job ends. */
  private qbdWaiters: Array<{ onProgress: boolean; wake(): void }> = [];

  constructor(private host: AdminHost) {
    this.seedPeople();
    this.settings = {
      public_urls: ['https://books.example.com'],
      backups: {
        schedule: { enabled: true, every: null, daily_at: '02:30:00' },
        destinations: [{ Local: { dir: '/var/lib/librebooks/backups' } }],
        encryption: { Passphrase: { passphrase_ref: 'backup-passphrase', work_factor: 18 } },
        retention: { keep_last: 7, daily: 14, weekly: 8, monthly: 12, yearly: 5 },
        compression_level: 9,
        time_zone: 'America/Los_Angeles',
        work_dir: null,
      },
      updates: { check: true, auto_install: false, channel: 'Stable', interval: 'PT6H' },
      tunnel: { enabled: false, token_ref: 'tunnel-token', public_hostname: null, cloudflared_path: null, auto_download: true },
      access: null,
      feeds: { auto_sync: true, every: 'PT12H' },
    };
    this.secretList = [{ name: 'backup-passphrase', updated_at: daysAgo(40) }];
    this.updateView = {
      config: this.settings.updates,
      status: {
        current: VERSION,
        can_install: true,
        install_kind: 'Binary',
        advice: null,
        last_check: hoursAgo(3),
        available: { version: '0.4.2', tag: 'v0.4.2', page: 'https://github.com/cinderblock/LibreBooks/releases/tag/v0.4.2', asset_name: 'librebooks-x86_64-linux.tar.gz', asset_url: 'https://github.com/cinderblock/LibreBooks/releases/download/v0.4.2/librebooks-x86_64-linux.tar.gz' },
        phase: 'Idle',
      },
      unavailable: null,
    };
    this.tunnelView = {
      enabled: false,
      public_hostname: null,
      public_url: null,
      token_ref: 'tunnel-token',
      token_set: false,
      tunnel_id: null,
      cloudflared_path: null,
      auto_download: true,
      cloudflared: null,
      preparing_since: null,
      status: { state: { state: 'stopped' }, last_log_lines: [], version: null, restarts: 0, tunnel_id: null, public_url: null },
      problems: [],
      restart_required: [],
    };
    this.seedBackups();
    this.seedAttachments();
    this.qbd = {
      transport: 'bridge',
      bridge: { url: 'http://192.168.1.40:8750', key_ref: `secret:${this.bridgeKeyName()}`, delay_ms: 250 },
      qbwc: { user_name: 'librebooks', app_url: null, run_every_minutes: 60, owner_id: '90a44fb5-33d9-4815-ac85-bc87a7e7d1eb', file_id: '57f3b9b6-86f1-4fcc-b1ee-566de1813d20' },
      sdk: { file_mode: 'dont_care' },
      company_file: '',
    };
    this.lastSync = this.sampleReport(daysAgo(2), {});
  }

  // ---- seed -------------------------------------------------------------------

  private seedPeople() {
    const co = this.host.companyId;
    const people: Array<[string, string, Role, Role | null, string | null]> = [
      [FIXTURE_USER_ID, 'Demo User', 'admin', 'admin', null],
      ['user-maya', 'Maya Okafor', 'bookkeeper', 'bookkeeper', null],
      ['user-tom', 'Tom Castellano (CPA)', 'viewer', 'viewer', null],
      ['user-old', 'Former Bookkeeper', 'bookkeeper', null, daysAgo(90)],
    ];
    for (const [id, name, role, coRole, disabled] of people) {
      this.users.push({ id, display_name: name, role, created_at: daysAgo(200), disabled_at: disabled });
      if (coRole) this.access.set(`${id}:${co}`, coRole);
    }
    this.passkeys.set(FIXTURE_USER_ID, [
      { id: 'pk-1', name: 'MacBook Touch ID', rp_id: 'books.example.com', created_at: daysAgo(200), last_used_at: hoursAgo(1), backup_eligible: true, backed_up: true },
      { id: 'pk-2', name: 'YubiKey 5C', rp_id: 'books.example.com', created_at: daysAgo(120), last_used_at: daysAgo(12), backup_eligible: false, backed_up: false },
    ]);
    this.passkeys.set('user-maya', [{ id: 'pk-3', name: 'Pixel 8', rp_id: 'books.example.com', created_at: daysAgo(100), last_used_at: daysAgo(1), backup_eligible: true, backed_up: true }]);
    this.devices.set(FIXTURE_USER_ID, [{ id: 'dev-1', user_id: FIXTURE_USER_ID, name: 'Shop kiosk (Raspberry Pi)', created_at: daysAgo(60), last_used_at: hoursAgo(5) }]);
    this.sessions = [
      { id: FIXTURE_SESSION_ID, kind: 'passkey', label: 'Chrome on Windows', ip: '192.168.1.22', created_at: hoursAgo(2), last_seen_at: hoursAgo(0), expires_at: addDays(this.host.today, 14), current: true },
      { id: 'session-phone', kind: 'passkey', label: 'Safari on iPhone', ip: '10.0.0.8', created_at: daysAgo(3), last_seen_at: daysAgo(1), expires_at: addDays(this.host.today, 11), current: false },
      { id: 'session-kiosk', kind: 'device', label: 'Shop kiosk (Raspberry Pi)', ip: '192.168.1.60', created_at: daysAgo(10), last_seen_at: hoursAgo(5), expires_at: addDays(this.host.today, 4), current: false },
    ];
    this.enrollments = [
      { id: 'enr-1', purpose: 'new_user', role: 'bookkeeper', user_id: null, company_id: co, display_name: 'Priya Natarajan', created_by: FIXTURE_USER_ID, created_at: hoursAgo(0.2), expires_at: new Date(Date.now() + 13 * 60_000).toISOString() },
    ];
  }

  private manifest(createdAt: string, size: number): BackupManifest {
    const id = crypto.randomUUID();
    return {
      format: 1,
      app_version: VERSION,
      company_name: this.host.companyName(),
      company_slug: 'juniper-hill-woodworks',
      schema_version: 12,
      created_at: createdAt,
      id,
      artifact: `${id}.lbk.age`,
      database_size: size,
      database_sha256: id.replace(/-/g, '').padEnd(64, '0'),
      stored_size: Math.round(size * 0.31),
      stored_sha256: id.replace(/-/g, '').split('').reverse().join('').padEnd(64, '1'),
      compression: 'zstd:9',
      encryption: 'passphrase',
    };
  }

  private seedBackups() {
    for (let i = 1; i <= 5; i++) {
      const m = this.manifest(daysAgo(i), 5_300_000 + i * 12_000);
      this.backupRecords.push({ manifest: m, destinations: [{ destination: 'Local', stored: true, error: null, pruned: i === 1 ? ['2026-09-10T02:30:00Z'] : [] }] });
    }
    const last = this.backupRecords[0]!;
    this.lastBackup = { started_at: last.manifest.created_at, finished_at: last.manifest.created_at, record: last, error: null };
  }

  private seedAttachments() {
    const receipt = new Blob([`<svg xmlns="http://www.w3.org/2000/svg" width="320" height="200"><rect width="320" height="200" fill="#fff8e1"/><text x="20" y="40" font-family="monospace" font-size="18">Rockler Woodworking</text><text x="20" y="80" font-family="monospace" font-size="14">Walnut 8/4 FAS  3 bf  $ 54.00</text><text x="20" y="110" font-family="monospace" font-size="14">Titebond III      1   $ 11.49</text><text x="20" y="160" font-family="monospace" font-size="16">TOTAL             $ 65.49</text></svg>`], { type: 'image/svg+xml' });
    const stmt = new Blob(['Statement — Hilltop Community Bank\nClosing balance: see the register.\n'], { type: 'text/plain' });
    this.put('receipt-rockler.svg', receipt, null, daysAgo(1));
    this.put('bank-statement-sept.txt', stmt, null, daysAgo(6));
  }

  private put(filename: string, blob: Blob, record: AttachmentRecord, createdAt = new Date().toISOString()): AttachmentMeta {
    const id = crypto.randomUUID();
    // The media type without parameters (`text/plain;charset=utf-8` → `text/plain`), as the server records it.
    const mime = (blob.type || 'application/octet-stream').split(';')[0]!.trim();
    const meta: AttachmentMeta = { id, record_kind: record?.record_kind ?? null, record_id: record?.record_id ?? null, filename, mime, size: blob.size, sha256: id.replace(/-/g, '').padEnd(64, 'a'), created_at: createdAt, created_by: 'Demo User' };
    this.attachmentStore.set(id, { meta, blob });
    return meta;
  }

  // ---- users ------------------------------------------------------------------

  listUsers(): User[] {
    return [...this.users].sort((a, b) => a.display_name.localeCompare(b.display_name));
  }

  user(id: string): UserDetail {
    const user = this.users.find((u) => u.id === id);
    if (!user) throw new ApiError(404, 'not_found', 'No such user');
    const companies = [...this.access.entries()].filter(([k]) => k.startsWith(`${id}:`)).map(([k, role]) => ({ company_id: k.slice(id.length + 1), role }));
    return { user, companies, passkeys: this.passkeys.get(id) ?? [], devices: this.devices.get(id) ?? [] };
  }

  updateUser(id: string, update: UserUpdate): User {
    const i = this.users.findIndex((u) => u.id === id);
    if (i < 0) throw new ApiError(404, 'not_found', 'No such user');
    const u = this.users[i]!;
    if (id === FIXTURE_USER_ID && (update.disabled || (update.role && update.role !== 'admin'))) {
      throw new ApiError(400, 'invalid', "You can't disable or demote yourself; have another admin do it.");
    }
    const next: User = {
      ...u,
      display_name: update.display_name?.trim() || u.display_name,
      role: update.role ?? u.role,
      disabled_at: update.disabled === undefined ? u.disabled_at : update.disabled ? (u.disabled_at ?? new Date().toISOString()) : null,
    };
    if (update.disabled) this.sessions = this.sessions.filter((s) => !(s.kind !== 'local' && s.label?.includes(u.display_name)));
    this.users[i] = next;
    this.host.emit('user', id, 'updated');
    return next;
  }

  deleteUser(id: string) {
    if (id === FIXTURE_USER_ID) throw new ApiError(400, 'invalid', "You can't delete yourself.");
    const i = this.users.findIndex((u) => u.id === id);
    if (i < 0) throw new ApiError(404, 'not_found', 'No such user');
    this.users.splice(i, 1);
    for (const k of [...this.access.keys()]) if (k.startsWith(`${id}:`)) this.access.delete(k);
    this.passkeys.delete(id);
    this.devices.delete(id);
    this.host.emit('user', id, 'deleted');
  }

  grantAccess(userId: string, co: CompanyId, role: Role) {
    if (!this.users.some((u) => u.id === userId)) throw new ApiError(404, 'not_found', 'No such user');
    this.access.set(`${userId}:${co}`, role);
    this.host.emit('user', userId, 'updated');
  }

  revokeAccess(userId: string, co: CompanyId) {
    this.access.delete(`${userId}:${co}`);
    this.host.emit('user', userId, 'updated');
  }

  // ---- enrolment links --------------------------------------------------------

  listEnrollments(): EnrolmentInfo[] {
    const now = new Date().toISOString();
    this.enrollments = this.enrollments.filter((e) => e.expires_at > now);
    return [...this.enrollments].sort((a, b) => b.created_at.localeCompare(a.created_at));
  }

  createEnrollment(body: NewEnrolment): MintedEnrolmentLink {
    if (body.purpose === 'first_admin') throw new ApiError(400, 'invalid', 'first_admin links only come from bootstrap');
    const ttl = Math.min(Math.max(body.ttl_minutes ?? 15, 1), 24 * 60);
    const id = crypto.randomUUID();
    const token = crypto.randomUUID().replace(/-/g, '') + crypto.randomUUID().replace(/-/g, '').slice(0, 11);
    const expires_at = new Date(Date.now() + ttl * 60_000).toISOString();
    const info: EnrolmentInfo = {
      id,
      purpose: body.purpose,
      role: body.purpose === 'new_user' ? (body.role ?? 'bookkeeper') : null,
      user_id: body.purpose === 'add_device' ? (body.user_id ?? FIXTURE_USER_ID) : null,
      company_id: body.purpose === 'new_user' ? (body.company_id ?? null) : null,
      display_name: body.display_name?.trim() || null,
      created_by: FIXTURE_USER_ID,
      created_at: new Date().toISOString(),
      expires_at,
    };
    this.enrollments.push(info);
    this.host.emit('enrolment', id, 'created');
    const origin = typeof window !== 'undefined' ? window.location.origin : 'https://books.example.com';
    return { id, token, purpose: body.purpose, expires_at, url: `${origin}/enroll#token=${token}` };
  }

  revokeEnrollment(id: string) {
    const i = this.enrollments.findIndex((e) => e.id === id);
    if (i < 0) throw new ApiError(404, 'not_found', 'No such enrolment link (it may have expired)');
    this.enrollments.splice(i, 1);
    this.host.emit('enrolment', id, 'deleted');
  }

  // ---- my account -------------------------------------------------------------

  myPasskeys(): PasskeyInfo[] {
    return this.passkeys.get(FIXTURE_USER_ID) ?? [];
  }

  renamePasskey(id: string, name: string) {
    const list = this.passkeys.get(FIXTURE_USER_ID) ?? [];
    const pk = list.find((p) => p.id === id);
    if (!pk) throw new ApiError(404, 'not_found', 'No such passkey');
    if (!name.trim()) throw new ApiError(400, 'invalid', 'A name is required');
    pk.name = name.trim();
  }

  deletePasskey(id: string) {
    const list = this.passkeys.get(FIXTURE_USER_ID) ?? [];
    const i = list.findIndex((p) => p.id === id);
    if (i < 0) throw new ApiError(404, 'not_found', 'No such passkey');
    if (list.length === 1 && (this.devices.get(FIXTURE_USER_ID) ?? []).length === 0) {
      throw new ApiError(409, 'conflict', "That's your last passkey; add another before removing it, or you'd be locked out.", 'last_credential');
    }
    list.splice(i, 1);
  }

  myDevices(): DeviceKeyInfo[] {
    return this.devices.get(FIXTURE_USER_ID) ?? [];
  }

  revokeDevice(id: string) {
    const list = this.devices.get(FIXTURE_USER_ID) ?? [];
    const i = list.findIndex((d) => d.id === id);
    if (i < 0) throw new ApiError(404, 'not_found', 'No such device');
    const [gone] = list.splice(i, 1);
    // A revoked key's sessions end with it.
    this.sessions = this.sessions.filter((s) => !(s.kind === 'device' && s.label === gone!.name));
  }

  mySessions(): SessionInfo[] {
    return [...this.sessions].sort((a, b) => Number(b.current) - Number(a.current) || b.last_seen_at.localeCompare(a.last_seen_at));
  }

  revokeSession(id: string) {
    const i = this.sessions.findIndex((s) => s.id === id);
    if (i < 0) throw new ApiError(404, 'not_found', 'No such session');
    this.sessions.splice(i, 1);
  }

  // ---- server -----------------------------------------------------------------

  serverView(): ServerView {
    return {
      version: VERSION,
      data_dir: '/var/lib/librebooks',
      settings: this.settings,
      secrets: this.secretList,
      update: this.updateView,
      tunnel: this.tunnelView,
      backups: [{ company_id: this.host.companyId, company_name: this.host.companyName(), status: this.scheduler() }],
      restart_required: this.tunnelView.restart_required,
      running: true,
    };
  }

  private scheduler() {
    const enabled = this.settings.backups.schedule.enabled;
    const next = enabled ? `${addDays(this.host.today, 1)}T${this.settings.backups.schedule.daily_at ?? '02:30:00'}` : null;
    return { running: enabled, next_run: next, last_success_at: this.lastBackup?.error ? null : (this.lastBackup?.finished_at ?? null), last: this.lastBackup };
  }

  putServer(settings: ServerSettings): ServerView {
    const problems: string[] = [];
    for (const u of settings.public_urls) {
      try {
        const parsed = new URL(u);
        if (!/^https?:$/.test(parsed.protocol)) problems.push(`${u}: not an http(s) URL`);
      } catch {
        problems.push(`${u}: not a URL`);
      }
    }
    const names = new Set(this.secretList.map((s) => s.name));
    const enc = settings.backups.encryption;
    if (typeof enc === 'object' && 'Passphrase' in enc && !names.has(enc.Passphrase.passphrase_ref)) problems.push(`backups.encryption: no secret named "${enc.Passphrase.passphrase_ref}"`);
    if (settings.tunnel.enabled && !names.has(settings.tunnel.token_ref)) problems.push(`tunnel: no secret named "${settings.tunnel.token_ref}" (set the tunnel token first)`);
    if (settings.backups.schedule.enabled && !settings.backups.schedule.every && !settings.backups.schedule.daily_at) problems.push('backups.schedule: choose an interval or a daily time');
    if (settings.backups.schedule.enabled && settings.backups.destinations.length === 0) problems.push('backups: add a destination or turn the schedule off');
    if (problems.length) throw new ApiError(400, 'invalid', 'Fix these first', { problems });
    const restart: string[] = [];
    if (settings.public_urls.join() !== this.settings.public_urls.join()) restart.push('public_urls');
    this.settings = structuredClone(settings);
    this.updateView = { ...this.updateView, config: this.settings.updates };
    this.tunnelView = { ...this.tunnelView, enabled: settings.tunnel.enabled, public_hostname: settings.tunnel.public_hostname, token_ref: settings.tunnel.token_ref, cloudflared_path: settings.tunnel.cloudflared_path, auto_download: settings.tunnel.auto_download, restart_required: [...new Set([...this.tunnelView.restart_required, ...restart])] };
    this.host.emit('server', 'settings', 'updated');
    return this.serverView();
  }

  secrets(): SecretInfo[] {
    return [...this.secretList].sort((a, b) => a.name.localeCompare(b.name));
  }

  putSecret(name: string, value: string) {
    if (!/^[A-Za-z0-9._-]{1,128}$/.test(name)) throw new ApiError(400, 'invalid', 'Secret names are letters, digits, . _ and -', { field: 'name' });
    if (!value) throw new ApiError(400, 'invalid', 'A value is required', { field: 'value' });
    const now = new Date().toISOString();
    const i = this.secretList.findIndex((s) => s.name === name);
    if (i < 0) this.secretList.push({ name, updated_at: now });
    else this.secretList[i] = { name, updated_at: now };
    if (name === this.settings.tunnel.token_ref) this.tunnelView = { ...this.tunnelView, token_set: true };
    this.host.emit('server', 'secrets', 'updated');
  }

  deleteSecret(name: string) {
    const i = this.secretList.findIndex((s) => s.name === name);
    if (i < 0) throw new ApiError(404, 'not_found', 'No such secret');
    const enc = this.settings.backups.encryption;
    const used = (typeof enc === 'object' && 'Passphrase' in enc && enc.Passphrase.passphrase_ref === name) || (this.settings.tunnel.enabled && this.settings.tunnel.token_ref === name);
    if (used) throw new ApiError(409, 'in_use', 'The settings still refer to this secret');
    this.secretList.splice(i, 1);
    if (name === this.settings.tunnel.token_ref) this.tunnelView = { ...this.tunnelView, token_set: false };
    this.host.emit('server', 'secrets', 'updated');
  }

  update(): UpdateView {
    return this.updateView;
  }

  checkUpdate(): UpdateView {
    if (this.updateView.unavailable) throw new ApiError(409, 'unavailable', this.updateView.unavailable);
    this.updateView = { ...this.updateView, status: { ...this.updateView.status!, last_check: new Date().toISOString() } };
    return this.updateView;
  }

  installUpdate(restart: boolean): UpdateInstalled {
    const st = this.updateView.status;
    if (!st?.available) throw new ApiError(409, 'up_to_date', `Already on ${st?.current ?? VERSION}`);
    const installed = { version: st.available.version, path: `/opt/librebooks/${st.available.asset_name}` };
    this.updateView = { ...this.updateView, status: { ...st, available: null, phase: restart ? 'Restarting' : 'Installed', advice: restart ? null : `Version ${installed.version} starts on the next restart.` } };
    return { installed, restarting: restart };
  }

  tunnel(): TunnelView {
    return this.tunnelView;
  }

  putTunnel(update: TunnelUpdate): TunnelView {
    const t = this.settings.tunnel;
    if (update.token !== undefined) {
      if (update.token === '') throw new ApiError(400, 'invalid', 'A tunnel token is required', { field: 'token' });
      // "cloudflared service install <token>" pasted whole is accepted; the token is its last word.
      const token = update.token.trim().split(/\s+/).pop()!;
      if (token.length < 20) throw new ApiError(400, 'invalid', "That doesn't look like a Cloudflare tunnel token", { field: 'token' });
      this.putSecret(t.token_ref, token);
    }
    const enabled = update.enabled ?? t.enabled;
    if (enabled && !this.tunnelView.token_set) throw new ApiError(400, 'invalid', 'Set the tunnel token before turning the tunnel on', { field: 'token' });
    const hostname = update.public_hostname === undefined ? t.public_hostname : update.public_hostname || null;
    this.settings = { ...this.settings, tunnel: { ...t, enabled, public_hostname: hostname, cloudflared_path: update.cloudflared_path === undefined ? t.cloudflared_path : update.cloudflared_path || null, auto_download: update.auto_download ?? t.auto_download } };
    const restart = hostname !== t.public_hostname ? ['tunnel.public_hostname'] : [];
    this.tunnelView = {
      ...this.tunnelView,
      enabled,
      public_hostname: hostname,
      public_url: enabled && hostname ? `https://${hostname}` : null,
      cloudflared_path: this.settings.tunnel.cloudflared_path,
      auto_download: this.settings.tunnel.auto_download,
      cloudflared: enabled ? { located: { path: '/usr/local/bin/cloudflared', origin: 'downloaded' }, version: '2026.9.1' } : this.tunnelView.cloudflared,
      status: enabled
        ? { state: { state: 'connected', connections: 4 }, last_log_lines: ['INF Registered tunnel connection connIndex=0 location=sjc01', 'INF Registered tunnel connection connIndex=1 location=lax02'], version: '2026.9.1', restarts: 0, tunnel_id: 'a1b2c3d4-0000-4000-8000-tunnel000001', public_url: hostname ? `https://${hostname}` : null }
        : { state: { state: 'stopped' }, last_log_lines: [], version: null, restarts: 0, tunnel_id: null, public_url: null },
      problems: enabled && !hostname ? ['No public hostname: passkeys need one to be registered against'] : [],
      restart_required: [...new Set([...this.tunnelView.restart_required, ...restart])],
    };
    this.host.emit('server', 'tunnel', 'updated');
    return this.tunnelView;
  }

  serverBackupsStatus(): ServerBackupsStatus {
    return { enabled: this.settings.backups.schedule.enabled, running: false, companies: this.serverView().backups };
  }

  // ---- company backups --------------------------------------------------------

  backups(): BackupList {
    return { backups: this.backupRecords.map((r) => ({ destination: 0, destination_name: 'Local', manifest: r.manifest })), errors: [] };
  }

  backupsStatus(): CompanyBackupsStatus {
    return { enabled: this.settings.backups.schedule.enabled, destinations: ['Local'], scheduler: this.scheduler() };
  }

  runBackup(): BackupRecord {
    const started = new Date().toISOString();
    const m = this.manifest(started, 5_400_000 + this.backupRecords.length * 9_000);
    const record: BackupRecord = { manifest: m, destinations: [{ destination: 'Local', stored: true, error: null, pruned: [] }] };
    this.backupRecords.unshift(record);
    this.lastBackup = { started_at: started, finished_at: new Date().toISOString(), record, error: null };
    this.host.emit('backup', m.id, 'created');
    return record;
  }

  private backup(id: string): BackupRecord {
    const r = this.backupRecords.find((b) => b.manifest.id === id);
    if (!r) throw new ApiError(404, 'not_found', 'No such backup');
    return r;
  }

  verifyBackup(id: string, unlock: RestoreRequest = {}): VerifyReport {
    this.backup(id);
    // The fixture's backups are passphrase-encrypted: contents can only be checked with it.
    const unlocked = !!unlock.passphrase || (unlock.identities?.length ?? 0) > 0;
    return { checksum_ok: true, contents_checked: unlocked, problems: unlocked ? [] : ['Not unlocked: only the checksum was checked (give the passphrase to open it)'] };
  }

  restoreBackup(id: string, body: RestoreRequest): Restored {
    const r = this.backup(id);
    if (!body.passphrase && !(body.identities?.length ?? 0)) throw new ApiError(400, 'invalid', 'This backup is encrypted; give the passphrase to restore it', { field: 'passphrase' });
    const name = body.name?.trim() || `${r.manifest.company_name} (restored from ${r.manifest.created_at.slice(0, 10)})`;
    return { id: crypto.randomUUID(), name, from: r.manifest };
  }

  // ---- attachments ------------------------------------------------------------

  /** `all` → every document; a record → its files; otherwise the inbox (as the server's route chooses). */
  attachments(query: AttachmentQuery = {}): AttachmentMeta[] {
    const all = [...this.attachmentStore.values()].map((a) => a.meta);
    const rows = query.all ? all : query.record_kind && query.record_id ? all.filter((a) => a.record_kind === query.record_kind && a.record_id === query.record_id) : all.filter((a) => !a.record_kind);
    return rows.sort((a, b) => b.created_at.localeCompare(a.created_at));
  }

  upload(file: File, record?: AttachmentRecord): AttachmentMeta {
    if (file.size === 0) throw new ApiError(400, 'invalid', 'The file is empty');
    if (file.size > 25 * 1024 * 1024) throw new ApiError(400, 'invalid', 'Attachments are limited to 25 MB');
    const meta = this.put(file.name || 'attachment', file, record ?? null);
    this.host.emit('attachment', meta.id, 'created');
    return meta;
  }

  attach(id: string, record: AttachmentRecord): AttachmentMeta {
    const a = this.attachmentStore.get(id);
    if (!a) throw new ApiError(404, 'not_found', 'No such attachment');
    a.meta = { ...a.meta, record_kind: record?.record_kind ?? null, record_id: record?.record_id ?? null };
    this.host.emit('attachment', id, 'updated');
    return a.meta;
  }

  deleteAttachment(id: string) {
    const a = this.attachmentStore.get(id);
    if (!a) throw new ApiError(404, 'not_found', 'No such attachment');
    if (a.url) URL.revokeObjectURL(a.url);
    this.attachmentStore.delete(id);
    this.host.emit('attachment', id, 'deleted');
  }

  attachmentUrl(id: string): string {
    const a = this.attachmentStore.get(id);
    if (!a) return '';
    a.url ??= URL.createObjectURL(a.blob);
    return a.url;
  }

  // ---- QuickBooks import ------------------------------------------------------

  private bridgeKeyName(): string {
    return `qbd.${this.host.companyId}.bridge-key`;
  }

  /** Mirrors lb-qbd's `ImportSettings::problems()`: what must be fixed before a job can run. */
  private qbdProblems(s: ImportSettings): string[] {
    const out: string[] = [];
    const isUrl = (u: string) => {
      try {
        new URL(u);
        return true;
      } catch {
        return false;
      }
    };
    if (s.transport === 'bridge') {
      if (!s.bridge.url.trim()) out.push('the bridge URL is empty');
      else if (!isUrl(s.bridge.url)) out.push(`the bridge URL ${JSON.stringify(s.bridge.url)} is invalid`);
      if (!s.bridge.key_ref) out.push("the bridge's API key hasn't been set");
    } else if (s.transport === 'qbwc') {
      if (!s.qbwc.user_name.trim()) out.push('the Web Connector user name is empty');
      if (s.qbwc.app_url && !isUrl(s.qbwc.app_url)) out.push(`the Web Connector app URL ${JSON.stringify(s.qbwc.app_url)} is invalid`);
      if (s.qbwc.run_every_minutes === 0) out.push("the Web Connector schedule can't be every 0 minutes");
    }
    return out;
  }

  /** `GET /import/qbd` (and `…/status`). */
  qbdStatus(): ImportStatus {
    return {
      settings: structuredClone(this.qbd),
      bridge_key_set: this.qbd.bridge.key_ref !== null,
      qbwc_password_set: this.qbwcPasswordSet,
      sdk_available: false,
      running: this.qbdJob ? { ...this.qbdJob } : null,
      last_sync: this.lastSync,
      last_parity: this.parity,
      last_error: null,
    };
  }

  /**
   * `?wait=N` on `…/status`, as the server's `wait_for_change`: resolves on the
   * running job's next progress step or its end (so a long-polled status
   * follows the progress bar), or after `wait` seconds (at most 120, as the
   * server caps it). At once when nothing runs.
   */
  qbdWaitChange(wait: number): Promise<void> {
    return this.qbdWait(wait, true);
  }

  /** `?wait=N` on `…/parity`, as the server's `wait_for_idle`: resolves when no job runs, or after `wait` seconds. */
  qbdWaitIdle(wait: number): Promise<void> {
    return this.qbdWait(wait, false);
  }

  private qbdWait(wait: number, onProgress: boolean): Promise<void> {
    if (!this.qbdJob || wait <= 0) return Promise.resolve();
    return new Promise<void>((resolve) => {
      const waiter = {
        onProgress,
        wake: () => {
          clearTimeout(timer);
          this.qbdWaiters = this.qbdWaiters.filter((w) => w !== waiter);
          resolve();
        },
      };
      const timer = setTimeout(waiter.wake, Math.min(wait, 120) * 1000);
      this.qbdWaiters.push(waiter);
    });
  }

  /** The running job reported progress: the status long-polls answer with it. */
  private qbdProgressed() {
    for (const w of [...this.qbdWaiters]) if (w.onProgress) w.wake();
  }

  private qbdFinished() {
    this.qbdJob = null;
    for (const w of [...this.qbdWaiters]) w.wake();
  }

  /**
   * `PUT /import/qbd`, as the server does it: the `.qwc` ids can't change, an
   * empty user name keeps the old one, a given bridge key becomes the secret
   * reference (and is never returned), and problems are a 400 listing them.
   */
  putQbdSettings(s: ImportSettings, bridgeKey?: string | null): ImportStatus {
    const next: ImportSettings = structuredClone(s);
    next.qbwc.owner_id = this.qbd.qbwc.owner_id;
    next.qbwc.file_id = this.qbd.qbwc.file_id;
    next.qbwc.user_name = next.qbwc.user_name.trim() || this.qbd.qbwc.user_name;
    next.bridge.url = next.bridge.url.trim();
    if (bridgeKey?.trim()) next.bridge.key_ref = `secret:${this.bridgeKeyName()}`;
    else if (!next.bridge.key_ref && this.qbd.bridge.key_ref) next.bridge.key_ref = this.qbd.bridge.key_ref;
    const problems = this.qbdProblems(next);
    if (problems.length) throw new ApiError(400, 'invalid', problems.join('; '), { problems });
    // No change event: the server emits `import` events for jobs only (plans/api.md).
    this.qbd = next;
    return this.qbdStatus();
  }

  /**
   * One job per company at a time; the settings must be sound; the SDK needs a
   * Windows build with it. Announces `import` / `started` with the job kind as
   * the id, as the server does; `progress` and `finished` follow.
   */
  private startJob(kind: JobKind): RunningJob {
    if (this.qbdJob) throw new ApiError(409, 'running', `a ${this.qbdJob.kind} job is already running for this company`);
    const problems = this.qbdProblems(this.qbd);
    if (problems.length) throw new ApiError(400, 'invalid', problems.join('; '), { problems });
    if (this.qbd.transport === 'sdk') throw new ApiError(501, 'unsupported', 'the QuickBooks SDK transport needs LibreBooks running on the Windows machine that has QuickBooks (this build has no SDK support)');
    this.qbdJob = {
      kind,
      transport: this.qbd.transport,
      started_at: new Date().toISOString(),
      progress: null,
      percent: 1,
      waiting_for_web_connector: this.qbd.transport === 'qbwc',
      web_connector_connected: false,
    };
    this.host.emit('import', kind, 'started');
    return this.qbdJob;
  }

  /** A sync runs for about two seconds, stepping through the phases, then lands a report (202). */
  qbdSync(opts: Partial<SyncBody> = {}): ImportStatus {
    const job = this.startJob('sync');
    const steps: SyncProgress[] = [
      { entity: 'Account', phase: 'Listing', done: 0, total: 56 },
      { entity: 'Account', phase: 'Importing', done: 56, total: 56 },
      { entity: 'Customer', phase: 'Importing', done: 9, total: 18 },
      { entity: 'Invoice', phase: 'Importing', done: 3, total: 24 },
      { entity: 'Check', phase: 'Importing', done: 30, total: 41 },
      { entity: 'Deposit', phase: opts.sweep ? 'Sweeping' : 'Checking', done: 6, total: 6 },
    ];
    steps.forEach((p, i) => {
      this.qbdTimers.push(
        setTimeout(() => {
          // Over the Web Connector the first poll arrives with the first step.
          job.waiting_for_web_connector = false;
          job.web_connector_connected = job.transport === 'qbwc';
          job.progress = p;
          job.percent = Math.min(99, Math.round((100 * (i + 1)) / (steps.length + 1)));
          this.qbdProgressed();
          this.host.emit('import', 'sync', 'progress');
        }, 300 * (i + 1)),
      );
    });
    this.qbdTimers.push(
      setTimeout(() => {
        this.lastSync = this.sampleReport(job.started_at, opts);
        this.qbdFinished();
        // `finished` (not `progress`): the books changed, so registers and reports are stale.
        this.host.emit('import', 'sync', 'finished');
      }, 300 * (steps.length + 1)),
    );
    return this.qbdStatus();
  }

  /** `GET /import/qbd/parity`. */
  qbdParity(): ParityStatus {
    return { running: this.qbdJob?.kind === 'parity', job: this.qbdJob ? { ...this.qbdJob } : null, report: this.parity, error: null };
  }

  /** A parity check asks QuickBooks for every balance; here it answers in a moment (202). */
  qbdParityRun(opts: Partial<ParityBody> = {}): ImportStatus {
    this.startJob('parity');
    this.qbdTimers.push(
      setTimeout(() => {
        this.parity = this.buildParity(opts);
        this.qbdFinished();
        this.host.emit('import', 'parity', 'finished');
      }, 400),
    );
    return this.qbdStatus();
  }

  private buildParity(opts: Partial<ParityBody>): ParityReport {
    const balances = new Map(this.host.balances().map((b) => [b.account_id, b.debit_positive]));
    const rows: ParityRow[] = this.host
      .accounts()
      .filter((a) => a.is_active && a.account_type !== 'NonPosting')
      .map((a, i) => {
        const ours = balances.get(a.id) ?? 0;
        // Two accounts off: one by a rounding cent, one by a bill the bridge can't list.
        const off = i === 7 ? 1 : i === 19 ? -12_500 : 0;
        const qbd = ours + off;
        return { account_id: a.id, qbd_list_id: `8000${String(i).padStart(4, '0')}-1700000000`, full_name: a.full_name, account_type: a.account_type, qbd, ours, difference: ours - qbd, qbd_total: null, ours_total: null, note: off === 1 ? 'Sales tax rounding (QBD rounds per line)' : off ? 'A bill with a payroll item the bridge does not list' : null };
      });
    const off = rows.filter((r) => r.difference !== 0);
    const notes = [opts.as_of ? `Balances as of ${opts.as_of}.` : 'The bridge reports account balances as of today only; dated trial balances are not available over this transport.'];
    if (opts.trial_balance) notes.push('A trial balance is not available over the bridge; the account balances were compared instead.');
    return {
      generated_at: new Date().toISOString(),
      source_id: this.lastSync?.source_id ?? 'qbd:ein-94-0000000',
      accounts: rows,
      trial_balance: null,
      summary: { accounts_compared: rows.length, accounts_matching: rows.length - off.length, accounts_off: off.length, total_abs_difference: off.reduce((s, r) => s + Math.abs(r.difference), 0), trial_balance_rows: 0, trial_balance_matching: 0, notes },
    };
  }

  /** `POST …/qbwc/password`: a chosen password (≥ 8 characters; nothing back) or a generated one, shown this once. */
  qbwcPassword(password?: string | null): string | null {
    const given = password?.trim() ?? '';
    if (given && given.length < 8) throw new ApiError(400, 'invalid', 'the Web Connector password needs at least 8 characters');
    this.qbwcPasswordSet = true;
    if (given) return null;
    return Array.from(crypto.getRandomValues(new Uint8Array(12)), (b) => 'abcdefghjkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789'[b % 54]).join('');
  }

  /** `GET …/qbwc.qwc`: `AppURL` is the company's `app_url`, else the server's public URL; always read-only. */
  qbwcFile(): Blob {
    const q = this.qbd.qbwc;
    const base = (q.app_url ?? this.settings.public_urls[0] ?? 'https://books.example.com').replace(/\/$/, '');
    const braces = (uuid: string) => `{${uuid.toUpperCase()}}`;
    const scheduler = q.run_every_minutes ? `  <Scheduler><RunEveryNMinutes>${q.run_every_minutes}</RunEveryNMinutes></Scheduler>\n` : '';
    const xml = `<?xml version="1.0"?>\n<QBWCXML>\n  <AppName>LibreBooks (${this.host.companyName()})</AppName>\n  <AppID></AppID>\n  <AppURL>${base}/qbwc</AppURL>\n  <AppDescription>Imports this company file into LibreBooks</AppDescription>\n  <AppSupport>${base}/</AppSupport>\n  <UserName>${q.user_name}</UserName>\n  <OwnerID>${braces(q.owner_id)}</OwnerID>\n  <FileID>${braces(q.file_id)}</FileID>\n  <QBType>QBFS</QBType>\n${scheduler}  <IsReadOnly>true</IsReadOnly>\n</QBWCXML>\n`;
    return new Blob([xml], { type: 'application/xml' });
  }

  /**
   * The report a sync lands, shaped by its options as the importer's would be:
   * `full` lists everything again (every record created); a limit or a date
   * range is a partial run (watermarks untouched); `entities` narrows the
   * report to those kinds; `reimport` counts the unchanged as updated.
   */
  private sampleReport(startedAt: string, opts: Partial<SyncBody>): SyncReport {
    const full = !!opts.full;
    const partial = !!(opts.txn_limit || opts.txn_from || opts.txn_to);
    const entity = (e: SyncReport['entities'][number]['entity'], listed: number, created: number, updated: number, extra: Partial<SyncReport['entities'][number]> = {}) => {
      const unchanged = Math.max(0, listed - created - updated);
      return { entity: e, listed, created, updated: opts.reimport ? updated + unchanged : updated, unchanged: opts.reimport ? 0 : unchanged, skipped: 0, failed: 0, partial: 0, deleted: 0, deactivated: 0, watermark_before: full ? null : daysAgo(9), watermark_after: partial ? daysAgo(9) : startedAt, ...extra };
    };
    const only = opts.entities?.length ? new Set(opts.entities) : null;
    return {
      source_id: 'qbd:ein-94-0000000',
      started_at: startedAt,
      finished_at: new Date(new Date(startedAt).getTime() + 48_000).toISOString(),
      requests: 214,
      full,
      partial_run: partial,
      entities: [
        entity('Account', 56, full ? 56 : 0, full ? 0 : 1),
        entity('Customer', 18, full ? 18 : 1, 0),
        entity('Vendor', 11, full ? 11 : 0, 0),
        entity('Item', 18, full ? 18 : 0, 2),
        entity('Invoice', 24, full ? 24 : 3, 1),
        entity('Check', 41, full ? 41 : 4, 0, { skipped: 1 }),
        entity('Bill', 12, full ? 12 : 1, 0, { partial: 1 }),
        entity('Deposit', 6, full ? 6 : 1, 0),
        entity('JournalEntry', 3, full ? 3 : 0, 0),
      ].filter((e) => !only || only.has(e.entity)),
      issues: [
        { severity: 'Warning', entity: 'Check', qbd_id: '1A2B-1700000001', record: 'Check 1180 (2026-09-14)', message: 'Paycheck: payroll items are not imported; recorded as a check to the employee with the net amount' },
        { severity: 'Info', entity: 'Bill', qbd_id: '3C4D-1700000002', record: 'Bill 8821 (Rockler Woodworking)', message: 'Linked item receipt not listed yet; the link is retried next sync' },
      ],
      mismatches: [{ entity: 'Invoice', qbd_id: '5E6F-1700000003', record: 'Invoice 1042 (Hollis Residence)', kind: 'SalesTax', qbd: 4_613, ours: 4_612 }],
      gaps: [{ qbd_type: 'PayrollItemWage', links: 1, note: 'The bridge cannot list payroll items' }],
      unmapped_fields: { 'Customer.PriceLevelRef': 3, 'Item.UnitOfMeasureSetRef': 2 },
      created_lists: ['SalesTaxCode "Non"'],
      aborted: null,
    } as SyncReport;
  }

  /** Tests and teardown: stop a running job's timers and let any long-poll go. */
  dispose() {
    for (const t of this.qbdTimers) clearTimeout(t);
    this.qbdTimers = [];
    this.qbdFinished();
  }
}
