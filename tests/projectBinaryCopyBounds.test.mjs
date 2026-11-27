import { URL } from "node:url";
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const source = readFileSync(new URL("../src/lib/projectBinaryStorage.ts", import.meta.url), "utf8");

test("binary copy validates size before decoding untrusted base64", () => {
  const validation = source.indexOf("Number.isSafeInteger(file.sizeBytes)");
  const encodedLength = source.indexOf("file.dataBase64.length !== expectedEncodedLength");
  const decode = source.indexOf("globalThis.atob(file.dataBase64)");
  assert.ok(validation >= 0 && validation < encodedLength);
  assert.ok(encodedLength < decode, "encoded payload must be bounded before atob");
});

test("binary copy rejects malformed alphabet before decoding", () => {
  const alphabet = source.indexOf("A-Za-z0-9+/");
  const decode = source.indexOf("globalThis.atob(file.dataBase64)");
  assert.ok(alphabet >= 0 && alphabet < decode);
  assert.match(source, /Saved attachment has invalid base64 encoding/);
});
