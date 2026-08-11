import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri 通过固定端口连接 dev server，端口被占用时应直接失败而非另选端口。
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
});
