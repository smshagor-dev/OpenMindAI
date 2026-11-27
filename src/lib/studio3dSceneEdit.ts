import * as THREE from "three";
import { mergeGeometries } from "three/examples/jsm/utils/BufferGeometryUtils.js";

export type WindowLayout = {
  horizontalAxis: 0 | 2;
  center: THREE.Vector3;
  wallWidth: number;
  wallHeight: number;
  wallDepth: number;
  openingWidth: number;
  openingHeight: number;
  frameWidth: number;
};

export type SceneMatrixChange = {
  target: THREE.Object3D;
  before: THREE.Matrix4;
  after: THREE.Matrix4;
};

export type CloudSceneEditTarget = {
  name: string;
  parent: string | null;
  kind: "mesh" | "container";
  dimensions: [number, number, number];
  position: [number, number, number];
  rotation: [number, number, number];
  color: [number, number, number];
  metallic: number;
  roughness: number;
};

/** Strip local geometry and spatial-analysis context before a hosted reasoning call. */
export function cloudSceneEditTarget(target: CloudSceneEditTarget): CloudSceneEditTarget {
  return {
    name: target.name,
    parent: target.parent,
    kind: target.kind,
    dimensions: [...target.dimensions],
    position: [...target.position],
    rotation: [...target.rotation],
    color: [...target.color],
    metallic: target.metallic,
    roughness: target.roughness,
  };
}

function findSceneObjectByExactName(root: THREE.Object3D, name: string): THREE.Object3D | null {
  let result: THREE.Object3D | null = null;
  root.traverse((object) => { if (result === null && object.name === name) result = object; });
  return result as THREE.Object3D | null;
}

