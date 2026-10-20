// `/`: who am I, which companies can I open. One company goes straight in;
// no session goes to sign in.

import { Alert, Anchor, Button, Card, Center, Loader, Stack, Text, Title } from '@mantine/core';
import { Navigate, useNavigate } from 'react-router';
import { describeError, isApiError } from '~/api/errors';
import { useSession } from '~/api/hooks';

export function CompanyPicker() {
  const navigate = useNavigate();
  const session = useSession();
  if (session.isLoading) {
    return (
      <Center h="100dvh">
        <Loader />
      </Center>
    );
  }
  if (session.error) {
    if (isApiError(session.error) && session.error.code === 'unauthenticated') return <Navigate to="/signin" replace />;
    return (
      <Center h="100dvh" p="md">
        <Stack maw={460}>
          <Title order={2}>LibreBooks</Title>
          <Alert color="red" title="Can't reach the server">
            {describeError(session.error)}
          </Alert>
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
