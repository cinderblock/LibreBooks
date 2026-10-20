// `/`: who am I, which companies can I open. One company goes straight in.
// Sign-in (passkeys, enrolment) is the next phase; until then an unauthenticated
// session gets a plain explanation and the fixture-mode link.

import { Alert, Anchor, Button, Card, Center, Loader, Stack, Text, Title } from '@mantine/core';
import { useQuery } from '@tanstack/react-query';
import { Navigate, useNavigate } from 'react-router';
import { useApi } from '~/api/context';
import { describeError, isApiError } from '~/api/errors';

export function CompanyPicker() {
  const api = useApi();
  const navigate = useNavigate();
  const session = useQuery({ queryKey: ['session'], queryFn: () => api.session(), retry: false });
  if (session.isLoading) {
    return (
      <Center h="100dvh">
        <Loader />
      </Center>
    );
  }
  if (session.error) {
    const unauth = isApiError(session.error) && session.error.code === 'unauthenticated';
    return (
      <Center h="100dvh" p="md">
        <Stack maw={460}>
          <Title order={2}>LibreBooks</Title>
          {unauth ? (
            <Alert title="Sign in needed">Signing in with a passkey arrives with the server's auth screens.</Alert>
          ) : (
            <Alert color="red" title="Can't reach the server">
              {describeError(session.error)}
            </Alert>
          )}
          <Text size="sm" c="dimmed">
            To try the interface without a server, open it in{' '}
            <Anchor href="/?fixtures=1">fixture mode</Anchor> (synthetic demo books held in memory).
          </Text>
        </Stack>
      </Center>
    );
  }
  const companies = session.data?.companies ?? [];
  if (companies.length === 1) return <Navigate to={`/c/${encodeURIComponent(companies[0]!.id)}/home`} replace />;
  return (
    <Center h="100dvh" p="md">
      <Stack w={420}>
        <Title order={2}>Open a company</Title>
        {companies.length === 0 && <Text c="dimmed">You don't have access to any company yet.</Text>}
        {companies.map((c) => (
          <Card key={c.id} withBorder>
            <Button variant="subtle" justify="space-between" fullWidth onClick={() => navigate(`/c/${encodeURIComponent(c.id)}/home`)}>
              {c.name}
            </Button>
          </Card>
        ))}
      </Stack>
    </Center>
  );
}
