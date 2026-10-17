import { mkdtemp, readFile, readdir, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, relative, isAbsolute } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createClient } from '@hey-api/openapi-ts';
import config from './openapi-ts.config.ts';

// Compare generated artifacts, including the bundled client, without rewriting them.
async function files(directory, prefix = '') {
  const entries = await readdir(join(directory, prefix), { withFileTypes: true });
  const result = [];
  for (const entry of entries) {
    const path = join(prefix, entry.name);
    if (entry.isDirectory()) result.push(...await files(directory, path));
    else if (entry.isFile()) result.push(path);
  }
  return result.sort();
}

const parent = tmpdir();
const temporary = await mkdtemp(join(parent, 'limen-api-'));
try {
  await createClient({ ...config, output: temporary });
  const destination = fileURLToPath(new URL('../../web/src/api/generated/', import.meta.url));
  const expected = await files(temporary);
  let current;
  try { current = await files(destination); }
  catch (error) {
    if (error.code !== 'ENOENT') throw error;
    current = [];
  }
  let matching = JSON.stringify(expected) === JSON.stringify(current);
  if (matching) {
    for (const name of expected) {
      const [fresh, saved] = await Promise.all([
        readFile(join(temporary, name), 'utf8'),
        readFile(join(destination, name), 'utf8'),
      ]);
      if (fresh.replaceAll('\r\n', '\n') !== saved.replaceAll('\r\n', '\n')) {
        matching = false;
        break;
      }
    }
  }
  if (!matching) {
    console.error('Generated API client differs; run pnpm api:generate.');
    process.exitCode = 1;
  }
} finally {
  const child = relative(parent, temporary);
  if (!child || child.startsWith('..') || isAbsolute(child)) {
    throw new Error('Refusing to remove a directory outside the temporary root.');
  }
  await rm(temporary, { recursive: true, force: true });
}
