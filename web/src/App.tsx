import '@mantine/core/styles.css';
import '@mantine/dates/styles.css';
import '@mantine/notifications/styles.css';
import '@mantine/spotlight/styles.css';
import './app.css';

import { createTheme, MantineProvider } from '@mantine/core';
import { Notifications } from '@mantine/notifications';
import { QueryCache, QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { useState } from 'react';
import { createBrowserRouter, Navigate, RouterProvider } from 'react-router';
import { ApiProvider } from './api/context';
import { isApiError } from './api/errors';
import { getApiClient } from './api/select';
import { Enrol } from './auth/Enrol';
import { SignIn } from './auth/SignIn';
import { KeyboardProvider } from './keyboard/KeyboardProvider';
import { CompanyPicker } from './shell/CompanyPicker';
import { CompanyShell } from './shell/CompanyShell';

const theme = createTheme({
  primaryColor: 'teal',
  defaultRadius: 'sm',
  cursorType: 'pointer',
  fontFamily: 'Inter, system-ui, -apple-system, "Segoe UI", Roboto, sans-serif',
  fontFamilyMonospace: 'ui-monospace, "Cascadia Mono", Consolas, monospace',
});

const router = createBrowserRouter([
  { path: '/', element: <CompanyPicker /> },
  { path: '/signin', element: <SignIn /> },
  { path: '/enroll', element: <Enrol /> },
  { path: '/c/:companyId/*', element: <CompanyShell /> },
  { path: '*', element: <Navigate to="/" replace /> },
]);

/** A session that expired mid-use sends the person to sign in, then back. */
function onQueryError(e: unknown) {
  if (!isApiError(e) || e.code !== 'unauthenticated') return;
  const here = window.location.pathname;
  if (here === '/signin' || here === '/enroll') return;
  void router.navigate(`/signin?next=${encodeURIComponent(here + window.location.search)}`, { replace: true });
}

function makeQueryClient() {
  return new QueryClient({
    queryCache: new QueryCache({ onError: onQueryError }),
    defaultOptions: {
      queries: {
        // Client errors won't fix themselves; network blips might.
        retry: (count, e) => count < 2 && (!isApiError(e) || e.status === 0 || e.status >= 500),
        refetchOnWindowFocus: false,
      },
    },
  });
}

export function App() {
  const [client] = useState(getApiClient);
  const [queryClient] = useState(makeQueryClient);
  return (
    <MantineProvider theme={theme} defaultColorScheme="auto">
      <Notifications position="top-right" limit={4} />
      <QueryClientProvider client={queryClient}>
        <ApiProvider client={client}>
          <KeyboardProvider>
            <RouterProvider router={router} />
          </KeyboardProvider>
        </ApiProvider>
      </QueryClientProvider>
    </MantineProvider>
  );
}
