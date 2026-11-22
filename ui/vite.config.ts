import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Port and strictPort match devUrl in crates/app/tauri.conf.json.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true, watch: { ignored: ["**/crates/**", "**/target/**"] } },
});
