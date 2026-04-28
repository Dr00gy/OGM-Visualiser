import { defineConfig } from "vitest/config";
import { sveltekit } from "@sveltejs/kit/vite";
import path from "node:path";

export default defineConfig({
  plugins: [sveltekit()],
  resolve: {
    conditions: ['browser', 'development'],
    alias: {
      'svelte/internal/server': 'svelte/internal',
      'svelte/server': 'svelte',
      $routes: path.resolve('./src/routes'),
    }
  },
  test: {
    globals: true,
    environment: "jsdom",
    setupFiles: ["./tests/setup.ts"],
    include: ["tests/**/*.test.ts"],
    server: {
      deps: {
        inline: [
          "@sveltejs/kit",
          "svelte",
          "svelte/internal"
        ]
      }
    },
    environmentOptions: {
      jsdom: {
        resources: "usable",
        pretendToBeVisual: true,
      },
    },
  },
});