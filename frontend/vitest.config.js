import { defineConfig } from "vitest/config";
import { sveltekit } from "@sveltejs/kit/vite";

export default defineConfig({
  plugins: [sveltekit()],
  resolve: {
    conditions: ['browser', 'development'],
    alias: {
      'svelte/internal/server': 'svelte/internal',
      'svelte/server': 'svelte'
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