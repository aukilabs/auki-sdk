import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [react()],
  server: {
    allowedHosts: ["taina-proclergy-chang.ngrok-free.dev"],
    host: "0.0.0.0",
    port: 7880,
    proxy: {
      "/discovery": {
        changeOrigin: true,
        rewrite: (path) => path.replace(/^\/discovery/, ""),
        target: "http://127.0.0.1:8091",
      },
    },
  },
});
