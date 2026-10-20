import '@mantine/core/styles.css';
import '@mantine/dates/styles.css';
import '@mantine/notifications/styles.css';
import '@mantine/spotlight/styles.css';
import './app.css';

import { createTheme, MantineProvider } from '@mantine/core';
import { Notifications } from '@mantine/notifications';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { useState } from 'react';
import { createBrowserRouter, Navigate, RouterProvider } from 'react-router';
import { ApiProvider } from './api/context';
import { isApiError } from './api/errors';
import { createApiClient } from './api/select';
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
  { path: '/c/:companyId/*', element: <CompanyShell /> },
  { path: '*', element: <Navigate to="/" replace /> },
]);

function makeQueryClient() {
  return new QueryClient({
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
  const [client] = useState(createApiClient);
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
