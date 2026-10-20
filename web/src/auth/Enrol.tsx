// Enrolment (`/enroll#token=…`): turn a one-time link from an administrator
// into a passkey and a signed-in session.
//
// The token arrives in the URL fragment, which browsers never send to a server
// or put in Referer headers; the page reads it once and strips it from the
// address bar so it doesn't linger in history. The person's name is asked
// *before* the passkey prompt, because it labels the passkey in their password
// manager and can't be changed afterwards. If the server still wants a name at
// the finish step, the same credential is resubmitted with it (no second prompt).

import { Alert, Badge, Button, Center, Paper, Stack, Text, TextInput, Title } from '@mantine/core';
import { IconKey } from '@tabler/icons-react';
import { browserSupportsWebAuthn, startRegistration, type RegistrationResponseJSON } from '@simplewebauthn/browser';
import { useQueryClient } from '@tanstack/react-query';
import { useState } from 'react';
import { useNavigate } from 'react-router';
import { useApi } from '~/api/context';
import { describeError, isApiError } from '~/api/errors';
import type { EnrolmentPurpose } from '~/api/generated/EnrolmentPurpose';
import { ceremonyError, isAborted } from './webauthn';

let taken: string | null | undefined;

/**
 * Read `#token=…` (or `?token=`) once per page load, then remove it from the URL.
 * Cached because React may run a state initializer twice (StrictMode) and the
 * second run would find the address bar already cleaned.
 */
function takeToken(): string | null {
  if (taken !== undefined) return taken;
  const hash = new URLSearchParams(window.location.hash.replace(/^#/, ''));
  const query = new URLSearchParams(window.location.search);
  const token = hash.get('token') ?? query.get('token');
  if (token) window.history.replaceState(window.history.state, '', window.location.pathname);
  taken = token;
  return token;
}

const PURPOSE_TEXT: Record<EnrolmentPurpose, string> = {
  first_admin: 'Set up the first administrator',
  new_user: 'Create your account',
  add_device: 'Add a passkey on this device',
};

export function Enrol() {
  const api = useApi();
  const qc = useQueryClient();
  const navigate = useNavigate();
  const [token] = useState(takeToken);
  const [name, setName] = useState('');
  const [passkeyName, setPasskeyName] = useState('');
  const [purpose, setPurpose] = useState<EnrolmentPurpose | null>(null);
  const [pending, setPending] = useState<{ challenge_id: string; credential: RegistrationResponseJSON } | null>(null);
  const [needName, setNeedName] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const finish = async (challenge_id: string, credential: RegistrationResponseJSON) => {
    try {
      await api.enrolFinish({
        token: token!,
        challenge_id,
        credential,
        ...(name.trim() ? { display_name: name.trim() } : {}),
        ...(passkeyName.trim() ? { passkey_name: passkeyName.trim() } : {}),
      });
      qc.clear();
      navigate('/', { replace: true });
    } catch (e) {
      if (isApiError(e) && e.code === 'invalid' && /name/i.test(e.message)) {
        // Keep the ceremony: ask for the name and send the same credential again.
        setPending({ challenge_id, credential });
        setNeedName(true);
        setError(e.message);
      } else throw e;
    }
  };

  const submit = async () => {
    if (!token) return;
    setBusy(true);
    setError(null);
    try {
      if (pending) {
        await finish(pending.challenge_id, pending.credential);
        return;
      }
      const start = await api.enrolStart({ token, ...(name.trim() ? { display_name: name.trim() } : {}) });
      setPurpose(start.purpose);
      if (!name.trim() && start.display_name) setName(start.display_name);
      const credential = await startRegistration({ optionsJSON: start.options });
      await finish(start.challenge_id, credential);
    } catch (e) {
      if (!isAborted(e)) setError(ceremonyError(e) ?? describeError(e));
    } finally {
      setBusy(false);
    }
  };

  if (!token) {
    return (
      <Center mih="100dvh" p="md">
        <Stack maw={420}>
          <Title order={2}>Enrolment link needed</Title>
          <Text c="dimmed">
            This page sets up a passkey from a one-time link an administrator gives you. Open the link they sent (it ends in
            <code> #token=…</code>), or ask for a new one: they expire after a few minutes or their first use.
          </Text>
        </Stack>
      </Center>
    );
  }

  return (
    <Center mih="100dvh" p="md">
      <Paper withBorder p="xl" radius="md" w={420} maw="100%">
        <form
          onSubmit={(e) => {
            e.preventDefault();
            void submit();
          }}
        >
          <Stack>
            <Title order={2}>LibreBooks</Title>
            <Text fw={500}>{purpose ? PURPOSE_TEXT[purpose] : 'Set up your passkey'}</Text>
            {purpose && <Badge variant="light">{purpose.replace('_', ' ')}</Badge>}
            {!browserSupportsWebAuthn() && <Alert color="orange">This browser can't create passkeys. Try a current Chrome, Edge, Safari or Firefox.</Alert>}
            {purpose !== 'add_device' && (
              <TextInput
                label="Your name"
                description="Shown to others in the books and on the passkey in your password manager."
                value={name}
                onChange={(e) => setName(e.currentTarget.value)}
                error={needName ? 'Please enter your name' : undefined}
                autoComplete="name"
                data-autofocus
              />
            )}
            <TextInput
              label="Name this passkey (optional)"
              placeholder="Work laptop"
              value={passkeyName}
              onChange={(e) => setPasskeyName(e.currentTarget.value)}
              autoComplete="off"
            />
            <Button type="submit" size="md" leftSection={<IconKey size={18} />} loading={busy}>
              {pending ? 'Finish' : 'Create passkey'}
            </Button>
            {error && (
              <Alert color="red" role="alert">
                {error}
              </Alert>
            )}
            <Text size="xs" c="dimmed">
              Your device keeps the passkey's private half; LibreBooks never sees a password.
            </Text>
          </Stack>
        </form>
      </Paper>
    </Center>
  );
}
