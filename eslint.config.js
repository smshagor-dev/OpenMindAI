import js from "@eslint/js";
import tseslint from "@typescript-eslint/eslint-plugin";
import tsParser from "@typescript-eslint/parser";
import hooks from "eslint-plugin-react-hooks";

export default [
  {
    ignores: ["dist/**", "node_modules/**", "src-tauri/**"],
  },
  js.configs.recommended,
  {
    files: ["scripts/**/*.mjs"],
    languageOptions: {
      globals: {
        console: "readonly",
        process: "readonly",
      },
    },
  },
  {
    files: ["src/**/*.{ts,tsx}"],
    languageOptions: {
      parser: tsParser,
      parserOptions: {
        ecmaVersion: "latest",
        sourceType: "module",
      },
      globals: {
        window: "readonly",
        document: "readonly",
        console: "readonly",
        HTMLElement: "readonly",
        HTMLTextAreaElement: "readonly",
        HTMLDivElement: "readonly",
        HTMLInputElement: "readonly",
        HTMLButtonElement: "readonly",
        HTMLVideoElement: "readonly",
        HTMLMediaElement: "readonly",
        AudioBuffer: "readonly",
        MediaRecorder: "readonly",
        MediaStream: "readonly",
        Blob: "readonly",
        URL: "readonly",
        KeyboardEvent: "readonly",
        MouseEvent: "readonly",
        Node: "readonly",
        File: "readonly",
        FileList: "readonly",
        FileReader: "readonly",
        Image: "readonly",
        navigator: "readonly",
        crypto: "readonly",
      },
    },
    plugins: {
      "@typescript-eslint": tseslint,
      "react-hooks": hooks,
    },
    rules: {
      ...tseslint.configs.recommended.rules,
      ...hooks.configs.recommended.rules,
    },
  },
];
