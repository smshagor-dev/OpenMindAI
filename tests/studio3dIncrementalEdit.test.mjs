import test from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import { formatError } from "../src/lib/format.ts";
import { cloudSceneEditTarget, createBalconyResizePlan, createRoomAwareSofaMovePlan, createStaircaseMovePlan, createWindowAssembly, createWindowOpeningGeometry } from "../src/lib/studio3dSceneEdit.ts";

test("Cloud scene reasoning receives only the selected object's editable fields", () => {
  const selected = {
    name: "cube", parent: "Scene", kind: "mesh",
    dimensions: [1, 1, 1], position: [0, 0, 0], rotation: [0, 0, 0],
    color: [0.8, 0.1, 0.1], metallic: 0, roughness: 0.7,
    worldBounds: { min: [-0.5, -0.5, -0.5], max: [0.5, 0.5, 0.5] },
    spatialContext: { privateGeometryData: "local-only" },
  };
  const payload = cloudSceneEditTarget(selected);
  assert.deepEqual(payload, {
    name: "cube", parent: "Scene", kind: "mesh",
    dimensions: [1, 1, 1], position: [0, 0, 0], rotation: [0, 0, 0],
    color: [0.8, 0.1, 0.1], metallic: 0, roughness: 0.7,
  });
  assert.equal("worldBounds" in payload, false);
  assert.equal("spatialContext" in payload, false);
});

test("Desktop displays the message from a serialized native command error", () => {
  assert.equal(formatError({ code: "INTERNAL", message: "Cloud chat gateway returned no text scene response" }), "Cloud chat gateway returned no text scene response");
});

function assertHasOpening(geometry, horizontalAxis, halfWidth = 0.59, halfHeight = 0.56) {
  const position = geometry.getAttribute("position");
  const index = geometry.index;
  assert.ok(index, "split wall is indexed");
  for (let i = 0; i < index.count; i += 3) {
    const a = new THREE.Vector3().fromBufferAttribute(position, index.getX(i));
    const b = new THREE.Vector3().fromBufferAttribute(position, index.getX(i + 1));
    const c = new THREE.Vector3().fromBufferAttribute(position, index.getX(i + 2));
    const center = a.add(b).add(c).multiplyScalar(1 / 3);
    assert.ok(!(Math.abs(center.getComponent(horizontalAxis)) < halfWidth && Math.abs(center.y) < halfHeight), "no triangle spans the opening center");
  }
}

test("AI window edit creates a true opening through an X-aligned box wall", () => {
  const original = new THREE.BoxGeometry(4, 3, 0.2);
  const originalIndexCount = original.index.count;
  const edited = createWindowOpeningGeometry(original);
  assert.equal(edited.boundingBox.max.x - edited.boundingBox.min.x, 4);
  assert.equal(edited.boundingBox.max.y - edited.boundingBox.min.y, 3);
  assert.ok(edited.index.count > originalIndexCount);
  assertHasOpening(edited, 0);
  original.dispose();
  edited.dispose();
});

test("AI window edit handles a Z-aligned wall and rejects non-wall geometry", () => {
  const wall = new THREE.BoxGeometry(0.2, 3, 4);
  const edited = createWindowOpeningGeometry(wall);
  assertHasOpening(edited, 2);
  assert.throws(() => createWindowOpeningGeometry(new THREE.SphereGeometry(1)), /box-shaped wall/);
  wall.dispose();
  edited.dispose();
});

test("window assembly creates independently named frame bars and a glass pane without changing source bounds", () => {
  const wall = new THREE.BoxGeometry(4, 3, 0.2);
  const originalPosition = wall.getAttribute("position").array.slice();
  const opening = createWindowOpeningGeometry(wall);
  const assembly = createWindowAssembly(wall);
  const meshes = [];
  assembly.traverse((node) => { if (node instanceof THREE.Mesh) meshes.push(node); });
  assert.equal(meshes.length, 5, "pane plus four separately editable frame bars");
  assert.deepEqual(new Set(meshes.map((mesh) => mesh.name)), new Set([
    "Window Glass Pane", "Window Frame Top", "Window Frame Sill", "Window Frame Left", "Window Frame Right",
  ]));
  const pane = meshes.find((mesh) => mesh.name === "Window Glass Pane");
  assert.ok(pane.material instanceof THREE.MeshPhysicalMaterial);
  assert.equal(pane.material.transparent, true);
  assert.deepEqual(wall.getAttribute("position").array, originalPosition, "planning the operation does not mutate source geometry");
  assertHasOpening(opening, 0, 0.57, 0.26);
  opening.dispose();
  assembly.traverse((node) => { if (node instanceof THREE.Mesh) { node.geometry.dispose(); node.material.dispose(); } });
  wall.dispose();
});

