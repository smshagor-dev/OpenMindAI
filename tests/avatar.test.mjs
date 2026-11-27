import test from "node:test";
import assert from "node:assert/strict";
import { readImageAsDataUrl } from "../src/lib/avatar.ts";

function withBrowserMocks({ width = 4000, height = 2000, drawError = false, encodeError = false }, run) {
  const originalReader = globalThis.FileReader;
  const originalImage = globalThis.Image;
  const originalDocument = globalThis.document;
  const canvases = [];
  class MockReader {
    result = "data:image/png;base64,AA==";
    readAsDataURL() { queueMicrotask(() => this.onload?.()); }
  }
  class MockImage {
    width = width;
    height = height;
    set src(_value) { queueMicrotask(() => this.onload?.()); }
  }
  globalThis.FileReader = MockReader;
  globalThis.Image = MockImage;
  globalThis.document = {
    createElement(name) {
      assert.equal(name, "canvas");
      const canvas = {
        width: 0, height: 0,
        getContext() {
          return { drawImage() { if (drawError) throw new Error("draw failure"); } };
        },
        toDataURL() {
          if (encodeError) throw new Error("encoding failure");
          return "data:image/jpeg;base64,AA==";
        },
      };
      canvases.push(canvas);
      return canvas;
    },
  };
  return Promise.resolve().then(() => run(canvases)).finally(() => {
    globalThis.FileReader = originalReader;
    globalThis.Image = originalImage;
    globalThis.document = originalDocument;
  });
}

test("avatar output is bounded even when caller requests a huge canvas", async () => {
  await withBrowserMocks({}, async (canvases) => {
    assert.equal(await readImageAsDataUrl({}, 100000), "data:image/jpeg;base64,AA==");
    assert.equal(canvases[0].width, 1024);
    assert.equal(canvases[0].height, 512);
  });
});

test("invalid target sizes reject before reading the image", async () => {
  await withBrowserMocks({}, async (canvases) => {
    for (const size of [0, -1, NaN, Infinity, 0.5]) {
      await assert.rejects(readImageAsDataUrl({}, size), /Invalid avatar output dimension/);
    }
    assert.equal(canvases.length, 0);
  });
});

test("invalid decoded geometry rejects without allocating a canvas", async () => {
  for (const dimensions of [{ width: 0, height: 5 }, { width: Infinity, height: 2 }, { width: NaN, height: 2 }]) {
    await withBrowserMocks(dimensions, async (canvases) => {
      await assert.rejects(readImageAsDataUrl({}), /Invalid decoded avatar dimensions/);
      assert.equal(canvases.length, 0);
    });
  }
});

test("canvas draw and encoding exceptions reject the returned promise", async () => {
  for (const options of [{ drawError: true }, { encodeError: true }]) {
    await withBrowserMocks(options, async () => {
      await assert.rejects(readImageAsDataUrl({}), /Failed to render avatar image/);
    });
  }
});
