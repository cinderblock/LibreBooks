import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import { fileURLToPath } from 'node:url';

// In development the SPA is served by Vite and the API by `librebooks serve`
// (default port 8740); everything under /api is proxied there.
const api = process.env.LIBREBOOKS_API ?? 'http://127.0.0.1:8740';

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: { '~': fileURLToPath(new URL('./src', import.meta.url)) },
  },
  server: {
    port: 5740,
    strictPort: true,
    proxy: {
      '/api': { target: api, changeOrigin: false, ws: true },
    },
  },
  build: {
    outDir: 'dist',
    sourcemap: true,
    target: 'es2023',
  },
});
