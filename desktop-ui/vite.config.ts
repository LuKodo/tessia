import { defineConfig } from "vitest/config";

// Plain Vite SPA: `vite build` writes dist/ with index.html, which the Go desktop build embeds.
// Port 3000 keeps the CORS origin already documented for `tessia serve --allowed-origin`.
export default defineConfig({
  server: { port: 3000 },
  build: { outDir: "dist", target: "es2022" },
  test: {
    environment: "happy-dom",
    include: ["tests/unit/**/*.test.ts"],
  },
});
