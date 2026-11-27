import test from "node:test";
import assert from "node:assert/strict";
import { Buffer, Blob } from "node:buffer";
import { TextDecoder, TextEncoder } from "node:util";
import { JSDOM } from "jsdom";
import * as THREE from "three";
import { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";
import { GLTFExporter } from "three/examples/jsm/exporters/GLTFExporter.js";
import validator from "gltf-validator";

const dom = new JSDOM("");
const { document } = dom.window;
globalThis.document = dom.window.document;
globalThis.FileReader = class {
  onload = null;
  onloadend = null;
  onerror = null;
  result = null;
  readAsArrayBuffer(blob) {
    blob.arrayBuffer().then((result) => {
      this.result = result;
      this.onload?.({ target: this });
      this.onloadend?.({ target: this });
    }, (error) => this.onerror?.({ target: this, error }));
  }
};
globalThis.ProgressEvent = dom.window.ProgressEvent;

function makeTriangleGlb() {
  const positions = new Float32Array([
    0, 0, 0,
    1, 0, 0,
    0, 1, 0,
  ]);
  const indices = new Uint16Array([0, 1, 2]);
  const binary = new Uint8Array(44);
  binary.set(new Uint8Array(positions.buffer), 0);
  binary.set(new Uint8Array(indices.buffer), 36);
  const json = {
    asset: { version: "2.0", generator: "OpenMindAI mesh roundtrip test" },
    scene: 0,
    scenes: [{ nodes: [0] }],
    nodes: [{ name: "Root", children: [1] }, { name: "Assembly", children: [2] }, { name: "EditableTriangle", mesh: 0 }],
    meshes: [{ primitives: [{ attributes: { POSITION: 0 }, indices: 1, material: 0 }] }],
    materials: [{ name: "TestMetal", pbrMetallicRoughness: { baseColorFactor: [0.2, 0.4, 0.8, 1], metallicFactor: 0.65, roughnessFactor: 0.25 } }],
    buffers: [{ byteLength: binary.byteLength }],
    bufferViews: [
      { buffer: 0, byteOffset: 0, byteLength: 36, target: 34962 },
      { buffer: 0, byteOffset: 36, byteLength: 6, target: 34963 },
    ],
    accessors: [
      { bufferView: 0, componentType: 5126, count: 3, type: "VEC3", min: [0, 0, 0], max: [1, 1, 0] },
      { bufferView: 1, componentType: 5123, count: 3, type: "SCALAR" },
    ],
  };
  const jsonBytes = new TextEncoder().encode(JSON.stringify(json));
  const jsonLength = (jsonBytes.byteLength + 3) & ~3;
  const total = 12 + 8 + jsonLength + 8 + binary.byteLength;
  const glb = new ArrayBuffer(total);
  const view = new DataView(glb);
  view.setUint32(0, 0x46546c67, true);
  view.setUint32(4, 2, true);
  view.setUint32(8, total, true);
  view.setUint32(12, jsonLength, true);
  view.setUint32(16, 0x4e4f534a, true);
  new Uint8Array(glb, 20, jsonBytes.byteLength).set(jsonBytes);
  new Uint8Array(glb, 20 + jsonBytes.byteLength, jsonLength - jsonBytes.byteLength).fill(0x20);
  const binHeader = 20 + jsonLength;
  view.setUint32(binHeader, binary.byteLength, true);
  view.setUint32(binHeader + 4, 0x004e4942, true);
  new Uint8Array(glb, binHeader + 8).set(binary);
  return glb;
}

test("GLB hierarchy, transforms, undo/redo, materials and edited export survive reimport", async () => {
  const loader = new GLTFLoader();
  const original = await loader.parseAsync(makeTriangleGlb(), "");
  const mesh = original.scene.getObjectByName("EditableTriangle");
  assert.ok(mesh instanceof THREE.Mesh);
  assert.equal(mesh.parent?.name, "Assembly");
  assert.equal(mesh.geometry.getAttribute("position").count, 3);
  assert.equal(mesh.material.name, "TestMetal");
  assert.equal(mesh.material.metalness, 0.65);
  assert.equal(mesh.material.roughness, 0.25);

  mesh.updateMatrix();
  const before = mesh.matrix.clone();
  mesh.position.set(2, -1, 3);
  mesh.rotation.set(0, THREE.MathUtils.degToRad(45), 0);
  mesh.scale.set(1.5, 2, 0.75);
  mesh.updateMatrix();
  const after = mesh.matrix.clone();
  mesh.matrix.copy(before).decompose(mesh.position, mesh.quaternion, mesh.scale);
  mesh.updateMatrix();
  assert.ok(mesh.position.distanceTo(new THREE.Vector3(0, 0, 0)) < 1e-6);
  mesh.matrix.copy(after).decompose(mesh.position, mesh.quaternion, mesh.scale);
  mesh.updateMatrix();
  assert.ok(mesh.position.distanceTo(new THREE.Vector3(2, -1, 3)) < 1e-6);
  assert.ok(mesh.scale.distanceTo(new THREE.Vector3(1.5, 2, 0.75)) < 1e-6);

  const exported = await new GLTFExporter().parseAsync(original.scene, { binary: true });
  assert.ok(exported instanceof ArrayBuffer);
  assert.equal(new DataView(exported).getUint32(0, true), 0x46546c67);
  const reopened = await loader.parseAsync(exported, "");
  const reopenedMesh = reopened.scene.getObjectByName("EditableTriangle");
  assert.ok(reopenedMesh instanceof THREE.Mesh);
  assert.equal(reopenedMesh.parent?.name, "Assembly");
  assert.ok(reopenedMesh.position.distanceTo(new THREE.Vector3(2, -1, 3)) < 1e-5);
  assert.ok(reopenedMesh.scale.distanceTo(new THREE.Vector3(1.5, 2, 0.75)) < 1e-5);
  assert.ok(Math.abs(THREE.MathUtils.radToDeg(reopenedMesh.rotation.y) - 45) < 1e-4);
  assert.equal(reopenedMesh.geometry.getAttribute("position").count, 3);
  assert.equal(reopenedMesh.material.name, "TestMetal");
  assert.ok(Math.abs(reopenedMesh.material.metalness - 0.65) < 1e-5);
  assert.ok(Math.abs(reopenedMesh.material.roughness - 0.25) < 1e-5);
});

test("texture-bearing GLB export embeds its image and reimports the material texture", async () => {
  const png = Buffer.from(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+j9qkAAAAASUVORK5CYII=",
    "base64",
  );
  const originalCreateElement = document.createElement.bind(document);
  const originalImageData = globalThis.ImageData;
  const originalCreateImageBitmap = globalThis.createImageBitmap;
  const originalSelf = globalThis.self;
  document.createElement = (tagName, options) => {
    if (tagName.toLowerCase() !== "canvas") return originalCreateElement(tagName, options);
    return {
      width: 1,
      height: 1,
      getContext: () => ({ putImageData() {}, drawImage() {}, translate() {}, scale() {} }),
      toBlob: (callback) => callback(new Blob([png], { type: "image/png" })),
    };
  };
  globalThis.ImageData = class ImageData {
    constructor(data, width, height) { Object.assign(this, { data, width, height }); }
  };
  globalThis.createImageBitmap = async (blob) => ({ width: 1, height: 1, source: blob, close() {} });
  globalThis.self = {
    URL: {
      createObjectURL: () => `data:image/png;base64,${png.toString("base64")}`,
      revokeObjectURL() {},
    },
  };

  try {
    const scene = new THREE.Scene();
    const geometry = new THREE.BoxGeometry(1, 1, 1);
    const pixels = new Uint8Array([220, 80, 30, 255]);
    const texture = new THREE.DataTexture(pixels, 1, 1, THREE.RGBAFormat);
    texture.colorSpace = THREE.SRGBColorSpace;
    texture.needsUpdate = true;
    const material = new THREE.MeshStandardMaterial({
      name: "PersistedTextureMaterial",
      map: texture,
      metalness: 0.35,
      roughness: 0.6,
    });
    const mesh = new THREE.Mesh(geometry, material);
    mesh.name = "TexturedMesh";
    scene.add(mesh);

    const exported = await new GLTFExporter().parseAsync(scene, { binary: true });
    assert.ok(exported instanceof ArrayBuffer);
    const view = new DataView(exported);
    assert.equal(view.getUint32(0, true), 0x46546c67);
    let offset = 12;
    const jsonLength = view.getUint32(offset, true);
    assert.equal(view.getUint32(offset + 4, true), 0x4e4f534a);
    const json = JSON.parse(new TextDecoder().decode(new Uint8Array(exported, offset + 8, jsonLength)).trim());
    offset += 8 + jsonLength;
    const binLength = view.getUint32(offset, true);
    assert.equal(view.getUint32(offset + 4, true), 0x004e4942);
    const bin = new Uint8Array(exported, offset + 8, binLength);
    assert.equal(json.images?.length, 1);
    assert.equal(json.textures?.length, 1);
    assert.equal(json.materials[0].pbrMetallicRoughness.baseColorTexture.index, 0);
    const imageView = json.bufferViews[json.images[0].bufferView];
    const embeddedPng = bin.subarray(imageView.byteOffset ?? 0, (imageView.byteOffset ?? 0) + imageView.byteLength);
    assert.deepEqual(Buffer.from(embeddedPng), png);

    const reopened = await new GLTFLoader().parseAsync(exported, "");
    const reopenedMesh = reopened.scene.getObjectByName("TexturedMesh");
    assert.ok(reopenedMesh instanceof THREE.Mesh);
    assert.ok(reopenedMesh.material.map?.isTexture);
    assert.equal(reopenedMesh.material.metalness, 0.35);
    assert.equal(reopenedMesh.material.roughness, 0.6);
  } finally {
    document.createElement = originalCreateElement;
    if (originalImageData === undefined) delete globalThis.ImageData;
    else globalThis.ImageData = originalImageData;
    if (originalCreateImageBitmap === undefined) delete globalThis.createImageBitmap;
    else globalThis.createImageBitmap = originalCreateImageBitmap;
    if (originalSelf === undefined) delete globalThis.self;
    else globalThis.self = originalSelf;
  }
});

test("edited GLB roundtrip preserves authored and imported animation clips, lights and Khronos validity", async () => {
  const loader = new GLTFLoader();
  const seed = await loader.parseAsync(makeTriangleGlb(), "");
  const seedMesh = seed.scene.getObjectByName("EditableTriangle");
  const importedClip = new THREE.AnimationClip("Imported spin", 2, [
    new THREE.QuaternionKeyframeTrack(`${seedMesh.name}.quaternion`, [0, 2], [0, 0, 0, 1, 0, 1, 0, 0]),
  ]);
  const importedGlb = await new GLTFExporter().parseAsync(seed.scene, { binary: true, animations: [importedClip] });
  const source = await loader.parseAsync(importedGlb, "");
  const mesh = source.scene.getObjectByName("EditableTriangle");
  assert.ok(mesh);
  assert.equal(source.animations[0].name, "Imported spin");
  const tracks = [
    new THREE.VectorKeyframeTrack(`${mesh.name}.position`, [0, 1], [0, 0, 0, 2, 1, 0]),
    new THREE.QuaternionKeyframeTrack(`${mesh.name}.quaternion`, [0, 1], [0, 0, 0, 1, 0, 0.7071068, 0, 0.7071068]),
    new THREE.VectorKeyframeTrack(`${mesh.name}.scale`, [0, 1], [1, 1, 1, 2, 2, 2]),
  ];
  const clip = new THREE.AnimationClip("Move triangle", 1, tracks);
  const mixer = new THREE.AnimationMixer(source.scene);
  mixer.clipAction(clip).play();
  mixer.setTime(0.5);
  assert.ok(Math.abs(mesh.position.x - 1) < 1e-5, "scrubbing the mixer applies the position keyframe");
  assert.ok(Math.abs(mesh.scale.x - 1.5) < 1e-5, "scrubbing the mixer applies the scale keyframe");
  const exportScene = new THREE.Scene();
  exportScene.add(source.scene.clone(true));
  const light = new THREE.PointLight("#ffcc88", 3);
  light.name = "studio-light-test";
  light.position.set(2, 4, 1);
  exportScene.add(light);
  const directional = new THREE.DirectionalLight("#cceeff", 1.25);
  directional.name = "studio-sun-test";
  directional.position.set(-3, 5, 2);
  exportScene.add(directional);
  const exported = await new GLTFExporter().parseAsync(exportScene, { binary: true, animations: [clip, importedClip] });
  const report = await validator.validateBytes(new Uint8Array(exported), { uri: "openmindai-roundtrip.glb" });
  assert.equal(report.issues.numErrors, 0, JSON.stringify(report.issues.messages));
  const reopened = await new GLTFLoader().parseAsync(exported, "");
  assert.equal(reopened.animations.length, 2);
  assert.equal(reopened.animations[0].name, "Move triangle");
  assert.equal(reopened.animations[0].tracks.length, 3);
  assert.equal(reopened.animations[0].duration, 1);
  assert.equal(reopened.animations[1].name, "Imported spin");
  assert.equal(reopened.animations[1].duration, 2);
  const playbackMesh = reopened.scene.getObjectByName("EditableTriangle");
  const playback = new THREE.AnimationMixer(reopened.scene);
  playback.clipAction(reopened.animations[0]).play();
  playback.setTime(0.5);
  assert.ok(Math.abs(playbackMesh.position.x - 1) < 1e-4, "authored position keys are present and playable after reimport");
  assert.ok(Math.abs(playbackMesh.scale.x - 1.5) < 1e-4, "authored scale keys are present and playable after reimport");
  assert.match(reopened.animations[0].tracks[0].name, /position/);
  const jsonLength = new DataView(exported).getUint32(12, true);
  const json = JSON.parse(new TextDecoder().decode(new Uint8Array(exported, 20, jsonLength)).trim());
  assert.ok(json.extensionsUsed?.includes("KHR_lights_punctual"));
  assert.equal(json.extensions?.KHR_lights_punctual?.lights?.[0]?.type, "point");
  assert.equal(json.extensions?.KHR_lights_punctual?.lights?.[1]?.type, "directional");
});
