import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { URL } from "node:url";
const nav = readFileSync(new URL("../src/i18n/navigation.ts", import.meta.url), "utf8");
const sidebar = readFileSync(new URL("../src/components/Sidebar.tsx", import.meta.url), "utf8");
const app = readFileSync(new URL("../src/App.tsx", import.meta.url), "utf8");

test("sidebar receives persisted locale and translates navigation and accessibility titles", () => {
  assert.match(app, /language=\{preferences\?\.language\}/);
  assert.match(sidebar, /navigationText\(props\.language, key\)/);
  for (const key of ["newChat","library","studio3d","research","tools","projects","more","settings","search","collapse","expand"]) {
    assert.ok(sidebar.includes(`t("${key}")`), `missing translated ${key}`);
  }
});
test("Bangla and Hindi catalogs include the same keys as English and an explicit fallback", () => {
  const keysByLocale = ["English", "Bangla", "Hindi"].map((locale) => {
    const start = nav.indexOf(locale + ": {");
    const end = nav.indexOf("\n  },", start);
    assert.ok(start >= 0 && end > start, locale + " catalog exists");
    return [...nav.slice(start, end).split("{").slice(1).join("{").matchAll(/([a-zA-Z0-9]+): /g)].map((match) => match[1]);
  });
  assert.deepEqual(keysByLocale[1], keysByLocale[0]);
  assert.deepEqual(keysByLocale[2], keysByLocale[0]);
  assert.match(nav, /language === "Bangla" \|\| language === "Hindi"/);
  assert.match(nav, /navigationMessages\.English\[key\]/);
});
