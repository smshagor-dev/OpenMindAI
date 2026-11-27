import { useEffect, useRef, useState } from "react";
import * as THREE from "three";
import { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";
import { GLTFExporter } from "three/examples/jsm/exporters/GLTFExporter.js";
import { cloudSceneEditTarget, createBalconyResizePlan, createStaircaseMovePlan, createWindowAssembly, createWindowOpeningGeometry, createRoomAwareSofaMovePlan, findDoorwayClearanceBounds, type CloudSceneEditTarget, type SceneMatrixChange } from "../lib/studio3dSceneEdit";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import { invoke } from "@tauri-apps/api/core";
import { formatError } from "../lib/format";

// Inline GLB preview for locally generated 3D scenes. Mirrors the cloud
// studio-asset-viewport design (WebGL + OrbitControls + raycast pick), but
// loads a GLB straight from the local OpenCL worker's base64 payload instead
// of a hosted asset endpoint. The mesh is untrusted bytes: it is only parsed
// as glTF geometry, never executed.
function base64ToArrayBuffer(data: string): ArrayBuffer {
  const binary = window.atob(data);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes.buffer;
}

function transformValues(object: THREE.Object3D, field: "position" | "rotation" | "scale"): [number, number, number] {
  const value = object[field];
  return [value.x, value.y, value.z];
}

function findSceneNode(root: THREE.Object3D, ...labels: string[]): THREE.Object3D | null {
  const expected = new Set(labels.map((label) => label.toLocaleLowerCase().replace(/_/g, " ")));
  let match: THREE.Object3D | null = null;
  root.traverse((node) => {
    if (!match && expected.has(node.name.toLocaleLowerCase().replace(/_/g, " "))) match = node;
  });
  return match;
}

function createExportScene(object: THREE.Object3D, lights: Map<string, THREE.Light>, previewPositions?: Map<string, THREE.Vector3>, previewHiddenGroups: Set<string> = new Set()): THREE.Scene {
  object.updateMatrixWorld(true);
  const scene = new THREE.Scene();
  const clone = object.clone(true);
  previewPositions?.forEach((position, name) => clone.getObjectByName(name)?.position.copy(position));
  // Restore only groups hidden by viewport controls; preserve visibility
  // changes made by scene editing and undo/redo.
  for (const name of previewHiddenGroups) {
    const group = findSceneNode(clone, name);
    if (group) group.visible = true;
  }
  scene.add(clone);
  for (const light of lights.values()) {
    if (light instanceof THREE.AmbientLight) continue;
    scene.add(light.clone());
  }
  scene.updateMatrixWorld(true);
  return scene;
}

type StudioLightData = { id: string; type: "ambient" | "directional" | "point"; color: string; intensity: number; position: [number, number, number]; visible: boolean };

function parseSavedLights(value: unknown): StudioLightData[] | null {
  if (!Array.isArray(value)) return null;
  const ids = new Set<string>();
  return value.slice(0, 64).flatMap((entry): StudioLightData[] => {
    if (!entry || typeof entry !== "object") return [];
    const item = entry as Record<string, unknown>;
    const type = item.type;
    const color = item.color;
    const position = item.position;
    const intensity = item.intensity;
    if ((type !== "ambient" && type !== "directional" && type !== "point") || typeof color !== "string" || !/^#[0-9a-f]{6}$/i.test(color)
      || typeof intensity !== "number" || !Number.isFinite(intensity) || intensity < 0 || intensity > 100
      || !Array.isArray(position) || position.length !== 3 || position.some((coordinate) => typeof coordinate !== "number" || !Number.isFinite(coordinate) || Math.abs(coordinate) > 100_000)) return [];
    let id = typeof item.id === "string" && item.id.length > 0 && item.id.length <= 100 ? item.id : crypto.randomUUID();
    if (ids.has(id)) id = crypto.randomUUID();
    ids.add(id);
    return [{ id, type, color, intensity, position: position as [number, number, number], visible: item.visible !== false }];
  });
}

export function Studio3DViewport({ dataBase64, onSaveRevision, sceneJson, imageAssets = [], loadTextureAsset, onSceneModified }: {
  dataBase64: string;
  onSceneModified?: () => void;
  onSaveRevision?: (glbBase64: string, sceneJson: Record<string, unknown>) => Promise<void>;
  sceneJson?: Record<string, unknown>;
  imageAssets?: Array<{ id: string; name: string }>;
  loadTextureAsset?: (id: string) => Promise<{ mediaType: string; dataBase64: string }>;
}) {
  type HistoryEntry = { target: THREE.Object3D; before: THREE.Matrix4; after: THREE.Matrix4; beforeGeometry?: THREE.BufferGeometry; afterGeometry?: THREE.BufferGeometry; beforeMaterial?: THREE.Material | THREE.Material[]; afterMaterial?: THREE.Material | THREE.Material[]; added?: boolean; generated?: THREE.Object3D; openingEdit?: boolean; groupChanges?: SceneMatrixChange[] };
  const host = useRef<HTMLDivElement>(null);
  const viewerRef = useRef<HTMLDivElement>(null);
  const reset = useRef<() => void>(() => {});
  const zoom = useRef<(factor: number) => void>(() => {});
  const controlsRef = useRef<OrbitControls | null>(null);
  const setViewRef = useRef<(view: "exterior" | "interior" | "top" | "isometric") => void>(() => {});
  const cutPlaneRef = useRef<THREE.Plane | null>(null);
  const explodedPositionsRef = useRef(new Map<string, THREE.Vector3>());
  const walkthroughRef = useRef<{ camera: THREE.PerspectiveCamera; controls: OrbitControls; object: THREE.Object3D; renderer: THREE.WebGLRenderer; floorY: number } | null>(null);
  const renderRef = useRef<() => void>(() => {});
  const objectRef = useRef<THREE.Object3D | null>(null);
  const sceneRef = useRef<THREE.Scene | null>(null);
  const animationsRef = useRef<THREE.AnimationClip[]>([]);
  const mixerRef = useRef<THREE.AnimationMixer | null>(null);
  const clipTimeRef = useRef(0);
  const lightObjects = useRef(new Map<string, THREE.Light>());
  const [lights, setLights] = useState<StudioLightData[]>([]);
  const [clips, setClips] = useState<THREE.AnimationClip[]>([]);
  const [clipIndex, setClipIndex] = useState(0);
  const [clipTime, setClipTime] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [loop, setLoop] = useState(true);
  const [playbackSpeed, setPlaybackSpeed] = useState(1);
  const [selectedImageAsset, setSelectedImageAsset] = useState("");
  const [backgroundAssetId, setBackgroundAssetId] = useState(typeof sceneJson?.backgroundAssetId === "string" ? sceneJson.backgroundAssetId : "");
  const savedFloorVisibility = sceneJson && typeof sceneJson.floorVisibility === "object" ? sceneJson.floorVisibility as Record<string, unknown> : {};
  const [floorVisibility, setFloorVisibility] = useState({ ground: savedFloorVisibility.ground !== false, upper: savedFloorVisibility.upper !== false });
  const [cutaway, setCutaway] = useState(false);
  const [cutPosition, setCutPosition] = useState(0);
  const [exploded, setExploded] = useState(false);
  const [walkthrough, setWalkthrough] = useState(false);
  const [walkSpeed, setWalkSpeed] = useState(2.5);
  const userFrames = useRef(new Map<string, { times: number[]; positions: number[][]; quaternions: number[][]; scales: number[][] }>());
  const generatedTrackName = useRef("Studio animation 1");
  const rafRef = useRef<number | null>(null);
  const [error, setError] = useState("");
  const [ready, setReady] = useState(false);
  const [selected, setSelected] = useState("");
  const [autoRotate, setAutoRotate] = useState(false);
  const [autoPlay, setAutoPlay] = useState(false);
  const [isFullscreen, setIsFullscreen] = useState(false);
  const [zoomPercent, setZoomPercent] = useState(100);
  const [wireframe, setWireframe] = useState(false);
  const [sceneObjects, setSceneObjects] = useState<Array<{ object: THREE.Object3D; label: string; depth: number }>>([]);
  const [selection, setSelection] = useState<THREE.Object3D | null>(null);
  const selectionRef = useRef<THREE.Object3D | null>(null);
  const [transform, setTransform] = useState({ position: [0, 0, 0], rotation: [0, 0, 0], scale: [1, 1, 1] });
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [undoStack, setUndoStack] = useState<HistoryEntry[]>([]);
  const [redoStack, setRedoStack] = useState<HistoryEntry[]>([]);
  const undoStackRef = useRef<HistoryEntry[]>([]);
  const redoStackRef = useRef<HistoryEntry[]>([]);
  const autoPlayRef = useRef(autoPlay);
  const [aiEditPrompt, setAiEditPrompt] = useState("");
  const [aiEditBusy, setAiEditBusy] = useState(false);
  const [aiEditStatus, setAiEditStatus] = useState("");
  const [aiEditRoute, setAiEditRoute] = useState<"local" | "cloud">("local");
  const windowOpeningTargets = useRef(new WeakSet<THREE.Mesh>());
  const [materialRevision, setMaterialRevision] = useState(0);
  const materialSnapshots = useRef(new WeakMap<THREE.Mesh, THREE.Material | THREE.Material[]>());
  const editedMaterials = useRef(new WeakSet<THREE.Mesh>());
  const sceneMetadataRef = useRef(sceneJson);
  const aiEditRecords = useRef<Array<Record<string, unknown>>>(Array.isArray(sceneJson?.aiEdits) ? sceneJson.aiEdits as Array<Record<string, unknown>> : []);
  const pushUndo = (entry: HistoryEntry) => {
    onSceneModified?.();
    setUndoStack((previous) => [...previous, entry].slice(-100));
  };
  const clearRedo = () => setRedoStack([]);
  useEffect(() => { undoStackRef.current = undoStack; }, [undoStack]);
  useEffect(() => { redoStackRef.current = redoStack; }, [redoStack]);
  useEffect(() => { autoPlayRef.current = autoPlay; }, [autoPlay]);
  useEffect(() => { sceneMetadataRef.current = sceneJson; }, [sceneJson]);
  useEffect(() => { aiEditRecords.current = Array.isArray(sceneJson?.aiEdits) ? sceneJson.aiEdits as Array<Record<string, unknown>> : []; }, [sceneJson]);

  const selectedMesh = selection instanceof THREE.Mesh ? selection : null;
  const selectedMaterial = selectedMesh
    ? (Array.isArray(selectedMesh.material) ? selectedMesh.material[0] : selectedMesh.material) as THREE.MeshStandardMaterial
    : null;

  const updateMaterial = (update: (material: THREE.MeshStandardMaterial) => void) => {
    onSceneModified?.();
    const target = selectionRef.current;
    if (!(target instanceof THREE.Mesh)) return;
    if (!editedMaterials.current.has(target)) {
      const wasMultiMaterial = Array.isArray(target.material);
      const source: THREE.Material[] = wasMultiMaterial ? target.material : [target.material];
      const clones: THREE.Material[] = source.map((material: THREE.Material) => material.clone());
      target.material = wasMultiMaterial ? clones : clones[0];
      editedMaterials.current.add(target);
    }
    const current = Array.isArray(target.material) ? target.material : [target.material];
    for (const value of current) {
      if (!(value instanceof THREE.MeshStandardMaterial)) continue;
      update(value);
      value.needsUpdate = true;
    }
    setMaterialRevision((value) => value + 1);
    renderRef.current();
  };

  const applyTexture = async (assetId: string) => {
    if (!(selectionRef.current instanceof THREE.Mesh) || !loadTextureAsset) return;
    try {
      const asset = await loadTextureAsset(assetId);
      const texture = await new THREE.TextureLoader().loadAsync(`data:${asset.mediaType};base64,${asset.dataBase64}`);
      texture.colorSpace = THREE.SRGBColorSpace;
      texture.flipY = false;
      updateMaterial((material) => { material.map = texture; });
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Could not load this image as a texture.");
    }
  };

  const duplicateMaterial = () => {
    onSceneModified?.();
    const target = selectionRef.current;
    if (!(target instanceof THREE.Mesh)) return;
    const wasMultiMaterial = Array.isArray(target.material);
    const materials: THREE.Material[] = wasMultiMaterial ? target.material : [target.material];
    const clones: THREE.Material[] = materials.map((material: THREE.Material) => material.clone());
    target.material = wasMultiMaterial ? clones : clones[0];
    editedMaterials.current.add(target);
    setMaterialRevision((value) => value + 1);
    renderRef.current();
  };

  const duplicateObject = () => {
    onSceneModified?.();
    const target = selectionRef.current;
    if (!target || target === objectRef.current) return;
    const clone = target.clone(true);
    const baseName = target.name || "Object";
    let name = baseName === "front_camera" ? "front_camera_2" : `${baseName}_copy`;
    let suffix = 2;
    while (sceneObjects.some(({ object }) => object !== target && object.name === name)) {
      name = `${baseName}_${suffix++}`;
    }
    clone.name = name;
    clone.position.x += 0.24;
    clone.updateMatrix();
    target.parent?.add(clone);
    const depth = sceneObjects.find(({ object }) => object === target)?.depth ?? 0;
    setSceneObjects((items) => [...items, { object: clone, label: name, depth }]);
    selectionRef.current = clone;
    setSelection(clone);
    setSelected(name);
    setTransform({ position: clone.position.toArray(), rotation: transformValues(clone, "rotation"), scale: clone.scale.toArray() });
    setDrafts(Object.fromEntries((["position", "rotation", "scale"] as const).flatMap((field) =>
      (["X", "Y", "Z"] as const).map((axis, index) => [
        `${field}-${axis}`,
        String(Number((field === "rotation" ? THREE.MathUtils.radToDeg(transformValues(clone, field)[index]) : transformValues(clone, field)[index]).toFixed(3))),
      ]),
    )));
    renderRef.current();
  };

  const resetMaterial = () => {
    const target = selectionRef.current;
    if (!(target instanceof THREE.Mesh)) return;
    const original = materialSnapshots.current.get(target);
    if (original) target.material = Array.isArray(original) ? original.map((material) => material.clone()) : original.clone();
    editedMaterials.current.delete(target);
    setMaterialRevision((value) => value + 1);
    renderRef.current();
  };

  useEffect(() => {
    let disposed = false;
    const cleaners: (() => void)[] = [];
    const cleanup = () => {
      if (rafRef.current !== null) cancelAnimationFrame(rafRef.current);
      rafRef.current = null;
      while (cleaners.length) cleaners.pop()?.();
      controlsRef.current = null;
      objectRef.current = null;
      walkthroughRef.current = null;
      cutPlaneRef.current = null;
    };
    const disposeObject = (object: THREE.Object3D) =>
      object.traverse((o) => {
        if (o instanceof THREE.Mesh) {
          o.geometry.dispose();
          for (const material of Array.isArray(o.material) ? o.material : [o.material]) {
            for (const value of Object.values(material))
              if (value instanceof THREE.Texture) value.dispose();
            material.dispose();
          }
        }
      });

    (async () => {
      try {
        const buffer = base64ToArrayBuffer(dataBase64);
        const gltf = await new GLTFLoader().parseAsync(buffer, "");
        const object = gltf.scene;
        const savedLights = parseSavedLights(sceneMetadataRef.current?.lights);
        // Saved project lights are stored both in scene metadata and in the
        // GLB. Rebuild them from metadata so reopening and saving does not
        // duplicate the embedded punctual lights.
        if (savedLights) {
          const embeddedLights: THREE.Light[] = [];
          object.traverse((child) => { if (child instanceof THREE.Light) embeddedLights.push(child); });
          for (const light of embeddedLights) light.parent?.remove(light);
        }
        animationsRef.current = gltf.animations;
        setClips(gltf.animations);
        if (autoPlayRef.current && gltf.animations.length > 0) setPlaying(true);
        setClipIndex(0);
        setClipTime(0);
        if (disposed) {
          disposeObject(object);
          return;
        }
        cleaners.push(() => disposeObject(object));
        object.traverse((child) => {
          if (child instanceof THREE.Mesh) {
            const materials = Array.isArray(child.material) ? child.material : [child.material];
            materialSnapshots.current.set(child, Array.isArray(child.material) ? materials.map((material) => material.clone()) : materials[0].clone());
          }
        });
        objectRef.current = object;
        const rows: Array<{ object: THREE.Object3D; label: string; depth: number }> = [];
        object.traverse((child) => {
          if (child === object) return;
          let depth = 0;
          for (let parent = child.parent; parent && parent !== object; parent = parent.parent) depth++;
          rows.push({ object: child, label: child.name || (child instanceof THREE.Mesh ? "Mesh" : child.type), depth });
        });
        // Architecture can exceed 500 editable parts once furnishing is on.
        // Keep the list bounded for imported files while retaining every
        // generated building component supported by the native validator.
        setSceneObjects(rows.slice(0, 2000));
        const container = host.current!;
        const read = (token: string, fallback: string) =>
          getComputedStyle(container).getPropertyValue(token).trim() || fallback;
        const surface = new THREE.Color(read("--bg", "#303030"));

        const renderer = new THREE.WebGLRenderer({ antialias: true });
        renderer.localClippingEnabled = true;
        renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
        cleaners.push(() => {
          renderer.dispose();
          renderer.domElement.remove();
        });

        const scene = new THREE.Scene();
        sceneRef.current = scene;
        scene.background = surface;
        cleaners.push(() => { if (scene.background instanceof THREE.Texture) scene.background.dispose(); sceneRef.current = null; });
        const initialLights = savedLights ?? [
          { id: crypto.randomUUID(), type: "ambient" as const, color: "#ffffff", intensity: 1.4, position: [0, 0, 0] as [number, number, number], visible: true },
          { id: crypto.randomUUID(), type: "directional" as const, color: "#ffffff", intensity: 2.6, position: [5, 8, 4] as [number, number, number], visible: true },
          { id: crypto.randomUUID(), type: "directional" as const, color: "#ffffff", intensity: 0.8, position: [-6, 3, -4] as [number, number, number], visible: true },
        ];
        setLights(initialLights);
        scene.add(object);
        cutPlaneRef.current = new THREE.Plane(new THREE.Vector3(-1, 0, 0), 0);

        const bounds = new THREE.Box3().setFromObject(object);
        const center = bounds.getCenter(new THREE.Vector3());
        const span = bounds.getSize(new THREE.Vector3());
        const size = span.length();
        const defaultCameraDistance = new THREE.Vector3(size, size * 0.7, size).length() * 0.9;

        // Subtle ground grid for spatial context, sized to the model and sunk to
        // its base so it reads as a floor rather than a slice through the mesh.
        const gridColor = new THREE.Color(read("--muted", "#8a8a8a"));
        const grid = new THREE.GridHelper(Math.max(size * 1.6, 1), 24, gridColor, gridColor);
        const gridMaterial = grid.material as THREE.Material | THREE.Material[];
        for (const material of Array.isArray(gridMaterial) ? gridMaterial : [gridMaterial]) {
          material.transparent = true;
          material.opacity = 0.18;
        }
        grid.position.set(center.x, bounds.min.y, center.z);
        scene.add(grid);
        cleaners.push(() => {
          grid.geometry.dispose();
          for (const material of Array.isArray(gridMaterial) ? gridMaterial : [gridMaterial])
            material.dispose();
        });

        const camera = new THREE.PerspectiveCamera(
          45,
          1,
          Math.max(0.001, size / 10000),
          Math.max(100, size * 20),
        );
        const controls = new OrbitControls(camera, renderer.domElement);
        controls.enableDamping = true;
        controls.dampingFactor = 0.08;
        controlsRef.current = controls;
        const groundSlab = findSceneNode(object, "Ground floor slab");
        const groundTop = groundSlab ? new THREE.Box3().setFromObject(groundSlab).max.y : bounds.min.y;
        walkthroughRef.current = { camera, controls, object, renderer, floorY: groundTop + 1.65 };
        cleaners.push(() => controls.dispose());
        controls.target.copy(center);
        setViewRef.current = (view) => {
          if (walkthroughRef.current) walkthroughRef.current.controls.enabled = true;
          setWalkthrough(false);
          const distance = Math.max(size * 0.85, 4);
          const target = center.clone();
          if (view === "top") camera.position.set(center.x, bounds.max.y + distance, center.z + 0.01);
          else if (view === "interior") { target.set(center.x, bounds.min.y + Math.min(span.y * 0.38, 2.8), center.z); camera.position.set(center.x, target.y + 0.15, center.z + distance * 0.2); }
          else if (view === "exterior") camera.position.set(center.x + distance * 1.5, center.y + distance * 0.55, center.z + distance * 1.5);
          else camera.position.set(center.x + distance, center.y + distance * 0.8, center.z + distance);
          controls.target.copy(target);
          controls.update();
          setZoomPercent(Math.round(THREE.MathUtils.clamp(defaultCameraDistance / Math.max(camera.position.distanceTo(target), 0.001) * 100, 20, 500)));
          render();
        };

        function render() {
          renderer.render(scene, camera);
        }
        renderRef.current = render;
        reset.current = () => {
          camera.position
            .copy(center)
            .add(new THREE.Vector3(size, size * 0.7, size).multiplyScalar(0.9));
          controls.target.copy(center);
          controls.update();
          setZoomPercent(100);
          render();
        };
        zoom.current = (factor) => {
          const offset = camera.position.clone().sub(controls.target);
          const distance = THREE.MathUtils.clamp(offset.length() * factor, defaultCameraDistance / 5, defaultCameraDistance * 5);
          camera.position.copy(controls.target).add(offset.setLength(distance));
          controls.update();
          setZoomPercent(Math.round(defaultCameraDistance / Math.max(distance, 0.001) * 100));
          render();
        };

        container.append(renderer.domElement);
        function resize() {
          const w = container.clientWidth;
          const h = container.clientHeight;
          renderer.setSize(w, h);
          camera.aspect = w / Math.max(1, h);
          camera.updateProjectionMatrix();
          render();
        }
        const observer = new ResizeObserver(resize);
        cleaners.push(() => observer.disconnect());
        observer.observe(container);
        controls.addEventListener("change", render);
        reset.current();
        resize();

        const pick = (raw: Event) => {
          const event = raw as PointerEvent;
          const rect = renderer.domElement.getBoundingClientRect();
          const ray = new THREE.Raycaster();
          ray.setFromCamera(
            new THREE.Vector2(
              ((event.clientX - rect.left) / rect.width) * 2 - 1,
              (-(event.clientY - rect.top) / rect.height) * 2 + 1,
            ),
            camera,
          );
          const hit = ray.intersectObject(object, true)[0];
          const picked = hit?.object ?? null;
          selectionRef.current = picked;
          setSelection(picked);
          setSelected(picked?.name || (picked ? "Mesh part" : ""));
          if (picked) setTransform({
            position: picked.position.toArray(),
            rotation: transformValues(picked, "rotation"),
            scale: picked.scale.toArray(),
          });
          if (picked) setDrafts(Object.fromEntries((["position", "rotation", "scale"] as const).flatMap((field) =>
            (["X", "Y", "Z"] as const).map((axis, index) => [
              `${field}-${axis}`,
              String(Number((field === "rotation" ? THREE.MathUtils.radToDeg(transformValues(picked, field)[index]) : transformValues(picked, field)[index]).toFixed(3))),
            ]),
          )));
        };
        renderer.domElement.addEventListener("pointerup", pick);
        cleaners.push(() => renderer.domElement.removeEventListener("pointerup", pick));
        setReady(true);
      } catch (e) {
        cleanup();
        if (!disposed) setError(e instanceof Error ? e.message : "Could not preview model.");
      }
    })();

    return () => {
      disposed = true;
      cleanup();
    };
  }, [dataBase64]);

  useEffect(() => {
    const plane = cutPlaneRef.current;
    const object = objectRef.current;
    if (!plane || !object) return;
    plane.constant = cutPosition;
    const roof = findSceneNode(object, "Roof");
    if (roof) roof.visible = !cutaway;
    object.traverse((child) => {
      if (!(child instanceof THREE.Mesh)) return;
      for (const material of Array.isArray(child.material) ? child.material : [child.material])
        material.clippingPlanes = cutaway ? [plane] : [];
    });
    renderRef.current();
  }, [cutPosition, cutaway, ready]);

  useEffect(() => {
    const object = objectRef.current;
    if (!object) return;
    const groups = [findSceneNode(object, "Roof"), findSceneNode(object, "Upper Floor"), findSceneNode(object, "Staircase")].filter((value): value is THREE.Object3D => Boolean(value));
    const positions = groups.map((group) => group.position.clone());
    explodedPositionsRef.current = new Map(groups.map((group, index) => [group.name, positions[index].clone()]));
    if (!exploded) return;
    let frame = 0;
    let start = 0;
    const animate = (now: number) => {
      if (!start) start = now;
      const t = Math.min(1, (now - start) / 420);
      const ease = t * (2 - t);
      groups.forEach((group, index) => {
        const name = group.name.toLocaleLowerCase().replace(/_/g, " ");
        const amount = name === "roof" ? 2.4 : name === "upper floor" ? 1.15 : 0.35;
        group.position.set(positions[index].x, positions[index].y + amount * ease, positions[index].z);
      });
      renderRef.current();
      if (t < 1) frame = requestAnimationFrame(animate);
    };
    frame = requestAnimationFrame(animate);
    return () => {
      cancelAnimationFrame(frame);
      groups.forEach((group, index) => group.position.copy(positions[index]));
      renderRef.current();
    };
  }, [exploded, ready]);

  useEffect(() => {
    const state = walkthroughRef.current;
    if (!state) return;
    const { camera, controls, object, renderer, floorY } = state;
    if (!walkthrough) {
      controls.enabled = true;
      if (document.pointerLockElement === renderer.domElement) document.exitPointerLock();
      renderRef.current();
      return;
    }
    controls.enabled = false;
    camera.position.set(0, floorY, 3.8);
    camera.rotation.set(0, 0, 0);
    const keys = new Set<string>();
    let previous = globalThis.performance.now();
    let frame = 0;
    const down = (event: KeyboardEvent) => {
      if (event.code === "Escape") { setWalkthrough(false); return; }
      if (["KeyW", "KeyA", "KeyS", "KeyD"].includes(event.code)) { event.preventDefault(); keys.add(event.code); }
    };
    const up = (event: KeyboardEvent) => keys.delete(event.code);
    const look = (event: MouseEvent) => {
      if (document.pointerLockElement !== renderer.domElement) return;
      camera.rotation.order = "YXZ";
      camera.rotation.y -= event.movementX * 0.002;
      camera.rotation.x = THREE.MathUtils.clamp(camera.rotation.x - event.movementY * 0.002, -Math.PI * 0.47, Math.PI * 0.47);
    };
    const lock = () => { void renderer.domElement.requestPointerLock(); };
    const raycaster = new THREE.Raycaster();
    const worldUp = new THREE.Vector3(0, 1, 0);
    const floorRaycaster = new THREE.Raycaster();
    const downVector = new THREE.Vector3(0, -1, 0);
    window.addEventListener("keydown", down); window.addEventListener("keyup", up); window.addEventListener("mousemove", look);
    renderer.domElement.addEventListener("click", lock);
    const tick = (now: number) => {
      const dt = Math.min(0.05, (now - previous) / 1000); previous = now;
      const movement = new THREE.Vector3(Number(keys.has("KeyD")) - Number(keys.has("KeyA")), 0, Number(keys.has("KeyS")) - Number(keys.has("KeyW")));
      if (movement.lengthSq()) {
        movement.normalize().applyAxisAngle(worldUp, camera.rotation.y).multiplyScalar(walkSpeed * dt);
        const candidate = camera.position.clone().add(movement);
        candidate.y = floorY;
        raycaster.set(camera.position, movement.clone().normalize()); raycaster.far = movement.length() + 0.32;
        const hit = raycaster.intersectObject(object, true).find((item) => item.object instanceof THREE.Mesh && !/glass|pane|door leaf/i.test(item.object.name));
        if (!hit || hit.distance > movement.length() + 0.3) {
          floorRaycaster.set(new THREE.Vector3(candidate.x, camera.position.y + 0.4, candidate.z), downVector);
          floorRaycaster.far = 4.5;
          const surface = floorRaycaster.intersectObject(object, true).find((item) => item.object instanceof THREE.Mesh && /(floor|slab|stair tread|stair landing)/i.test(item.object.name.replace(/_/g, " ")));
          if (surface) {
            const nextEyeHeight = surface.point.y + 1.65;
            // Treads rise in short steps. Follow those surfaces while keeping
            // the eye height fixed over each tread; downward drops are clamped
            // by the ray to a real floor surface rather than passing through it.
            if (nextEyeHeight <= camera.position.y || Math.abs(nextEyeHeight - camera.position.y) <= 0.32) candidate.y = nextEyeHeight;
          }
          camera.position.copy(candidate);
        }
      }
      renderRef.current(); frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => {
      cancelAnimationFrame(frame);
      window.removeEventListener("keydown", down); window.removeEventListener("keyup", up); window.removeEventListener("mousemove", look);
      renderer.domElement.removeEventListener("click", lock);
      if (document.pointerLockElement === renderer.domElement) document.exitPointerLock();
    };
  }, [walkthrough, walkSpeed, ready]);

  useEffect(() => {
    const object = objectRef.current;
    if (!object) return;
    const ground = findSceneNode(object, "Ground Floor");
    const upper = findSceneNode(object, "Upper Floor");
    if (ground) ground.visible = floorVisibility.ground;
    if (upper) upper.visible = floorVisibility.upper;
    renderRef.current();
  }, [floorVisibility, ready]);

  useEffect(() => {
    const scene = sceneRef.current;
    if (!scene) return;
    for (const [id, light] of lightObjects.current) { scene.remove(light); light.dispose?.(); lightObjects.current.delete(id); }
    for (const item of lights) {
      const light = item.type === "ambient" ? new THREE.AmbientLight(item.color, item.intensity)
        : item.type === "point" ? new THREE.PointLight(item.color, item.intensity)
          : new THREE.DirectionalLight(item.color, item.intensity);
      light.name = `studio-light-${item.id}`;
      light.position.set(...item.position);
      light.visible = item.visible;
      scene.add(light);
      lightObjects.current.set(item.id, light);
    }
    renderRef.current();
  }, [lights, ready]);

  useEffect(() => {
    setBackgroundAssetId(typeof sceneMetadataRef.current?.backgroundAssetId === "string" ? sceneMetadataRef.current.backgroundAssetId : "");
  }, [dataBase64]);

  useEffect(() => {
    if (!backgroundAssetId || !loadTextureAsset || !sceneRef.current) return;
    let cancelled = false;
    void (async () => {
      try {
        const asset = await loadTextureAsset(backgroundAssetId);
        const texture = await new THREE.TextureLoader().loadAsync(`data:${asset.mediaType};base64,${asset.dataBase64}`);
        if (cancelled) { texture.dispose(); return; }
        texture.colorSpace = THREE.SRGBColorSpace;
        const scene = sceneRef.current;
        if (scene) { if (scene.background instanceof THREE.Texture) scene.background.dispose(); scene.background = texture; renderRef.current(); }
      } catch (caught) { setError(caught instanceof Error ? caught.message : "Could not load the project background image."); }
    })();
    return () => { cancelled = true; };
  }, [backgroundAssetId, loadTextureAsset, ready]);

  const placeImagePlane = async () => {
    onSceneModified?.();
    if (!selectedImageAsset || !objectRef.current || !loadTextureAsset) return;
    try {
      const asset = await loadTextureAsset(selectedImageAsset);
      const texture = await new THREE.TextureLoader().loadAsync(`data:${asset.mediaType};base64,${asset.dataBase64}`);
      texture.colorSpace = THREE.SRGBColorSpace;
      const image = texture.image as { width?: number; height?: number };
      const aspect = image.width && image.height ? image.width / image.height : 1;
      const plane = new THREE.Mesh(new THREE.PlaneGeometry(aspect * 2, 2), new THREE.MeshStandardMaterial({ map: texture, side: THREE.DoubleSide, roughness: 1, metalness: 0 }));
      plane.name = "2D image plane";
      plane.position.y = 1;
      objectRef.current.add(plane);
      setSceneObjects((rows) => [...rows, { object: plane, label: plane.name, depth: 0 }]);
      renderRef.current();
    } catch (caught) { setError(caught instanceof Error ? caught.message : "Could not place this project image."); }
  };

  const setBackgroundImage = () => {
    onSceneModified?.();
    if (!selectedImageAsset) return;
    setBackgroundAssetId(selectedImageAsset);
  };

  useEffect(() => {
    const object = objectRef.current;
    if (!object || !clips.length) return;
    mixerRef.current?.stopAllAction();
    const mixer = new THREE.AnimationMixer(object);
    mixerRef.current = mixer;
    const clip = clips[Math.min(clipIndex, clips.length - 1)];
    const action = mixer.clipAction(clip);
    action.setLoop(loop ? THREE.LoopRepeat : THREE.LoopOnce, loop ? Infinity : 1);
    action.play();
    mixer.setTime(THREE.MathUtils.clamp(clipTimeRef.current, 0, clip.duration));
    renderRef.current();
    return () => { mixer.stopAllAction(); mixer.uncacheRoot(object); };
  }, [clips, clipIndex, loop, ready]);

  useEffect(() => {
    const clip = clips[clipIndex];
    if (!clip || !mixerRef.current) return;
    const action = mixerRef.current.clipAction(clip);
    action.paused = !playing;
    if (clipTime >= clip.duration && !loop) action.paused = true;
  }, [playing, clips, clipIndex, clipTime, loop]);

  useEffect(() => {
    const viewer = viewerRef.current;
    if (!viewer) return;
    const syncFullscreen = () => setIsFullscreen(document.fullscreenElement === viewer);
    document.addEventListener("fullscreenchange", syncFullscreen);
    return () => document.removeEventListener("fullscreenchange", syncFullscreen);
  }, []);

  const toggleFullscreen = async () => {
    const viewer = viewerRef.current;
    if (!viewer) return;
    try {
      if (document.fullscreenElement === viewer) await document.exitFullscreen();
      else await viewer.requestFullscreen();
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Could not change 3D viewer fullscreen mode.");
    }
  };

  useEffect(() => {
    if (!playing || !mixerRef.current) return;
    let frame = 0;
    let previous = globalThis.performance.now();
    const tick = (now: number) => {
      const delta = Math.min(0.1, (now - previous) / 1000) * playbackSpeed;
      previous = now;
      mixerRef.current?.update(delta);
      const clip = clips[clipIndex];
      if (clip) {
        clipTimeRef.current = loop ? (clipTimeRef.current + delta) % Math.max(clip.duration, 0.001) : Math.min(clip.duration, clipTimeRef.current + delta);
        setClipTime(clipTimeRef.current);
        if (!loop && clipTimeRef.current >= clip.duration) setPlaying(false);
      }
      renderRef.current();
      frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, [playing, playbackSpeed, clips, clipIndex, loop]);

  // Auto-rotate runs a render loop only while enabled; otherwise the viewport
  // renders on demand (on orbit) to stay idle-friendly.
  useEffect(() => {
    const controls = controlsRef.current;
    if (!controls) return;
    controls.autoRotate = autoRotate;
    controls.autoRotateSpeed = 1.6;
    if (!autoRotate) {
      if (rafRef.current !== null) cancelAnimationFrame(rafRef.current);
      rafRef.current = null;
      return;
    }
    const tick = () => {
      controls.update();
      renderRef.current();
      rafRef.current = requestAnimationFrame(tick);
    };
    rafRef.current = requestAnimationFrame(tick);
    return () => {
      if (rafRef.current !== null) cancelAnimationFrame(rafRef.current);
      rafRef.current = null;
    };
  }, [autoRotate, ready]);

  // Wireframe reveals the underlying triangle mesh the local GPU produced.
  useEffect(() => {
    const object = objectRef.current;
    if (!object) return;
    object.traverse((o) => {
      if (o instanceof THREE.Mesh)
        for (const material of Array.isArray(o.material) ? o.material : [o.material])
          if ("wireframe" in material)
            (material as THREE.MeshStandardMaterial).wireframe = wireframe;
    });
    renderRef.current();
  }, [wireframe, ready]);

  const commitTransform = (field: "position" | "rotation" | "scale", axis: 0 | 1 | 2, raw: string) => {
    const target = selection;
    const value = Number(raw);
    if (!target) return;
    if (!Number.isFinite(value) || Math.abs(value) > 1_000_000) {
      setDrafts((old) => ({ ...old, [`${field}-${["X", "Y", "Z"][axis]}`]: String(transform[field][axis]) }));
      return;
    }
    target.updateMatrix();
    const before = target.matrix.clone();
    if (field === "position") target.position.setComponent(axis, value);
    else if (field === "rotation") {
      const euler = [target.rotation.x, target.rotation.y, target.rotation.z];
      euler[axis] = THREE.MathUtils.degToRad(value);
      target.rotation.set(euler[0], euler[1], euler[2]);
    } else target.scale.setComponent(axis, value);
    target.updateMatrix();
    const change = { target, before, after: target.matrix.clone() };
    pushUndo(change);
    clearRedo();
    setTransform({
      position: target.position.toArray(),
      rotation: transformValues(target, "rotation"),
      scale: target.scale.toArray(),
    });
    setDrafts(Object.fromEntries((["position", "rotation", "scale"] as const).flatMap((key) =>
      (["X", "Y", "Z"] as const).map((axisName, index) => [
        `${key}-${axisName}`,
        String(Number((key === "rotation" ? THREE.MathUtils.radToDeg(transformValues(target, key)[index]) : transformValues(target, key)[index]).toFixed(3))),
      ]),
    )));
  };

  const applyAiSceneEdit = async (operation: "edit" | "add") => {
    const target = selectionRef.current;
    const request = aiEditPrompt.trim();
    if (!target || request.length < 5) return;
    let cloudResponseDiagnostics: string | null = null;
    let cloudMutationCommitted = false;
    setAiEditBusy(true);
    setError("");
    try {
      target.updateMatrix();
      const root = objectRef.current;
      const bounds = target instanceof THREE.Mesh ? target.geometry.boundingBox ?? (target.geometry.computeBoundingBox(), target.geometry.boundingBox) : null;
      const groupBounds = bounds ? null : new THREE.Box3().setFromObject(target);
      const dimensions = (bounds ? [bounds.max.x - bounds.min.x, bounds.max.y - bounds.min.y, bounds.max.z - bounds.min.z].map((value, axis) => value * [target.scale.x, target.scale.y, target.scale.z][axis]) : groupBounds ? [groupBounds.max.x - groupBounds.min.x, groupBounds.max.y - groupBounds.min.y, groupBounds.max.z - groupBounds.min.z] : [1, 1, 1]) as [number, number, number];
      const material = target instanceof THREE.Mesh ? (Array.isArray(target.material) ? target.material[0] : target.material) : null;
      const pbr = material instanceof THREE.MeshStandardMaterial ? material : null;
      const toBounds = (object: THREE.Object3D) => {
        object.updateWorldMatrix(true, false);
        const box = new THREE.Box3().setFromObject(object);
        const rounded = (value: number) => Number(value.toFixed(3));
        return { min: box.min.toArray().map(rounded), max: box.max.toArray().map(rounded) };
      };
      const worldBounds = toBounds(target);
      let spatialContext: Record<string, unknown> | undefined;
      if (root && /sofa/i.test(target.name)) {
        const floor = findSceneNode(root, "Living_Room_floor");
        if (floor) {
          const floorBox = new THREE.Box3().setFromObject(floor);
          // Search bounded offsets relative to the sofa's current local position. Absolute
          // coordinates fail when an imported scene or its room parent has a transform.
          const offsets = [0.75, 1, 1.25, 1.5, 1.75, 2, 2.25, 2.5];
          const candidatePositions: Array<[number, number, number]> = offsets.flatMap((offset) => [
            [target.position.x - offset, target.position.y, target.position.z],
            [target.position.x + offset, target.position.y, target.position.z],
            [target.position.x, target.position.y, target.position.z - offset],
            [target.position.x, target.position.y, target.position.z + offset],
          ]);
          const safeCandidatePosition = candidatePositions.find((position) => {
            try { createRoomAwareSofaMovePlan(root, target, position); return true; } catch { return false; }
          }) ?? null;
          spatialContext = {
            kind: "room_aware_sofa_move",
            worldBounds,
            room: { name: "Living_Room", bounds: { min: floorBox.min.toArray(), max: floorBox.max.toArray() } },
            doorwayClearanceBounds: findDoorwayClearanceBounds(root, floorBox).map((box) => ({ min: box.min.toArray().map((value) => Number(value.toFixed(3))), max: box.max.toArray().map((value) => Number(value.toFixed(3))) })),
            nearbyFurniture: (target.parent?.children ?? []).filter((object) => object !== target && object instanceof THREE.Mesh).map((object) => ({ name: object.name, bounds: toBounds(object) })),
            safeCandidatePosition,
            maxTranslationMeters: 3,
          };
        }
      } else if (root && target.name === "Balcony_slab") {
        const members: Array<{ name: string; centerX: number }> = [];
        root.traverse((object) => {
          if (object instanceof THREE.Mesh && object.name.startsWith("Balcony_")) {
            const box = new THREE.Box3().setFromObject(object);
            members.push({ name: object.name, centerX: Number(box.getCenter(new THREE.Vector3()).x.toFixed(3)) });
          }
        });
        spatialContext = { kind: "balcony_assembly_resize", worldBounds, dependentNamesAndCenterX: members, minWidthIncreaseRatio: 1.02, maxWidthIncreaseRatio: 1.35 };
      } else if (root && target.name === "Staircase") {
        const landing = findSceneNode(root, "Stair_Landing_floor");
        const ground = findSceneNode(root, "Ground_floor_slab");
        const entrance = findSceneNode(root, "ground_front_wooden_door_leaf");
        const candidatePositions: Array<[number, number, number]> = [0.05, 0.1, 0.15].map((offset) => [target.position.x, target.position.y, target.position.z + offset]);
        const safeCandidatePosition = candidatePositions.find((position) => {
          try { createStaircaseMovePlan(root, target, position); return true; } catch { return false; }
        }) ?? null;
        spatialContext = {
          kind: "connected_staircase_move",
          worldBounds,
          upperLandingBounds: landing ? toBounds(landing) : null,
          groundFloorBounds: ground ? toBounds(ground) : null,
          mainEntranceBounds: entrance ? toBounds(entrance) : null,
          safeCandidatePosition,
          maximumTranslationMeters: 0.2,
        };
      }
      const command = aiEditRoute === "cloud" ? "studio3d_cloud_edit_object" : "studio3d_ai_edit_object";
      const result = await invoke<{ modelId: string; requestId?: string; clientRequestId?: string; providerRequestId?: string; httpStatus?: number; finishReason?: string; editApplied?: boolean; usage?: { prompt_tokens?: number; completion_tokens?: number; total_tokens?: number }; object: { name: string; primitive: string; dimensions: [number, number, number]; position: [number, number, number]; rotation: [number, number, number]; color: [number, number, number]; metallic: number; roughness: number } }>(command, {
        prompt: request,
        operation,
        target: (() => {
          const selectedTarget: CloudSceneEditTarget & { worldBounds: Record<string, unknown>; spatialContext: Record<string, unknown> | undefined } = {
            name: target.name,
            parent: target.parent?.name ?? null,
            kind: target instanceof THREE.Mesh ? "mesh" : "container",
            dimensions,
            position: [target.position.x, target.position.y, target.position.z] as [number, number, number],
            rotation: [target.rotation.x, target.rotation.y, target.rotation.z],
            color: pbr ? [pbr.color.r, pbr.color.g, pbr.color.b] : [0.7, 0.7, 0.7],
            metallic: pbr?.metalness ?? 0,
            roughness: pbr?.roughness ?? 0.8,
            worldBounds,
            spatialContext,
          };
          return aiEditRoute === "cloud" ? cloudSceneEditTarget(selectedTarget) : selectedTarget;
        })(),
      });
      const patch = result.object;
      if (aiEditRoute === "cloud") {
        cloudResponseDiagnostics = `Cloud response: model=${result.modelId}; client_request_id=${result.clientRequestId ?? "unavailable"}; provider_request_id=${result.providerRequestId ?? "unavailable"}; gateway_status=${result.httpStatus ?? "unknown"}; finish_reason=${result.finishReason ?? "unknown"}; usage=${result.usage ? JSON.stringify(result.usage) : "unknown"};`;
      }
      let transactionAffectedIds = [target.uuid];
      let transactionAffectedNames = [target.name];
      if (operation === "edit") {
        if (patch.name !== target.name) throw new Error("The AI patch did not preserve the selected object identity.");
        const openingRequest = /\b(?:window|opening)\b/i.test(request);
        const balconyRequest = target.name === "Balcony_slab" && /\b(?:balcony|railing|width|widen)\b/i.test(request);
        const sofaRequest = /sofa/i.test(target.name) && /\bmove\b/i.test(request);
        const staircaseRequest = target.name === "Staircase" && /\b(?:staircase|stairs)\b/i.test(request);
        let groupChanges: SceneMatrixChange[] | undefined;
        if (balconyRequest) {
          if (Math.abs(patch.dimensions[1] - dimensions[1]) > 0.03 || Math.abs(patch.dimensions[2] - dimensions[2]) > 0.03)
            throw new Error("The validated balcony patch changed slab height or depth; no assembly members were changed.");
          if (!root) throw new Error("The scene root is unavailable for balcony assembly validation.");
          // All 21 transforms are validated before any member is changed.
          groupChanges = createBalconyResizePlan(root, target, patch.dimensions[0]);
        } else if (sofaRequest) {
          if (dimensions.some((value, axis) => Math.abs(patch.dimensions[axis] - value) > 0.04))
            throw new Error("The sofa patch changed its size; the room-aware move was rejected before mutation.");
          if (!root) throw new Error("The scene root is unavailable for room-boundary validation.");
          groupChanges = [createRoomAwareSofaMovePlan(root, target, patch.position)];
        } else if (staircaseRequest) {
          if (dimensions.some((value, axis) => Math.abs(patch.dimensions[axis] - value) > 0.08))
            throw new Error("The staircase patch changed assembly dimensions; the move was rejected before mutation.");
          if (!root) throw new Error("The scene root is unavailable for floor-connection validation.");
          groupChanges = createStaircaseMovePlan(root, target, patch.position);
        }
        if (groupChanges) {
          transactionAffectedIds = groupChanges.map((change) => change.target.uuid);
          transactionAffectedNames = groupChanges.map((change) => change.target.name);
        }
        const before = target.matrix.clone();
        const beforeMaterial = target instanceof THREE.Mesh ? target.material : undefined;
        let beforeGeometry: THREE.BufferGeometry | undefined;
        let afterGeometry: THREE.BufferGeometry | undefined;
        let generatedWindow: THREE.Group | undefined;
        if (openingRequest) {
          if (!(target instanceof THREE.Mesh) || !bounds) throw new Error("Select a wall mesh to cut a window opening.");
          if (windowOpeningTargets.current.has(target) || target.getObjectByName(`window_assembly_${target.name}`))
            throw new Error("This wall already has a window opening; select a different wall or edit the existing frame and glass.");
          const sourceGeometry = target.geometry;
          beforeGeometry = sourceGeometry;
          // Build and validate the complete assembly before changing the wall so
          // a failure cannot leave a partial opening in the live scene.
          const openingGeometry = createWindowOpeningGeometry(sourceGeometry);
          generatedWindow = createWindowAssembly(sourceGeometry);
          generatedWindow.name = `window_assembly_${target.name}`;
          generatedWindow.traverse((object) => {
            if (object instanceof THREE.Mesh) { transactionAffectedIds.push(object.uuid); transactionAffectedNames.push(object.name); }
          });
          afterGeometry = openingGeometry;
          target.geometry = openingGeometry;
          target.add(generatedWindow);
          const depth = sceneObjects.find((entry) => entry.object === target)?.depth ?? 0;
          setSceneObjects((items) => [...items, { object: generatedWindow!, label: generatedWindow!.name, depth: depth + 1 }]);
          windowOpeningTargets.current.add(target);
        }
        if (!groupChanges && target instanceof THREE.Mesh && !openingRequest) {
          const updateMaterialPatch = (source: THREE.Material) => {
            const clone = source.clone();
            if (clone instanceof THREE.MeshStandardMaterial) {
              clone.color.setRGB(...patch.color);
              clone.metalness = patch.metallic;
              clone.roughness = patch.roughness;
              clone.needsUpdate = true;
            }
            return clone;
          };
          target.material = Array.isArray(target.material) ? target.material.map(updateMaterialPatch) : updateMaterialPatch(target.material);
        }
        if (groupChanges) {
          for (const change of groupChanges) {
            change.target.matrix.copy(change.after);
            change.target.matrix.decompose(change.target.position, change.target.quaternion, change.target.scale);
            change.target.updateMatrix();
          }
        } else if (!openingRequest) {
          target.position.set(...patch.position);
          target.rotation.set(...patch.rotation);
        }
        if (!openingRequest && bounds) {
          const raw = [bounds.max.x - bounds.min.x, bounds.max.y - bounds.min.y, bounds.max.z - bounds.min.z];
          target.scale.set(...patch.dimensions.map((dimension, axis) => dimension / Math.max(1e-5, raw[axis])) as [number, number, number]);
        } else if (!openingRequest && groupBounds && !staircaseRequest) {
          const priorScale = [target.scale.x, target.scale.y, target.scale.z];
          const current = [groupBounds.max.x - groupBounds.min.x, groupBounds.max.y - groupBounds.min.y, groupBounds.max.z - groupBounds.min.z];
          target.scale.set(...patch.dimensions.map((dimension, axis) => priorScale[axis] * dimension / Math.max(1e-5, current[axis])) as [number, number, number]);
        }
        target.updateMatrix();
        const afterMaterial = target instanceof THREE.Mesh ? target.material : undefined;
        pushUndo({ target, before, after: target.matrix.clone(), beforeGeometry, afterGeometry, beforeMaterial, afterMaterial, generated: generatedWindow, openingEdit: Boolean(beforeGeometry), groupChanges });
        clearRedo();
      } else {
        const parent = target;
        const geometry = patch.primitive === "sphere" ? new THREE.SphereGeometry(1, 24, 16)
          : patch.primitive === "cylinder" || patch.primitive === "wheel" ? new THREE.CylinderGeometry(0.5, 0.5, 1, 24)
            : patch.primitive === "cone" ? new THREE.ConeGeometry(0.5, 1, 24)
              : new THREE.BoxGeometry(1, 1, 1);
        const mesh = new THREE.Mesh(geometry, new THREE.MeshStandardMaterial({ color: new THREE.Color(...patch.color), metalness: patch.metallic, roughness: patch.roughness }));
        let candidateName = patch.name;
        let suffix = 2;
        const root = objectRef.current;
        while (root && findSceneNode(root, candidateName)) candidateName = `${patch.name} ${suffix++}`;
        mesh.name = candidateName;
        mesh.position.set(...patch.position);
        mesh.rotation.set(...patch.rotation);
        mesh.scale.set(...patch.dimensions.map((dimension) => patch.primitive === "sphere" ? dimension / 2 : dimension) as [number, number, number]);
        mesh.updateMatrix();
        parent.add(mesh);
        const parentRow = sceneObjects.find((entry) => entry.object === parent);
        setSceneObjects((old) => [...old, { object: mesh, label: mesh.name, depth: (parentRow?.depth ?? 0) + 1 }]);
        const identity = new THREE.Matrix4();
        pushUndo({ target: mesh, before: identity, after: mesh.matrix.clone(), added: true });
        clearRedo();
        selectionRef.current = mesh;
        setSelection(mesh);
        setSelected(mesh.name);
      }
      cloudMutationCommitted = true;
      setTransform({ position: selectionRef.current?.position.toArray() ?? target.position.toArray(), rotation: selectionRef.current ? transformValues(selectionRef.current, "rotation") : transformValues(target, "rotation"), scale: selectionRef.current?.scale.toArray() ?? target.scale.toArray() });
      setAiEditPrompt("");
      renderRef.current();
      const provider = aiEditRoute === "cloud" ? "Cloud" : "local";
      const geometryOperation = operation !== "edit" ? operation
        : /\b(?:window|opening)\b/i.test(request) ? "window_with_frame_and_glass"
          : target.name === "Balcony_slab" && /\b(?:balcony|railing|width|widen)\b/i.test(request) ? "balcony_assembly_resize"
            : /sofa/i.test(target.name) && /\bmove\b/i.test(request) ? "room_aware_sofa_move"
              : target.name === "Staircase" && /\b(?:staircase|stairs)\b/i.test(request) ? "connected_staircase_move" : operation;
      const newRecord = { provider, modelId: result.modelId, requestId: result.requestId ?? result.providerRequestId ?? result.clientRequestId ?? null, clientRequestId: result.clientRequestId ?? null, providerRequestId: result.providerRequestId ?? null, httpStatus: result.httpStatus ?? null, finishReason: result.finishReason ?? null, editApplied: true, usage: result.usage ?? null, prompt: request, schemaVersion: 1, operation: geometryOperation, targetObjectIds: [target.uuid], affectedObjectIds: transactionAffectedIds, affectedObjectNames: transactionAffectedNames, addedObjectCount: geometryOperation === "window_with_frame_and_glass" ? 5 : operation === "add" ? 1 : 0, response: patch, createdAt: new Date().toISOString() };
      aiEditRecords.current = [...aiEditRecords.current, newRecord].slice(-24);
      const tokens = result.usage?.total_tokens;
      setAiEditStatus(`Applied ${provider} patch using ${result.modelId}${result.httpStatus ? ` · HTTP ${result.httpStatus}` : ""}${result.finishReason ? ` · ${result.finishReason}` : ""}${typeof tokens === "number" ? ` · ${tokens} tokens` : " · usage unavailable"}. Save a revision to retain it.`);
    } catch (caught) {
      setError(`${formatError(caught)}${cloudResponseDiagnostics ? `\n${cloudResponseDiagnostics} edit_applied=${cloudMutationCommitted ? "true" : "false"}.` : ""}`);
    } finally {
      setAiEditBusy(false);
    }
  };

  const travelHistory = (direction: "undo" | "redo") => {
    const source = direction === "undo" ? undoStackRef.current : redoStackRef.current;
    const entry = source[source.length - 1];
    if (!entry) return;
    if (direction === "undo") {
      undoStackRef.current = source.slice(0, -1);
      setUndoStack(undoStackRef.current);
    } else {
      redoStackRef.current = source.slice(0, -1);
      setRedoStack(redoStackRef.current);
    }
    if (entry.added) entry.target.visible = direction === "redo";
    if (entry.generated) entry.generated.visible = direction === "redo";
    if (entry.groupChanges) {
      for (const change of entry.groupChanges) {
        change.target.matrix.copy(direction === "undo" ? change.before : change.after);
        change.target.matrix.decompose(change.target.position, change.target.quaternion, change.target.scale);
        change.target.updateMatrix();
      }
    } else {
      entry.target.matrix.copy(direction === "undo" ? entry.before : entry.after);
      entry.target.matrix.decompose(entry.target.position, entry.target.quaternion, entry.target.scale);
    }
    const geometry = direction === "undo" ? entry.beforeGeometry : entry.afterGeometry;
    const material = direction === "undo" ? entry.beforeMaterial : entry.afterMaterial;
    if (geometry && entry.target instanceof THREE.Mesh) entry.target.geometry = geometry;
    if (entry.openingEdit && entry.target instanceof THREE.Mesh) {
      if (direction === "undo") windowOpeningTargets.current.delete(entry.target);
      else windowOpeningTargets.current.add(entry.target);
    }
    if (material && entry.target instanceof THREE.Mesh) entry.target.material = material;
    entry.target.updateMatrix();
    if (direction === "undo") {
      redoStackRef.current = [...redoStackRef.current, entry].slice(-100);
      setRedoStack(redoStackRef.current);
    } else {
      undoStackRef.current = [...undoStackRef.current, entry].slice(-100);
      setUndoStack(undoStackRef.current);
    }
    if (entry.target === selection) setTransform({
      position: entry.target.position.toArray(),
      rotation: transformValues(entry.target, "rotation"),
      scale: entry.target.scale.toArray(),
    });
    renderRef.current();
  };

  useEffect(() => {
    renderRef.current();
  }, [transform]);

  const exportEdited = async () => {
    const object = objectRef.current;
    if (!object) return;
    setError("");
    try {
      const hiddenPreviewGroups = new Set<string>([
        ...(!floorVisibility.ground ? ["Ground Floor"] : []),
        ...(!floorVisibility.upper ? ["Upper Floor"] : []),
        ...(cutaway ? ["Roof"] : []),
      ]);
      const result = await new GLTFExporter().parseAsync(createExportScene(object, lightObjects.current, exploded ? explodedPositionsRef.current : undefined, hiddenPreviewGroups), { binary: true, animations: animationsRef.current });
      if (!(result instanceof ArrayBuffer) || result.byteLength < 12) throw new Error("The edited scene could not be exported as GLB.");
      const bytes = new Uint8Array(result);
      if (String.fromCharCode(...bytes.subarray(0, 4)) !== "glTF") throw new Error("The edited export failed GLB validation.");
      const url = URL.createObjectURL(new Blob([bytes], { type: "model/gltf-binary" }));
      const anchor = document.createElement("a");
      anchor.href = url;
      anchor.download = "openmindai-edited-scene.glb";
      anchor.click();
      window.setTimeout(() => URL.revokeObjectURL(url), 3000);
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Could not export the edited scene.");
    }
  };

  const saveRevision = async () => {
    const object = objectRef.current;
    if (!object || !onSaveRevision) return;
    setError("");
    try {
      const hiddenPreviewGroups = new Set<string>([
        ...(!floorVisibility.ground ? ["Ground Floor"] : []),
        ...(!floorVisibility.upper ? ["Upper Floor"] : []),
        ...(cutaway ? ["Roof"] : []),
      ]);
      const result = await new GLTFExporter().parseAsync(createExportScene(object, lightObjects.current, exploded ? explodedPositionsRef.current : undefined, hiddenPreviewGroups), { binary: true, animations: animationsRef.current });
      if (!(result instanceof ArrayBuffer) || result.byteLength < 12) throw new Error("The edited scene could not be saved as GLB.");
      const bytes = new Uint8Array(result);
      if (new DataView(result).getUint32(0, true) !== 0x46546c67) throw new Error("The edited scene failed GLB validation.");
      let binary = "";
      for (let offset = 0; offset < bytes.length; offset += 0x8000)
        binary += String.fromCharCode(...bytes.subarray(offset, offset + 0x8000));
      await onSaveRevision(window.btoa(binary), { formatVersion: 1, storage: "glb", lights, backgroundAssetId: backgroundAssetId || null, floorVisibility, aiEdits: aiEditRecords.current, savedAt: new Date().toISOString() });
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Could not save the edited scene.");
    }
  };

  const addKeyframe = () => {
    onSceneModified?.();
    const target = selectionRef.current;
    if (!target) return;
    if (!target.name || !target.name.startsWith("OM3D_")) target.name = `OM3D_${crypto.randomUUID().replace(/-/g, "")}`;
    const time = Math.max(0, clipTimeRef.current);
    const frames = userFrames.current.get(target.name) ?? { times: [], positions: [], quaternions: [], scales: [] };
    let index = frames.times.findIndex((value) => Math.abs(value - time) < 0.0001);
    if (index < 0) { index = frames.times.length; frames.times.push(time); frames.positions.push([]); frames.quaternions.push([]); frames.scales.push([]); }
    frames.positions[index] = target.position.toArray();
    frames.quaternions[index] = target.quaternion.toArray();
    frames.scales[index] = target.scale.toArray();
    const order = frames.times.map((_, i) => i).sort((a, b) => frames.times[a] - frames.times[b]);
    frames.times = order.map((i) => frames.times[i]);
    frames.positions = order.map((i) => frames.positions[i]);
    frames.quaternions = order.map((i) => frames.quaternions[i]);
    frames.scales = order.map((i) => frames.scales[i]);
    userFrames.current.set(target.name, frames);
    const tracks: THREE.KeyframeTrack[] = [];
    for (const [name, sample] of userFrames.current) {
      if (!sample.times.length) continue;
      tracks.push(new THREE.VectorKeyframeTrack(`${name}.position`, sample.times, sample.positions.flat()));
      tracks.push(new THREE.QuaternionKeyframeTrack(`${name}.quaternion`, sample.times, sample.quaternions.flat()));
      tracks.push(new THREE.VectorKeyframeTrack(`${name}.scale`, sample.times, sample.scales.flat()));
    }
    const clip = new THREE.AnimationClip(generatedTrackName.current, Math.max(0.1, ...Array.from(userFrames.current.values()).flatMap((value) => value.times)), tracks);
    const next = [...clips.filter((item) => item.name !== clip.name), clip];
    animationsRef.current = next;
    setClips(next);
    const nextIndex = next.length - 1;
    setClipIndex(nextIndex);
  };

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (!(event.ctrlKey || event.metaKey) || event.key.toLowerCase() !== "z") return;
      const target = event.target as HTMLElement | null;
      if (target?.closest("input,textarea,select,[contenteditable='true']")) return;
      event.preventDefault();
      travelHistory(event.shiftKey ? "redo" : "undo");
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  });

  return (
    <section aria-label="3D mesh preview">
      <div className="studio-editor">
        <div className="studio-viewer" ref={viewerRef}>
        <div className="studio-viewport" ref={host} />
        <div className="studio-viewer-tools">
          <button type="button" disabled={!ready} onClick={() => reset.current()}>
            Reset view
          </button>
          <button type="button" disabled={!ready} onClick={() => setViewRef.current("exterior")}>Exterior</button>
          <button type="button" disabled={!ready} onClick={() => setViewRef.current("interior")}>Interior</button>
          <button type="button" disabled={!ready} onClick={() => setViewRef.current("top")}>Top-down</button>
          <button type="button" disabled={!ready} onClick={() => setViewRef.current("isometric")}>Isometric</button>
          <button type="button" disabled={!ready} className={cutaway ? "active" : ""} aria-pressed={cutaway} onClick={() => setCutaway((value) => !value)}>Cutaway</button>
          {cutaway && <label className="studio-cut-control">Section X<input aria-label="Section cut position" type="range" min="-6" max="6" step="0.1" value={cutPosition} onChange={(event) => setCutPosition(Number(event.target.value))} /></label>}
          <button type="button" disabled={!ready} className={exploded ? "active" : ""} aria-pressed={exploded} onClick={() => setExploded((value) => !value)}>{exploded ? "Restore assembly" : "Exploded view"}</button>
          <button type="button" disabled={!ready} className={walkthrough ? "active" : ""} aria-pressed={walkthrough} onClick={() => setWalkthrough((value) => !value)}>{walkthrough ? "Exit walkthrough" : "Walkthrough"}</button>
          {walkthrough && <label className="studio-cut-control">Speed<input aria-label="Walkthrough speed" type="range" min="0.5" max="6" step="0.25" value={walkSpeed} onChange={(event) => setWalkSpeed(Number(event.target.value))} /></label>}
          <button
            type="button"
            disabled={!ready}
            className={autoRotate ? "active" : ""}
            aria-pressed={autoRotate}
            onClick={() => setAutoRotate((value) => !value)}
          >
            {autoRotate ? "Stop spin" : "Auto-rotate"}
          </button>
          {clips.length > 0 && <button type="button" className={autoPlay ? "active" : ""} aria-pressed={autoPlay} onClick={() => {
            const next = !autoPlay;
            setAutoPlay(next);
            if (next && clips.length > 0) setPlaying(true);
          }}>
            Auto-play {autoPlay ? "On" : "Off"}
          </button>}
          <button type="button" disabled={!ready} onClick={() => {
            const name = `Studio animation ${clips.filter((clip) => clip.name.startsWith("Studio animation")).length + 1}`;
            generatedTrackName.current = name;
            userFrames.current.clear();
            const next = [...clips, new THREE.AnimationClip(name, 1, [])];
            animationsRef.current = next; setClips(next); setClipIndex(next.length - 1); clipTimeRef.current = 0; setClipTime(0);
            if (autoPlayRef.current) setPlaying(true);
          }}>New animation</button>
          {clips.length > 0 && <div className="studio-timeline" aria-label="Animation timeline">
            <select aria-label="Animation track" value={clipIndex} onChange={(event) => { setClipIndex(Number(event.target.value)); clipTimeRef.current = 0; setClipTime(0); }}>
              {clips.map((clip, index) => <option key={`${clip.name}-${index}`} value={index}>{clip.name || `Animation ${index + 1}`}</option>)}
            </select>
            <button type="button" onClick={() => { setPlaying(false); clipTimeRef.current = 0; setClipTime(0); mixerRef.current?.setTime(0); renderRef.current(); }}>Stop</button>
            <button type="button" aria-pressed={playing} onClick={() => setPlaying((value) => !value)}>{playing ? "Pause" : "Play"}</button>
            <button type="button" disabled={!selection} onClick={addKeyframe}>Add keyframe</button>
            <input aria-label="Timeline scrubber" type="range" min="0" max={clips[clipIndex]?.duration ?? 1} step="0.01" value={Math.min(clipTime, clips[clipIndex]?.duration ?? 0)} onChange={(event) => { const time = Number(event.target.value); clipTimeRef.current = time; setClipTime(time); mixerRef.current?.setTime(time); renderRef.current(); }} />
            <label><input type="checkbox" checked={loop} onChange={(event) => setLoop(event.target.checked)} />Loop</label>
            <select aria-label="Playback speed" value={playbackSpeed} onChange={(event) => setPlaybackSpeed(Number(event.target.value))}><option value="0.25">0.25×</option><option value="0.5">0.5×</option><option value="1">1×</option><option value="2">2×</option></select>
          </div>}
          <button
            type="button"
            disabled={!ready}
            className={wireframe ? "active" : ""}
            aria-pressed={wireframe}
            onClick={() => setWireframe((value) => !value)}
          >
            Wireframe
          </button>
        </div>
        <div className={`studio-viewer-corner-controls${isFullscreen ? " is-fullscreen" : ""}`}>
          <div className="studio-viewer-zoom" aria-label="Zoom controls">
            <button type="button" title="Zoom out" aria-label="Zoom out" disabled={!ready || zoomPercent <= 20} onClick={() => zoom.current(1.2)}>−</button>
            <select aria-label="Zoom percentage" value={zoomPercent} onChange={(event) => zoom.current(zoomPercent / Math.max(Number(event.target.value), 1))}>
              <option value={zoomPercent}>{zoomPercent}%</option>
              {[25, 50, 75, 100, 125, 150, 200, 300, 400, 500].filter((value) => value !== zoomPercent).map((value) => <option key={value} value={value}>{value}%</option>)}
            </select>
            <button type="button" title="Zoom in" aria-label="Zoom in" disabled={!ready || zoomPercent >= 500} onClick={() => zoom.current(1 / 1.2)}>+</button>
          </div>
          <button type="button" className="studio-fullscreen-toggle" title={isFullscreen ? "Exit full screen" : "Full screen"} aria-label={isFullscreen ? "Exit full screen" : "Enter full screen"} aria-pressed={isFullscreen} onClick={toggleFullscreen}>
            <svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
              {isFullscreen
                ? <path d="M8 3v6H2m20-6-7 7M2 14l6 7v-6m14-1-7 7" />
                : <path d="M15 3h6v6M9 21H3v-6m18-12-7 7M3 21l7-7" />}
            </svg>
          </button>
        </div>
        {error && (
          <p className="studio-preview-error" role="alert">
            {error}
          </p>
        )}
        {!ready && !error && (
          <p className="studio-preview-error" role="status">
            Loading validated geometry…
          </p>
        )}
        {selected && <div className="studio-node">Selected: {selected}</div>}
        {walkthrough && <div className="studio-walk-guide" role="status">Click viewport to capture mouse · WASD move · mouse look · Esc returns to orbit</div>}
        </div>
        <div className="studio-right-rail">
        <aside className="studio-scene-tree" aria-label="Scene hierarchy">
          <strong>Scene</strong>
          <div className="studio-light-actions" aria-label="Floor visibility">
            <label>Ground floor<input type="checkbox" checked={floorVisibility.ground} onChange={(event) => setFloorVisibility((value) => ({ ...value, ground: event.target.checked }))} /></label>
            <label>Upper floor<input type="checkbox" checked={floorVisibility.upper} onChange={(event) => setFloorVisibility((value) => ({ ...value, upper: event.target.checked }))} /></label>
          </div>

          <button type="button" className="studio-history-button" disabled={!undoStack.length} onClick={() => travelHistory("undo")}>Undo</button>
          <button type="button" className="studio-history-button" disabled={!redoStack.length} onClick={() => travelHistory("redo")}>Redo</button>
          <button type="button" className="studio-history-button" disabled={!selection} onClick={duplicateObject}>Duplicate selected object</button>
          <ul>{lights.map((light, index) => <li key={light.id}><button type="button" title={`${light.type} light`} onClick={() => document.getElementById(`studio-light-${light.id}`)?.scrollIntoView({ block: "nearest" })} style={{ paddingLeft: "8px" }}>Light · {light.type} {index + 1}</button></li>)}{sceneObjects.map(({ object, label, depth }, index) => <li key={`${index}-${label}`}>
            <button type="button" className={selection === object ? "selected" : ""} title={label} onClick={() => {
              selectionRef.current = object; setSelection(object); setSelected(label);
              setTransform({ position: object.position.toArray(), rotation: transformValues(object, "rotation"), scale: object.scale.toArray() });
              setDrafts(Object.fromEntries((["position", "rotation", "scale"] as const).flatMap((field) =>
                (["X", "Y", "Z"] as const).map((axis, axisIndex) => [
                  `${field}-${axis}`,
                  String(Number((field === "rotation" ? THREE.MathUtils.radToDeg(transformValues(object, field)[axisIndex]) : transformValues(object, field)[axisIndex]).toFixed(3))),
                ]),
              )));
            }} style={{ paddingLeft: `${8 + depth * 12}px` }}>{label}</button>
          </li>)}</ul>
        </aside>
        <aside className="studio-inspector" aria-label="Selected object properties">
          <strong>Lighting</strong>
          <div className="studio-light-actions"><button type="button" onClick={() => setLights((old) => [...old, { id: crypto.randomUUID(), type: "ambient", color: "#ffffff", intensity: 0.5, position: [0, 0, 0], visible: true }])}>Add ambient</button><button type="button" onClick={() => setLights((old) => [...old, { id: crypto.randomUUID(), type: "point", color: "#ffffff", intensity: 2, position: [0, 3, 0], visible: true }])}>Add point</button><button type="button" onClick={() => setLights((old) => [...old, { id: crypto.randomUUID(), type: "directional", color: "#ffffff", intensity: 1, position: [3, 5, 2], visible: true }])}>Add directional</button></div>
          {lights.map((light, index) => <fieldset id={`studio-light-${light.id}`} key={light.id}><legend>{light.type} {index + 1}</legend>
            <label>Enabled<input type="checkbox" checked={light.visible} onChange={(event) => setLights((old) => old.map((item) => item.id === light.id ? { ...item, visible: event.target.checked } : item))} /></label>
            <label>Color<input type="color" value={light.color} onChange={(event) => setLights((old) => old.map((item) => item.id === light.id ? { ...item, color: event.target.value } : item))} /></label>
            <label>Intensity<input type="number" min="0" max="100" step="0.1" value={light.intensity} onChange={(event) => setLights((old) => old.map((item) => item.id === light.id ? { ...item, intensity: THREE.MathUtils.clamp(Number(event.target.value) || 0, 0, 100) } : item))} /></label>
            {light.type !== "ambient" && <div className="studio-light-position">{(["X", "Y", "Z"] as const).map((axis, i) => <label key={axis}>{axis}<input type="number" value={light.position[i]} step="0.1" onChange={(event) => setLights((old) => old.map((item) => item.id === light.id ? { ...item, position: item.position.map((v, j) => j === i ? THREE.MathUtils.clamp(Number(event.target.value) || 0, -100000, 100000) : v) as [number, number, number] } : item))} /></label>)}</div>}
            <button type="button" onClick={() => setLights((old) => old.filter((item) => item.id !== light.id))}>Remove</button>
            <button type="button" onClick={() => setLights((old) => [...old, { ...light, id: crypto.randomUUID(), position: [...light.position] as [number, number, number] }])}>Duplicate</button>
          </fieldset>)}
          <p>Ambient light settings are saved with the project. GLB export includes directional and point lights.</p>
          <strong>Project images</strong>
          <label>Image<select aria-label="Select project image" value={selectedImageAsset} onChange={(event) => setSelectedImageAsset(event.target.value)}><option value="">Choose project image…</option>{imageAssets.map((asset) => <option key={asset.id} value={asset.id}>{asset.name}</option>)}</select></label>
          <div className="studio-light-actions"><button type="button" disabled={!selectedImageAsset} onClick={() => void placeImagePlane()}>Place 2D image plane</button><button type="button" disabled={!selectedImageAsset} onClick={setBackgroundImage}>Set background</button></div>
          <p>Image planes remain flat 2D references. They are not neural 3D reconstruction.</p>
          <strong>AI scene edit</strong>
          <p>Returns one validated object patch. Existing nodes and geometry stay in the scene; no full-scene regeneration.</p>
          <label>Reasoning route<select aria-label="AI edit reasoning route" value={aiEditRoute} onChange={(event) => setAiEditRoute(event.target.value as "local" | "cloud")}><option value="local">Local model</option><option value="cloud">Cloud chat model</option></select></label>
          {aiEditRoute === "cloud" && <p>Cloud receives the instruction and selected object fields only; this edit does not upload the scene asset or an image.</p>}
          <textarea aria-label="AI scene edit instruction" rows={3} maxLength={2000} value={aiEditPrompt} onChange={(event) => setAiEditPrompt(event.target.value)} placeholder={selection instanceof THREE.Mesh ? "Resize this furniture, move this stair, or change its material…" : "Add a lamp, chair, or other object inside this selected room…"} />
          <div className="studio-light-actions">
            <button type="button" disabled={!ready || aiEditBusy || !selection || aiEditPrompt.trim().length < 5} onClick={() => void applyAiSceneEdit("edit")}>{aiEditBusy ? "Planning…" : "Edit selected"}</button>
            <button type="button" disabled={!ready || aiEditBusy || !selection || aiEditPrompt.trim().length < 5} onClick={() => void applyAiSceneEdit("add")}>Add to selected</button>
          </div>
          {aiEditStatus && <p role="status">{aiEditStatus}</p>}
          <strong>Transform</strong>
          {selection ? (["position", "rotation", "scale"] as const).map((field) => <fieldset key={field}>
            <legend>{field === "rotation" ? "Rotation · degrees" : field[0].toUpperCase() + field.slice(1)}</legend>
            {(["X", "Y", "Z"] as const).map((axis, index) => {
              const key = `${field}-${axis}`;
              const display = drafts[key] ?? String(Number((field === "rotation" ? THREE.MathUtils.radToDeg(transform[field][index]) : transform[field][index]).toFixed(3)));
              return <label key={axis}>{axis}<input
                aria-label={`${field} ${axis}`}
                type="number"
                step={field === "rotation" ? 1 : 0.1}
                value={display}
                onChange={(event) => {
                  const value = event.target.value;
                  setDrafts((old) => ({ ...old, [key]: value }));
                }}
                onBlur={() => commitTransform(field, index as 0 | 1 | 2, drafts[key] ?? display)}
              /></label>;
            })}
          </fieldset>) : <p>Select an object in the hierarchy or viewport.</p>}
          <strong>Material</strong>
          {selectedMesh && selectedMaterial instanceof THREE.MeshStandardMaterial ? <>
            <label>Base color<input type="color" aria-label="Material base color" value={`#${selectedMaterial.color.getHexString()}`} onChange={(event) => updateMaterial((material) => material.color.set(event.target.value))} /></label>
            <label>Metallic<input aria-label="Material metallic" type="number" min="0" max="1" step="0.01" value={selectedMaterial.metalness} onChange={(event) => updateMaterial((material) => { material.metalness = THREE.MathUtils.clamp(Number(event.target.value) || 0, 0, 1); })} /></label>
            <label>Roughness<input aria-label="Material roughness" type="number" min="0" max="1" step="0.01" value={selectedMaterial.roughness} onChange={(event) => updateMaterial((material) => { material.roughness = THREE.MathUtils.clamp(Number(event.target.value) || 0, 0, 1); })} /></label>
            <label>Opacity<input aria-label="Material opacity" type="number" min="0" max="1" step="0.01" value={selectedMaterial.opacity} onChange={(event) => updateMaterial((material) => { material.opacity = THREE.MathUtils.clamp(Number(event.target.value) || 0, 0, 1); material.transparent = material.opacity < 1; })} /></label>
            <label>Emissive<input aria-label="Material emissive color" type="color" value={`#${selectedMaterial.emissive.getHexString()}`} onChange={(event) => updateMaterial((material) => material.emissive.set(event.target.value))} /></label>
            <label>Image texture<select aria-label="Assign image texture" value="" onChange={(event) => { if (event.target.value) void applyTexture(event.target.value); }}>
              <option value="">Choose project imageвЂ¦</option>
              {imageAssets.map((asset) => <option key={asset.id} value={asset.id}>{asset.name}</option>)}
            </select></label>
            {selectedMaterial.map && <p>Texture assigned: {selectedMaterial.map.name || "project image"}</p>}
            <div className="studio-material-actions">
              <button type="button" onClick={duplicateMaterial}>Duplicate material</button>
              <button type="button" onClick={resetMaterial}>Reset material</button>
            </div>
            <span className="studio-visually-hidden" aria-live="polite">Material updated {materialRevision}</span>
          </> : <p>Select a PBR mesh to edit its material.</p>}
          <button type="button" disabled={!ready} onClick={() => void exportEdited()}>Export edited GLB</button>
          <button type="button" disabled={!ready || !onSaveRevision} onClick={() => void saveRevision()}>Save project revision</button>
        </aside>
        </div>
      </div>
    </section>
  );
}
