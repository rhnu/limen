import { defineConfig } from '@hey-api/openapi-ts';

export default defineConfig({
  input: '../admin.openapi.json',
  output: '../../web/src/api/generated',
  plugins: ['@hey-api/typescript', '@hey-api/sdk', '@hey-api/client-fetch'],
});
