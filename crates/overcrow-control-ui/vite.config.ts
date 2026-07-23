import react from '@vitejs/plugin-react';
import { defineConfig } from 'vitest/config';

export default defineConfig({
  plugins: [react()],
  publicDir: '../../assets/branding',
  clearScreen: false,
  server: {
    host: '127.0.0.1',
    port: 1420,
    strictPort: true,
  },
  test: {
    environment: 'jsdom',
    environmentOptions: {
      jsdom: { url: 'http://localhost/' },
    },
    setupFiles: './src/test/setup.ts',
  },
});
