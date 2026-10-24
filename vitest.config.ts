import { defineConfig } from 'vitest/config';
export default defineConfig({ test: { include: ['src/**/*.test.{ts,tsx}'], maxWorkers: 4, environment: 'jsdom', setupFiles: ['./src/test/setup.ts'], clearMocks: true } });
