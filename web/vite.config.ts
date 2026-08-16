import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: {
    // Dev: the API and the preview proxy run on the CLI's port.
    proxy: {
      "/api": "http://127.0.0.1:7700",
    },
  },
  test: {
    environment: "jsdom",
    globals: true,
  },
});