/** Build a bounded, atomic resize plan for the named balcony assembly. */
export function createBalconyResizePlan(
  root: THREE.Object3D,
  slab: THREE.Object3D,
  requestedWidth: number,
): SceneMatrixChange[] {
  const exactSlab = findSceneObjectByExactName(root, "Balcony_slab");
  if (!exactSlab || slab !== exactSlab || !(slab instanceof THREE.Mesh))
    throw new Error("Select the Balcony_slab mesh before resizing the balcony assembly.");
  slab.updateWorldMatrix(true, false);
  const slabBounds = new THREE.Box3().setFromObject(slab);
  const width = slabBounds.max.x - slabBounds.min.x;
  if (!Number.isFinite(requestedWidth) || width < 1 || requestedWidth < width * 1.02 || requestedWidth > width * 1.35)
    throw new Error("The AI balcony width must increase by 2% to 35%; the assembly was left unchanged.");
  const parent = slab.parent;
  if (!parent) throw new Error("The balcony slab has no floor attachment.");
  const meshes: THREE.Mesh[] = [];
  root.traverse((object) => {
    if (object instanceof THREE.Mesh && (object.name === "Balcony_slab" || object.name === "Balcony_front_railing"
      || object.name === "Balcony_side_railing" || object.name === "Balcony_side_railing_2" || /^Balcony_baluster_\d+$/.test(object.name))) meshes.push(object);
  });
  const front = meshes.find((object) => object.name === "Balcony_front_railing");
  const sides = meshes.filter((object) => object.name === "Balcony_side_railing" || object.name === "Balcony_side_railing_2");
  const balusters = meshes.filter((object) => /^Balcony_baluster_\d+$/.test(object.name)).sort((a, b) => a.name.localeCompare(b.name, undefined, { numeric: true }));
  if (!front || sides.length !== 2 || balusters.length < 4 || meshes.length !== 1 + 1 + 2 + balusters.length
    || meshes.some((object) => object.parent !== parent))
    throw new Error("Balcony dependent rails or balusters are missing or detached; no changes were applied.");
  const matrixIsAxisAligned = (object: THREE.Object3D) => Math.abs(object.rotation.x) < 0.001 && Math.abs(object.rotation.y) < 0.001 && Math.abs(object.rotation.z) < 0.001;
  if (meshes.some((object) => !matrixIsAxisAligned(object)) || !matrixIsAxisAligned(parent))
    throw new Error("The balcony assembly is rotated; its axis-aligned attachment could not be validated.");
  const bounds = new Map<THREE.Object3D, THREE.Box3>(meshes.map((object) => [object, new THREE.Box3().setFromObject(object)]));
  const slabBox = bounds.get(slab)!;
  const frontBox = bounds.get(front)!;
  if (Math.abs((frontBox.max.x - frontBox.min.x) - width) > 0.15
    || Math.abs(frontBox.max.y - slabBox.max.y) > 0.8
    || sides.some((rail) => Math.min(Math.abs(bounds.get(rail)!.getCenter(new THREE.Vector3()).x - slabBox.min.x), Math.abs(bounds.get(rail)!.getCenter(new THREE.Vector3()).x - slabBox.max.x)) > 0.15)
    || balusters.some((post) => bounds.get(post)!.getCenter(new THREE.Vector3()).x <= slabBox.min.x || bounds.get(post)!.getCenter(new THREE.Vector3()).x >= slabBox.max.x))
    throw new Error("The current balcony rails are not aligned with the slab; no changes were applied.");

  const factor = requestedWidth / width;
  const changes: SceneMatrixChange[] = [];
  const matrixFor = (object: THREE.Object3D, transform: (position: THREE.Vector3, scale: THREE.Vector3) => void) => {
    object.updateMatrix();
    const before = object.matrix.clone();
    const position = object.position.clone();
    const scale = object.scale.clone();
    transform(position, scale);
    const after = new THREE.Matrix4().compose(position, object.quaternion, scale);
    changes.push({ target: object, before, after });
  };
  const scaleAboutCenter = (object: THREE.Object3D, box: THREE.Box3) => {
    const center = parent.worldToLocal(box.getCenter(new THREE.Vector3()));
    matrixFor(object, (position, scale) => {
      const priorScale = scale.x;
      scale.x *= factor;
      position.x = center.x - (center.x - position.x) * factor;
      if (!Number.isFinite(priorScale) || priorScale <= 0) throw new Error("The balcony assembly has an invalid scale.");
    });
  };
  scaleAboutCenter(slab, slabBox);
  scaleAboutCenter(front, frontBox);

  const slabCenter = slabBox.getCenter(new THREE.Vector3()).x;
  const newMin = slabCenter - requestedWidth / 2;
  const newMax = slabCenter + requestedWidth / 2;
  for (const rail of sides) {
    const railCenter = bounds.get(rail)!.getCenter(new THREE.Vector3()).x;
    const oldEdge = railCenter < slabCenter ? slabBox.min.x : slabBox.max.x;
    const newEdge = railCenter < slabCenter ? newMin : newMax;
    const inset = Math.abs(railCenter - oldEdge);
    const desired = railCenter < slabCenter ? newEdge + inset : newEdge - inset;
    const delta = desired - railCenter;
    matrixFor(rail, (position) => { position.x += delta; });
  }
  const sortedCenters = balusters.map((object) => bounds.get(object)!.getCenter(new THREE.Vector3()).x);
  const insetLeft = sortedCenters[0] - slabBox.min.x;
  const insetRight = slabBox.max.x - sortedCenters[sortedCenters.length - 1];
  for (let index = 0; index < balusters.length; index++) {
    const desired = newMin + insetLeft + index * (requestedWidth - insetLeft - insetRight) / (balusters.length - 1);
    const delta = desired - sortedCenters[index];
    matrixFor(balusters[index], (position) => { position.x += delta; });
  }
  if (changes.length !== meshes.length) throw new Error("The balcony group transaction is incomplete; no changes were applied.");
  return changes;
}

function objectBox(root: THREE.Object3D, exactName: string): { object: THREE.Object3D; bounds: THREE.Box3 } | null {
  const result = findSceneObjectByExactName(root, exactName);
  if (!result) return null;
  result.updateWorldMatrix(true, false);
  return { object: result, bounds: new THREE.Box3().setFromObject(result) };
}

