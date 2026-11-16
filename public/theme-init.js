// This must run before the application's styles mount. Tauri CSP forbids inline scripts.
(() => {
  let theme = "light";
  try {
    const savedTheme = globalThis.localStorage.getItem("openmindai:theme");
    if (savedTheme === "light" || savedTheme === "dark") {
      theme = savedTheme;
    } else if (savedTheme === "system" && globalThis.matchMedia("(prefers-color-scheme: dark)").matches) {
      theme = "dark";
    }
  } catch {
    // The native preference load will apply the user's saved setting.
  }
  globalThis.document.documentElement.dataset.theme = theme;
  globalThis.document
    .querySelector('meta[name="theme-color"]')
    ?.setAttribute("content", theme === "light" ? "#f1f1f1" : "#090909");
})();
