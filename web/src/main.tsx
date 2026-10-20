import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { initApiClient } from './api/select';
import { App } from './App';

// The client first: fixture mode loads its books on demand.
void initApiClient().then(() => {
  createRoot(document.getElementById('root')!).render(
    <StrictMode>
      <App />
    </StrictMode>,
  );
});