/** Convert named door frames into horizontal walk-through clearance volumes. */
export function findDoorwayClearanceBounds(root: THREE.Object3D, roomBounds: THREE.Box3): THREE.Box3[] {
  const leftFrames = new Map<string, THREE.Object3D>();
  root.traverse((object) => {
    const match = object.name.match(/^(.+)_door_frame_left$/i);
    if (match) leftFrames.set(match[1], object);
  });
  const clearances: THREE.Box3[] = [];
  for (const [prefix, left] of leftFrames) {
    const right = root.getObjectByName(`${prefix}_door_frame_right`);
    if (!right) continue;
    const opening = new THREE.Box3().setFromObject(left).union(new THREE.Box3().setFromObject(right));
    const widthX = opening.max.x - opening.min.x;
    const widthZ = opening.max.z - opening.min.z;
    const corridor = opening.clone().expandByVector(widthX >= widthZ
      ? new THREE.Vector3(0.12, 0, 0.42)
      : new THREE.Vector3(0.42, 0, 0.12));
    if (corridor.max.y < roomBounds.min.y || corridor.min.y > roomBounds.max.y + 2.1) continue;
    const roomHorizontal = new THREE.Box3(
      new THREE.Vector3(roomBounds.min.x, corridor.min.y, roomBounds.min.z),
      new THREE.Vector3(roomBounds.max.x, corridor.max.y, roomBounds.max.z),
    );
    if (corridor.intersectsBox(roomHorizontal)) clearances.push(corridor);
  }
  return clearances;
}

/** Preflight a sofa translation against its room, doorways and current furniture. */
export function createRoomAwareSofaMovePlan(
  root: THREE.Object3D,
  sofa: THREE.Object3D,
  requestedPosition: [number, number, number],
): SceneMatrixChange {
  if (!/sofa/i.test(sofa.name) || sofa.parent?.name !== "Living_Room")
    throw new Error("Select the sofa inside Living_Room before requesting this move.");
  const floor = objectBox(root, "Living_Room_floor");
  if (!floor || !(floor.object instanceof THREE.Mesh)) throw new Error("The living-room floor boundary is unavailable.");
  if (!requestedPosition.every(Number.isFinite) || Math.abs(requestedPosition[1] - sofa.position.y) > 0.05)
    throw new Error("The sofa move must preserve its floor height.");
  const floorBounds = floor.bounds;
  const currentBounds = new THREE.Box3().setFromObject(sofa);
  const parent = sofa.parent;
  if (!parent) throw new Error("The sofa has no room parent.");
  parent.updateWorldMatrix(true, false);
  const oldWorldPosition = parent.localToWorld(sofa.position.clone());
  const newWorldPosition = parent.localToWorld(new THREE.Vector3(...requestedPosition));
  const delta = newWorldPosition.sub(oldWorldPosition);
  const movedBounds = currentBounds.clone().translate(delta);
  const margin = 0.08;
  if (movedBounds.min.x < floorBounds.min.x + margin || movedBounds.max.x > floorBounds.max.x - margin
    || movedBounds.min.z < floorBounds.min.z + margin || movedBounds.max.z > floorBounds.max.z - margin)
    throw new Error("The sofa move would leave the living-room floor boundary.");
  if (delta.length() < 0.25) throw new Error("The sofa move is too small to count as a room rearrangement.");
  const clearanceBounds = findDoorwayClearanceBounds(root, floorBounds);
  if (clearanceBounds.some((box) => movedBounds.intersectsBox(box)))
    throw new Error("The sofa move would block a living-room doorway clearance area.");
  const collisions: string[] = [];
  parent.traverse((object) => {
    if (object === sofa || !(object instanceof THREE.Mesh)) return;
    if (movedBounds.intersectsBox(new THREE.Box3().setFromObject(object))) collisions.push(object.name || object.type);
  });
  if (collisions.length) throw new Error(`The sofa move would overlap ${collisions.slice(0, 3).join(", ")}.`);
  const distanceToWall = (box: THREE.Box3) => Math.min(
    box.getCenter(new THREE.Vector3()).x - floorBounds.min.x,
    floorBounds.max.x - box.getCenter(new THREE.Vector3()).x,
    box.getCenter(new THREE.Vector3()).z - floorBounds.min.z,
    floorBounds.max.z - box.getCenter(new THREE.Vector3()).z,
  );
  if (distanceToWall(movedBounds) > distanceToWall(currentBounds) - 0.05 && delta.length() < 0.5)
    throw new Error("The sofa move does not take it meaningfully toward an opposite room wall.");
  sofa.updateMatrix();
  const before = sofa.matrix.clone();
  const after = new THREE.Matrix4().compose(new THREE.Vector3(...requestedPosition), sofa.quaternion, sofa.scale);
  return { target: sofa, before, after };
}

