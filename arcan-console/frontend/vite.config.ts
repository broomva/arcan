import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  base: "/console/",
  server: {
    port: 5173,
    proxy: {
      "/health": "http://localhost:3000",
      "/sessions": "http://localhost:3000",
      "/openapi.json": "http://localhost:3000",
    },
  },
  build: {
    outDir: "dist",
    emptyDirOnBuild: true,
  },
});
