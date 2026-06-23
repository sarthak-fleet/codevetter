import path from "path";
import { defineConfig } from "vitest/config";

export default defineConfig({
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  test: {
    environment: "node",
    include: ["src/**/*.test.ts"],
    coverage: {
      provider: "v8",
      include: ["src/lib/**/*.ts"],
      exclude: [
        "**/*.test.ts",
        "**/*.d.ts",
        "**/index.ts",
        "node_modules",
        "dist",
        ".next",
        ".wrangler",
      ],
      // Ratchet gate: thresholds are set slightly ABOVE the current measured
      // coverage so coverage can only ever go UP. Raise these numbers whenever
      // coverage improves — never lower them. Target is the EXEMPLARY 80/80/70/80.
      // Baseline when this ratchet was introduced (2026-05):
      //   lines 70.32 · functions 54.86 · branches 57.42 · statements 69.75
      thresholds: {
        lines: 72,
        functions: 56,
        branches: 58,
        statements: 72,
      },
    },
  },
});
