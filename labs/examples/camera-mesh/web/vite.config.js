import { fileURLToPath } from 'node:url'
import { defineConfig } from 'vite'

const expoSrc = fileURLToPath(
  new URL('../../../../core/bindings/expo/src', import.meta.url),
)
const jsonStringify = fileURLToPath(
  new URL('../../../../core/bindings/expo/src/json-stringify.ts', import.meta.url),
)

export default defineConfig({
  resolve: {
    alias: {
      '@auki/json-stringify': jsonStringify,
    },
  },
  server: {
    fs: {
      allow: ['.', expoSrc],
    },
    allowedHosts: [
      'ca9d-2405-9800-b900-46e6-10ca-331a-67b-c34e.ngrok-free.app',
      'c4b2-2405-9800-b900-46e6-10ca-331a-67b-c34e.ngrok-free.app',
    ],
  },
})