/** Keep the staircase group coherent and attached to both floor connections. */
export function createStaircaseMovePlan(
  root: THREE.Object3D,
  staircase: THREE.Object3D,
  requestedPosition: [number, number, number],
): SceneMatrixChange[] {
  if (staircase.name !== "Staircase" || !staircase.parent)
    throw new Error("Select the complete Staircase assembly before moving it.");
  const members: THREE.Mesh[] = [];
  staircase.traverse((object) => { if (object instanceof THREE.Mesh) members.push(object); });
  const treads = members.filter((object) => /^Stair_tread_\d+$/.test(object.name));
  const landing = members.find((object) => object.name === "Stair_landing");
  const upperFloor = objectBox(root, "Stair_Landing_floor");
  const groundFloor = objectBox(root, "Ground_floor_slab");
  const entrance = objectBox(root, "ground_front_wooden_door_leaf");
  if (members.length < 4 || treads.length < 4 || !landing || !upperFloor || !groundFloor || !entrance)
    throw new Error("The stair treads, floor landings or entrance reference are incomplete.");
  if (!requestedPosition.every(Number.isFinite) || Math.abs(requestedPosition[1] - staircase.position.y) > 0.02)
    throw new Error("The staircase move must preserve its vertical floor connection.");
  const parent = staircase.parent;
  parent.updateWorldMatrix(true, false);
  const originalPosition = parent.localToWorld(staircase.position.clone());
  const proposedPosition = parent.localToWorld(new THREE.Vector3(...requestedPosition));
  const delta = proposedPosition.sub(originalPosition);
  if (delta.length() < 0.025 || delta.length() > 0.2)
    throw new Error("The staircase move must be a small, bounded adjustment that can preserve both landings.");
  const originalBounds = new THREE.Box3().setFromObject(staircase);
  const movedBounds = originalBounds.clone().translate(delta);
  const originalLanding = new THREE.Box3().setFromObject(landing);
  const movedLanding = originalLanding.clone().translate(delta);
  const overlapRatio = (a: THREE.Box3, b: THREE.Box3) => {
    const overlapX = Math.max(0, Math.min(a.max.x, b.max.x) - Math.max(a.min.x, b.min.x));
    const overlapZ = Math.max(0, Math.min(a.max.z, b.max.z) - Math.max(a.min.z, b.min.z));
    const area = Math.min((a.max.x - a.min.x) * (a.max.z - a.min.z), (b.max.x - b.min.x) * (b.max.z - b.min.z));
    return area > 0 ? overlapX * overlapZ / area : 0;
  };
  if (overlapRatio(movedLanding, upperFloor.bounds) < 0.8)
    throw new Error("The proposed move would disconnect the stair landing from the upper floor.");
  const firstTread = treads.slice().sort((a, b) => a.name.localeCompare(b.name, undefined, { numeric: true }))[0];
  const treadBounds = new THREE.Box3().setFromObject(firstTread).translate(delta);
  if (treadBounds.max.y < groundFloor.bounds.min.y - 0.05 || treadBounds.min.y > groundFloor.bounds.max.y + 0.35
    || treadBounds.min.x < groundFloor.bounds.min.x || treadBounds.max.x > groundFloor.bounds.max.x
    || treadBounds.min.z < groundFloor.bounds.min.z || treadBounds.max.z > groundFloor.bounds.max.z)
    throw new Error("The proposed move would disconnect the first stair tread from the ground floor.");
  const entranceCenter = entrance.bounds.getCenter(new THREE.Vector3());
  const distanceToEntrance = (box: THREE.Box3) => {
    const center = box.getCenter(new THREE.Vector3());
    return Math.hypot(center.x - entranceCenter.x, center.z - entranceCenter.z);
  };
  if (distanceToEntrance(movedBounds) > distanceToEntrance(originalBounds) - 0.02)
    throw new Error("The proposed staircase transform does not move closer to the main entrance.");
  const allBuilding = new THREE.Box3().setFromObject(root);
  const doors = findDoorwayClearanceBounds(root, allBuilding);
  if (doors.some((box) => movedBounds.intersectsBox(box) && !originalBounds.intersectsBox(box)))
    throw new Error("The proposed move would newly block a doorway clearance area.");
  // Exporters and imported GLB hierarchies can omit a transform on empty group
  // nodes. Move the editable treads and landing together so the assembly change
  // is represented by durable mesh transforms while preserving their parentage.
  return members.map((member) => {
    member.updateMatrix();
    const before = member.matrix.clone();
    const memberParent = member.parent;
    if (!memberParent) throw new Error(`Stair component ${member.name} lost its parent during preflight.`);
    memberParent.updateWorldMatrix(true, false);
    const destinationWorld = memberParent.localToWorld(member.position.clone()).add(delta);
    const destinationLocal = memberParent.worldToLocal(destinationWorld);
    const after = new THREE.Matrix4().compose(destinationLocal, member.quaternion, member.scale);
    return { target: member, before, after };
  });
}

