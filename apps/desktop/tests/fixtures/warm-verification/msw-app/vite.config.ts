import path from 'node:path';
import { fileURLToPath } from 'node:url';
import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

const root = path.dirname(fileURLToPath(import.meta.url));

export default defineConfig({
  root,
  publicDir: path.resolve(process.cwd(), 'node_modules/msw/lib'),
  plugins: [react()],
  appType: 'spa',
  server: {
    host: '127.0.0.1',
    strictPort: true,
  },
});
