// Sign in with a passkey (`/signin`). Usernameless: one button, plus the
// browser's passkey autofill on the name field where supported (conditional UI),
// so returning users usually just pick their passkey from the field.

import { Alert, Anchor, Button, Center, Paper, Stack, Text, TextInput, Title } from '@mantine/core';
import { IconKey } from '@tabler/icons-react';
import { browserSupportsWebAuthn, browserSupportsWebAuthnAutofill, startAuthentication, WebAuthnAbortService } from '@simplewebauthn/browser';
import { useQueryClient } from '@tanstack/react-query';
import { useEffect, useState } from 'react';
import { useNavigate, useSearchParams } from 'react-router';
import { useApi } from '~/api/context';
import { describeError } from '~/api/errors';
import { ceremonyError, isAborted } from './webauthn';

/** Only follow same-app paths after signing in (never an absolute URL). */
function safeNext(next: string | null): string {
  return next && next.startsWith('/') && !next.startsWith('//') ? next : '/';
}

export function SignIn() {
  const api = useApi();
  const qc = useQueryClient();
  const navigate = useNavigate();
  const [params] = useSearchParams();
  const next = safeNext(params.get('next'));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const supported = browserSupportsWebAuthn();

  const signIn = async (autofill: boolean) => {
    if (!autofill) setBusy(true);
    setError(null);
    try {
      const start = await api.loginStart();
      const credential = await startAuthentication({ optionsJSON: start.options, useBrowserAutofill: autofill });
      await api.loginFinish({ challenge_id: start.challenge_id, credential });
      qc.clear(); // nothing cached from before belongs to this user
      navigate(next, { replace: true });
    } catch (e) {
      // The autofill ceremony is cancelled whenever the button starts another; that's not an error.
      if (!isAborted(e)) setError(ceremonyError(e) ?? describeError(e));
    } finally {
      setBusy(false);
    }
  };

  useEffect(() => {
    let live = true;
    void browserSupportsWebAuthnAutofill().then((ok) => {
      if (ok && live) void signIn(true);
    });
    return () => {
      live = false;
      WebAuthnAbortService.cancelCeremony();
    };
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  return (
    <Center mih="100dvh" p="md">
      <Paper withBorder p="xl" radius="md" w={400} maw="100%">
        <Stack>
          <Title order={2}>LibreBooks</Title>
          <Text c="dimmed">Sign in with the passkey you set up for these books.</Text>
          {!supported && <Alert color="orange">This browser can't use passkeys. Try a current Chrome, Edge, Safari or Firefox.</Alert>}
          {/* The autofill target for conditional UI; the passkey carries the account. */}
          <TextInput label="Name" placeholder="Choose your passkey" autoComplete="username webauthn" name="username" />
          <Button size="md" leftSection={<IconKey size={18} />} loading={busy} disabled={!supported} onClick={() => void signIn(false)} data-autofocus>
            Sign in with a passkey
          </Button>
          {error && (
            <Alert color="red" role="alert">
              {error}
            </Alert>
          )}
          <Text size="sm" c="dimmed">
            No passkey yet? Ask an administrator for an enrolment link; it works once and expires after a few minutes.
          </Text>
          <Text size="xs" c="dimmed">
            Just looking around? <Anchor href="/?fixtures=1">Open the demo books</Anchor>.
          </Text>
        </Stack>
      </Paper>
    </Center>
  );
}