/** Validate the selected mesh and keep a structural margin around the cut. */
export function getWindowLayout(
  source: THREE.BufferGeometry,
  maxOpeningWidth = 1.2,
  maxOpeningHeight = 1.15,
): WindowLayout {
  if (!source.boundingBox) source.computeBoundingBox();
  const bounds = source.boundingBox;
  if (!bounds) throw new Error("The selected wall has no measurable geometry.");
  const size = [bounds.max.x - bounds.min.x, bounds.max.y - bounds.min.y, bounds.max.z - bounds.min.z];
  const horizontalAxis = (size[0] >= size[2] ? 0 : 2) as 0 | 2;
  const depthAxis = horizontalAxis === 0 ? 2 : 0;
  const wallWidth = size[horizontalAxis];
  const wallHeight = size[1];
  const wallDepth = size[depthAxis];
  if (![...size, maxOpeningWidth, maxOpeningHeight].every(Number.isFinite)
    || wallWidth < 1.2 || wallHeight < 0.9 || wallDepth <= 0
    || Math.min(size[0], size[2]) > Math.max(size[0], size[2]) * 0.35) {
    throw new Error("The selected mesh does not look like a box-shaped wall suitable for a through-opening.");
  }
  const openingWidth = Math.min(maxOpeningWidth, wallWidth * 0.42);
  const openingHeight = Math.min(maxOpeningHeight, wallHeight * 0.46);
  const frameWidth = Math.min(0.075, openingWidth * 0.08, openingHeight * 0.08);
  if (openingWidth < 0.6 || openingHeight < 0.4 || frameWidth < 0.025
    || (wallWidth - openingWidth) / 2 < 0.12 || (wallHeight - openingHeight) / 2 < 0.12) {
    throw new Error("The requested window would violate the minimum wall edge clearance.");
  }
  return { horizontalAxis, center: bounds.getCenter(new THREE.Vector3()), wallWidth, wallHeight, wallDepth, openingWidth, openingHeight, frameWidth };
}

