import { createHash } from 'node:crypto';
import { Buffer } from 'node:buffer';
import { readFile, writeFile, mkdir, rename, rm, stat } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { URL } from 'node:url';

const lock = JSON.parse(await readFile(new URL('./native-smoke-model.json', import.meta.url), 'utf8'));
const destination = resolve(process.argv[2] ?? 'native-smoke-model.gguf');
const digest = (data) => createHash('sha256').update(data).digest('hex');
const valid = (data) => data.length === lock.bytes && digest(data) === lock.sha256;
let cached;
try {
  if ((await stat(destination)).size !== lock.bytes) throw new Error('Existing file differs from smoke fixture; choose a different destination');
  cached = await readFile(destination);
  if (!valid(cached)) throw new Error('Existing file checksum differs; choose a different destination');
} catch (error) { if (error.code !== 'ENOENT') throw error; }
if (cached && valid(cached)) {
  console.log(`Verified cached smoke fixture: ${destination}`);
} else {
  const url = `https://huggingface.co/${lock.repository}/resolve/${lock.revision}/${lock.file}`;
  const response = await globalThis.fetch(url, { signal: globalThis.AbortSignal.timeout(120_000) });
  if (!response.ok || !response.body) throw new Error(`Smoke fixture download failed: HTTP ${response.status}`);
  const chunks = [];
  let size = 0;
  for await (const chunk of response.body) {
    size += chunk.length;
    if (size > lock.bytes) throw new Error('Smoke fixture exceeds pinned byte length');
    chunks.push(chunk);
  }
  const data = Buffer.concat(chunks);
  if (!valid(data)) throw new Error('Smoke fixture size/SHA256 mismatch');
  await mkdir(dirname(destination), { recursive: true });
  const temporary = `${destination}.${process.pid}.partial`;
  try {
    await writeFile(temporary, data, { flag: 'wx' });
    await rename(temporary, destination);
  } finally { await rm(temporary, { force: true }); }
  console.log(`Downloaded and verified smoke fixture: ${destination}`);
}
