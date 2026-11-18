import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const catalog = readFileSync(new URL("../src/lib/sidebarLocale.ts", import.meta.url), "utf8");
const sidebar = readFileSync(new URL("../src/components/Sidebar.tsx", import.meta.url), "utf8");
const app = readFileSync(new URL("../src/App.tsx", import.meta.url), "utf8");

test("sidebar language follows persisted application preferences", () => {
  assert.match(app, /<Sidebar\s+language=\{preferences\?\.language\}/);
  assert.match(sidebar, /sidebarText\(props\.language, key\)/);
});

test("English fallback is explicit for unsupported or missing locale", () => {
  assert.match(catalog, /language === "Bangla" \|\| language === "Hindi"/);
  assert.match(catalog, /: sidebarMessages\.English/);
  assert.match(catalog, /catalog\[key\] \|\| sidebarMessages\.English\[key\]/);
});

test("sidebar navigation and collapsed tooltips use translated labels", () => {
  for (const key of ["newChat", "library", "models", "datasets", "tools", "research", "studio3d", "webSearch", "files", "projects", "more", "settings"]) {
    assert.match(catalog, new RegExp(key + ":"));
    assert.ok(sidebar.includes('t("' + key + '")'), key);
  }
  for (const locale of ["English", "Bangla", "Hindi"]) assert.ok(catalog.includes(locale + ":"), locale);
  assert.match(sidebar, /title=\{t\("newChat"\)\}/);
  assert.match(sidebar, /title=\{t\("search"\)\}/);
});