/** Split an axis-aligned box wall into four pieces around a through-opening. */
export function createWindowOpeningGeometry(
  source: THREE.BufferGeometry,
  maxOpeningWidth = 1.2,
  maxOpeningHeight = 1.15,
): THREE.BufferGeometry {
  const layout = getWindowLayout(source, maxOpeningWidth, maxOpeningHeight);
  const { horizontalAxis, center, wallWidth, wallHeight, wallDepth, openingWidth, openingHeight } = layout;
  const sideWidth = (wallWidth - openingWidth) / 2;
  const capHeight = (wallHeight - openingHeight) / 2;
  const blocks: THREE.BufferGeometry[] = [];
  const appendBlock = (width: number, height: number, hOffset: number, yOffset: number) => {
    const block = new THREE.BoxGeometry(width, height, wallDepth);
    if (horizontalAxis === 2) block.rotateY(Math.PI / 2);
    const position = center.clone();
    position.setComponent(horizontalAxis, center.getComponent(horizontalAxis) + hOffset);
    position.y += yOffset;
    block.translate(position.x, position.y, position.z);
    blocks.push(block);
  };
  appendBlock(sideWidth, wallHeight, -(openingWidth + sideWidth) / 2, 0);
  appendBlock(sideWidth, wallHeight, (openingWidth + sideWidth) / 2, 0);
  appendBlock(openingWidth, capHeight, 0, (openingHeight + capHeight) / 2);
  appendBlock(openingWidth, capHeight, 0, -(openingHeight + capHeight) / 2);
  const result = mergeGeometries(blocks, false);
  blocks.forEach((block) => block.dispose());
  if (!result) throw new Error("The selected wall geometry could not be split around the opening.");
  result.computeVertexNormals();
  result.computeBoundingBox();
  return result;
}

/** Create separate editable frame and glass meshes in the source wall's local frame. */
export function createWindowAssembly(source: THREE.BufferGeometry): THREE.Group {
  const layout = getWindowLayout(source);
  const { horizontalAxis, center, openingWidth, openingHeight, wallDepth, frameWidth } = layout;
  const assembly = new THREE.Group();
  assembly.name = "AI Window Assembly";
  const frame = new THREE.Group();
  frame.name = "Window Frame";
  const pane = new THREE.Mesh(
    new THREE.BoxGeometry(
      horizontalAxis === 0 ? openingWidth - frameWidth * 1.4 : wallDepth * 0.12,
      openingHeight - frameWidth * 1.4,
      horizontalAxis === 0 ? wallDepth * 0.12 : openingWidth - frameWidth * 1.4,
    ),
    new THREE.MeshPhysicalMaterial({ color: 0x9dd9e8, roughness: 0.12, metalness: 0, transparent: true, opacity: 0.38, side: THREE.DoubleSide }),
  );
  pane.name = "Window Glass Pane";
  pane.position.copy(center);
  assembly.add(pane);

  const halfW = openingWidth / 2;
  const halfH = openingHeight / 2;
  const barDepth = wallDepth * 1.18;
  const horizontalSize = horizontalAxis === 0
    ? new THREE.Vector3(halfW * 2, frameWidth, barDepth)
    : new THREE.Vector3(barDepth, frameWidth, halfW * 2);
  const verticalSize = horizontalAxis === 0
    ? new THREE.Vector3(frameWidth, halfH * 2, barDepth)
    : new THREE.Vector3(barDepth, halfH * 2, frameWidth);
  const frameMaterial = new THREE.MeshStandardMaterial({ color: 0x343b42, metalness: 0.42, roughness: 0.34 });
  const bars: Array<[string, THREE.Vector3, THREE.Vector3]> = [
    ["Window Frame Top", horizontalSize, new THREE.Vector3(0, halfH - frameWidth / 2, 0)],
    ["Window Frame Sill", horizontalSize, new THREE.Vector3(0, -halfH + frameWidth / 2, 0)],
    ["Window Frame Left", verticalSize, new THREE.Vector3(-halfW + frameWidth / 2, 0, 0)],
    ["Window Frame Right", verticalSize, new THREE.Vector3(halfW - frameWidth / 2, 0, 0)],
  ];
  for (const [name, size, offset] of bars) {
    const bar = new THREE.Mesh(new THREE.BoxGeometry(size.x, size.y, size.z), frameMaterial.clone());
    bar.name = name;
    bar.position.copy(center).add(offset);
    frame.add(bar);
  }
  assembly.add(frame);
  assembly.userData.window = { openingWidth, openingHeight, frameWidth, horizontalAxis };
  return assembly;
}
