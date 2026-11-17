import { defineConfig } from 'vite'
import { svelte } from '@sveltejs/vite-plugin-svelte'

// Serves the UI to `tauri dev`, and builds it into dist/ for `tauri build`.
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  // Tauri expects the dev server on this exact port; the example scenes live in ../examples.
  server: { port: 5173, strictPort: true, fs: { allow: ['..'] }, watch: { ignored: ['**/src-tauri/**'] } },
  build: { target: 'es2022' },
})