test("window edit rejects a too-small structural boundary before producing any assembly", () => {
  const wall = new THREE.BoxGeometry(1.1, 3, 0.2);
  assert.throws(() => createWindowAssembly(wall), /box-shaped wall/);
  wall.dispose();
});

test("a window opening fits an existing 1.2 m tall facade section with wall-edge clearance", () => {
  const wall = new THREE.BoxGeometry(2.775, 1.2, 0.24);
  const opening = createWindowOpeningGeometry(wall);
  assert.ok(opening.index.count > wall.index.count);
  assertHasOpening(opening, 0, 0.57, 0.26);
  opening.dispose();
  wall.dispose();
});

function translatedBox(name, dimensions, center) {
  const geometry = new THREE.BoxGeometry(...dimensions);
  geometry.translate(...center);
  const mesh = new THREE.Mesh(geometry, new THREE.MeshStandardMaterial());
  mesh.name = name;
  return mesh;
}

test("balcony resize plans one atomic slab, rail and baluster transaction and restores exact matrices", () => {
  const root = new THREE.Group();
  const upper = new THREE.Group();
  upper.name = "Upper_Floor";
  root.add(upper);
  const slab = translatedBox("Balcony_slab", [4.2, 0.18, 1.55], [0, 3.4, 5.72]);
  upper.add(slab, translatedBox("Balcony_front_railing", [4.2, 0.08, 0.08], [0, 4, 6.47]));
  upper.add(translatedBox("Balcony_side_railing", [0.08, 0.08, 1.5], [-2.06, 4, 5.72]));
  upper.add(translatedBox("Balcony_side_railing_2", [0.08, 0.08, 1.5], [2.06, 4, 5.72]));
  for (let i = 0; i < 17; i++) upper.add(translatedBox(`Balcony_baluster_${String(i + 1).padStart(2, "0")}`, [0.035, 0.58, 0.035], [-2 + i * 0.25, 3.68, 6.47]));
  root.updateMatrixWorld(true);
  const before = new Map();
  upper.traverse((object) => { if (object instanceof THREE.Mesh) { object.updateMatrix(); before.set(object, object.matrix.clone()); } });
  const changes = createBalconyResizePlan(root, slab, 4.5);
  assert.equal(changes.length, 21);
  for (const change of changes) {
    change.target.matrix.copy(change.after);
    change.after.decompose(change.target.position, change.target.quaternion, change.target.scale);
    change.target.updateMatrix();
  }
  root.updateMatrixWorld(true);
  const slabBounds = new THREE.Box3().setFromObject(slab);
  assert.ok(Math.abs((slabBounds.max.x - slabBounds.min.x) - 4.5) < 1e-5);
  const left = new THREE.Box3().setFromObject(upper.getObjectByName("Balcony_side_railing"));
  const right = new THREE.Box3().setFromObject(upper.getObjectByName("Balcony_side_railing_2"));
  assert.ok(Math.abs(left.getCenter(new THREE.Vector3()).x - slabBounds.min.x - 0.04) < 1e-5);
  assert.ok(Math.abs(slabBounds.max.x - right.getCenter(new THREE.Vector3()).x - 0.04) < 1e-5);
  const first = new THREE.Box3().setFromObject(upper.getObjectByName("Balcony_baluster_01"));
  const last = new THREE.Box3().setFromObject(upper.getObjectByName("Balcony_baluster_17"));
  assert.ok(first.getCenter(new THREE.Vector3()).x > slabBounds.min.x && last.getCenter(new THREE.Vector3()).x < slabBounds.max.x);
  for (const change of changes) {
    change.target.matrix.copy(change.before);
    change.before.decompose(change.target.position, change.target.quaternion, change.target.scale);
    change.target.updateMatrix();
  }
  for (const [object, matrix] of before) assert.deepEqual(object.matrix.elements, matrix.elements, `${object.name} restored exactly`);
  assert.throws(() => createBalconyResizePlan(root, slab, 4.1), /increase by 2% to 35%/);
  for (const [object, matrix] of before) assert.deepEqual(object.matrix.elements, matrix.elements, `${object.name} unchanged after rejected plan`);
});

