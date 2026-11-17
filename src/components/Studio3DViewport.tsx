import { useEffect, useRef, useState } from "react";
import * as THREE from "three";
import { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";

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

export function Studio3DViewport({ dataBase64 }: { dataBase64: string }) {
  const host = useRef<HTMLDivElement>(null);
  const reset = useRef<() => void>(() => {});
  const controlsRef = useRef<OrbitControls | null>(null);
  const renderRef = useRef<() => void>(() => {});
  const objectRef = useRef<THREE.Object3D | null>(null);
  const rafRef = useRef<number | null>(null);
  const [error, setError] = useState("");
  const [ready, setReady] = useState(false);
  const [selected, setSelected] = useState("");
  const [autoRotate, setAutoRotate] = useState(false);
  const [wireframe, setWireframe] = useState(false);

  useEffect(() => {
    let disposed = false;
    const cleaners: (() => void)[] = [];
    const cleanup = () => {
      if (rafRef.current !== null) cancelAnimationFrame(rafRef.current);
      rafRef.current = null;
      while (cleaners.length) cleaners.pop()?.();
      controlsRef.current = null;
      objectRef.current = null;
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

    setReady(false);
    setError("");
    setSelected("");

    (async () => {
      try {
        const buffer = base64ToArrayBuffer(dataBase64);
        const gltf = await new GLTFLoader().parseAsync(buffer, "");
        const object = gltf.scene;
        if (disposed) {
          disposeObject(object);
          return;
        }
        cleaners.push(() => disposeObject(object));
        objectRef.current = object;
        const container = host.current!;
        const read = (token: string, fallback: string) =>
          getComputedStyle(container).getPropertyValue(token).trim() || fallback;
        const surface = new THREE.Color(read("--bg", "#303030"));

        const renderer = new THREE.WebGLRenderer({ antialias: true });
        renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
        cleaners.push(() => {
          renderer.dispose();
          renderer.domElement.remove();
        });

        const scene = new THREE.Scene();
        scene.background = surface;
        scene.add(new THREE.AmbientLight(0xffffff, 1.4));
        const key = new THREE.DirectionalLight(0xffffff, 2.6);
        key.position.set(5, 8, 4);
        scene.add(key);
        const fill = new THREE.DirectionalLight(0xffffff, 0.8);
        fill.position.set(-6, 3, -4);
        scene.add(fill);
        scene.add(object);

        const bounds = new THREE.Box3().setFromObject(object);
        const center = bounds.getCenter(new THREE.Vector3());
        const span = bounds.getSize(new THREE.Vector3());
        const size = span.length();

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
        cleaners.push(() => controls.dispose());
        controls.target.copy(center);

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
          setSelected(hit?.object.name || (hit ? "Mesh part" : ""));
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
          if ("wireframe" in material) (material as THREE.MeshStandardMaterial).wireframe = wireframe;
    });
    renderRef.current();
  }, [wireframe, ready]);

  return (
    <section aria-label="3D mesh preview">
      <div className="studio-viewer">
        <div className="studio-viewport" ref={host} />
        <div className="studio-viewer-tools">
          <button type="button" disabled={!ready} onClick={() => reset.current()}>
            Reset view
          </button>
          <button
            type="button"
            disabled={!ready}
            className={autoRotate ? "active" : ""}
            aria-pressed={autoRotate}
            onClick={() => setAutoRotate((value) => !value)}
          >
            {autoRotate ? "Stop spin" : "Auto-rotate"}
          </button>
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
      </div>
    </section>
  );
}
