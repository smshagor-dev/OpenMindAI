import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
const nav = readFileSync(new URL("../src/i18n/navigation.ts", import.meta.url), "utf8");
const sidebar = readFileSync(new URL("../src/components/Sidebar.tsx", import.meta.url), "utf8");
const app = readFileSync(new URL("../src/App.tsx", import.meta.url), "utf8");

test("sidebar receives persisted locale and translates navigation and accessibility titles", () => {
  assert.match(app, /language=\{preferences\?\.language\}/);
  assert.match(sidebar, /navigationText\(props\.language, key\)/);
  for (const key of ["newChat","library","models","datasets","tools","projects","more","settings","search","collapse","expand"]) {
    assert.ok(sidebar.includes(`t("${key}")`), `missing translated ${key}`);
  }
});
test("Bangla and Hindi catalogs include the same keys as English and an explicit fallback", () => {
  const keys = [...nav.matchAll(/^    ([a-zA-Z0-9]+): /gm)].map(x => x[1]);
  assert.equal(keys.length, 3 * 20);
  for (let i = 0; i < 20; i++) {
    assert.equal(keys[i], keys[i+20]);
    assert.equal(keys[i], keys[i+40]);
  }
  assert.match(nav, /language === "Bangla" \|\| language === "Hindi"/);
  assert.match(nav, /navigationMessages\.English\[key\]/);
});