function addDoorwayFrames(root, prefix, center, widthAxis) {
  const horizontal = widthAxis === "x";
  const offset = horizontal ? [0.45, 1.25, 0] : [0, 1.25, 0.45];
  for (const side of [-1, 1]) {
    const position = center.map((value, axis) => value + offset[axis] * side);
    root.add(translatedBox(`${prefix}_door_frame_${side < 0 ? "left" : "right"}`, [0.07, 2.05, 0.07], position));
  }
}

test("sofa move stays inside its room, clears doorway and furniture, and rejects blocked placements", () => {
  const root = new THREE.Group();
  root.add(translatedBox("Living_Room_floor", [6, 0.04, 4], [0, 0.22, 1.5]));
  const room = new THREE.Group(); room.name = "Living_Room"; root.add(room);
  const sofa = translatedBox("Living_Room_Sofa", [2.3, 0.78, 0.88], [0, 0.63, 2.9]); room.add(sofa);
  room.add(translatedBox("Living_Room_Coffee_table", [1, 0.42, 0.56], [0, 0.45, 1.3]));
  room.add(translatedBox("Living_Room_Floor_lamp", [0.22, 1.55, 0.22], [2.67, 1.0, 3.25]));
  room.add(translatedBox("Living_Room_Book_shelf", [0.34, 1.65, 0.85], [-2.72, 1.0, 1.5]));
  addDoorwayFrames(root, "ground_partition_4", [0, 1.25, 3.5], "x");
  addDoorwayFrames(root, "ground_partition_3", [0, 1.25, -0.5], "x");
  root.updateMatrixWorld(true);
  const before = sofa.matrix.clone();
  const move = createRoomAwareSofaMovePlan(root, sofa, [-1.77, 0, 0]);
  assert.equal(move.target, sofa);
  assert.deepEqual(sofa.matrix.elements, before.elements, "preflight must not mutate the scene");
  assert.throws(() => createRoomAwareSofaMovePlan(root, sofa, [0, 0, 0]), /too small/);
  assert.throws(() => createRoomAwareSofaMovePlan(root, sofa, [-1.3, 0, 0]), /doorway|overlap/);
});

test("staircase group move preserves lower and upper floor connections and rejects a disconnected shift", () => {
  const root = new THREE.Group();
  root.add(translatedBox("Ground_floor_slab", [12, 0.06, 10], [0, 0.21, 0]));
  root.add(translatedBox("Stair_Landing_floor", [1.44, 0.04, 1.44], [2, 3.38, 1]));
  root.add(translatedBox("ground_front_wooden_door_leaf", [0.86, 2.03, 0.04], [0, 1.25, 5]));
  const stairs = new THREE.Group(); stairs.name = "Staircase"; root.add(stairs);
  for (let i = 0; i < 12; i++) stairs.add(translatedBox(`Stair_tread_${String(i + 1).padStart(2, "0")}`, [1.05, 0.22, 0.3], [2, 0.3 + i * 0.26, -2.9 + i * 0.3]));
  stairs.add(translatedBox("Stair_landing", [1.5, 0.18, 1.5], [2, 3.27, 1]));
  root.updateMatrixWorld(true);
  const before = stairs.matrix.clone();
  const move = createStaircaseMovePlan(root, stairs, [0, 0, 0.1]);
  assert.equal(move.length, 13, "treads and landing share one atomic move transaction");
  assert.ok(move.every((change) => change.target.parent === stairs));
  assert.deepEqual(stairs.matrix.elements, before.elements, "preflight must not mutate the scene");
  const firstTreadBefore = stairs.getObjectByName("Stair_tread_01").matrix.clone();
  for (const change of move) {
    change.target.matrix.copy(change.after);
    change.target.matrix.decompose(change.target.position, change.target.quaternion, change.target.scale);
    change.target.updateMatrix();
  }
  root.updateMatrixWorld(true);
  assert.ok(Math.abs(stairs.getObjectByName("Stair_tread_01").position.z - (firstTreadBefore.elements[14] + 0.1)) < 1e-6);
  assert.throws(() => createStaircaseMovePlan(root, stairs, [0, 0, -0.1]), /closer/);
  assert.throws(() => createStaircaseMovePlan(root, stairs, [0, 0, 0.3]), /bounded/);
});
