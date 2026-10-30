import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

// `pnpm dev` serves this on every interface so phones/laptops on the LAN can join.
// /api (signaling + REST) is proxied to gs-server; game traffic goes straight to its UDP port.
const server = process.env.GS_SERVER ?? 'http://127.0.0.1:8080';

export default defineConfig({
  plugins: [react()],
  server: {
    host: true,
    port: 5173,
    proxy: { '/api': server },
  },
  preview: { host: true, proxy: { '/api': server } },
  build: { target: 'es2022', chunkSizeWarningLimit: 2000 },
});
