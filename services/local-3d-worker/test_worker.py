"""Unit and HTTP contract tests for the local OpenMindAI 3D GPU worker.

These tests run without OpenCL; a fake device models GPU transform outputs.
They do not claim to validate a physical GPU/driver installation.
"""
import base64
import http.client
import importlib.util
import json
from pathlib import Path
import struct
import threading
import time
import unittest

MODULE = Path(__file__).with_name("worker.py")
spec = importlib.util.spec_from_file_location("openmind_local_3d_worker", MODULE)
worker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(worker)


class MockGpu:
    name = "Mock GPU"

    def compute(self, vertices, scale, offset, angle):
        output = []
        for x, y, z in vertices:
            output.extend((float(x * scale[0] + offset[0]),
                           float(y * scale[1] + offset[1]),
                           float(z * scale[2] + offset[2])))
        return output


class LocalGpuWorkerTest(unittest.TestCase):
    def test_gpu_adapter_exposes_transforms_and_probe_preserves_cpu_readiness(self):
        self.assertTrue(callable(getattr(worker.GPUBackend, "compute", None)))
        backend, diagnostics = worker.probe_gpu()
        self.assertTrue(diagnostics["cpu_ready"])
        self.assertIn("python_version", diagnostics)
        self.assertIn("numpy", diagnostics)
        self.assertIn("pyopencl", diagnostics)
        self.assertEqual(diagnostics["gpu_ready"], backend is not None)

    def test_cpu_fallback_builds_cone_and_transforms_without_gpu(self):
        plan = {"objects": [
            {"name":"Cone","primitive":"cone","dimensions":[2,3,2],"position":[1,2,3],"rotation":[0,0,0],"color":[0.4,0.5,0.6],"metallic":0.1,"roughness":0.8,"parent":None},
        ]}
        payload, vertices, triangles = worker.make_glb("traffic cone", "object", None, plan)
        size = struct.unpack_from("<I", payload, 12)[0]
        data = json.loads(payload[20:20 + size])
        self.assertGreater(vertices, 20)
        self.assertGreater(triangles, 20)
        self.assertEqual(data["meshes"][0]["name"], "Cone")
        accessor = data["accessors"][0]
        self.assertEqual(accessor["min"], [0.0, 0.5, 2.0])
        self.assertEqual(accessor["max"], [2.0, 3.5, 4.0])

    def test_glb_contains_real_geometry(self):
        for mode, prompt in [
            ("object", "Generate a ring-shaped solid object"),
            ("object", "Generate a sphere"),
            ("architecture", "Design a four-wall building"),
        ]:
            with self.subTest(mode=mode, prompt=prompt):
                payload, vertices, triangles = worker.make_glb(prompt, mode, MockGpu())
                self.assertEqual(payload[:4], b"glTF")
                self.assertEqual(struct.unpack_from("<I", payload, 4)[0], 2)
                self.assertEqual(struct.unpack_from("<I", payload, 8)[0], len(payload))
                json_size, magic = struct.unpack_from("<I4s", payload, 12)
                self.assertEqual(magic, b"JSON")
                doc = json.loads(payload[20:20 + json_size])
                self.assertEqual(doc["asset"]["version"], "2.0")
                self.assertGreaterEqual(len(doc["meshes"]), 1)
                self.assertGreater(vertices, 12)
                self.assertGreater(triangles, 10)
                for mesh in doc["meshes"]:
                    primitive = mesh["primitives"][0]
                    self.assertEqual(primitive["mode"], 4)
                    self.assertIn("POSITION", primitive["attributes"])
                    self.assertIn("indices", primitive)

    def test_ai_room_layout_builds_editable_architecture_with_real_openings(self):
        layout = {"schemaVersion":2,"footprint":[12,10],"storyHeight":3.2,"rooms":[
            {"name":"Living Room","floor":"ground","x":-3,"z":-2.5,"width":5.8,"depth":4.8},
            {"name":"Dining Room","floor":"ground","x":2,"z":-2.5,"width":4.0,"depth":4.8},
            {"name":"Kitchen","floor":"ground","x":-3,"z":2.5,"width":5.8,"depth":4.8},
            {"name":"Ground Bathroom","floor":"ground","x":2,"z":2.5,"width":4.0,"depth":4.8},
            {"name":"Bedroom 1","floor":"upper","x":-4.0,"z":-2.5,"width":3.5,"depth":4.8},
            {"name":"Bedroom 2","floor":"upper","x":0.0,"z":-2.5,"width":4.0,"depth":4.8},
            {"name":"Bedroom 3","floor":"upper","x":4.0,"z":-2.5,"width":3.8,"depth":4.8},
            {"name":"Upper Bathroom","floor":"upper","x":-4.0,"z":2.5,"width":3.5,"depth":2.5},
            {"name":"Stair Landing","floor":"upper","x":0.0,"z":2.5,"width":4.0,"depth":1.2},
        ]}
        payload, vertices, triangles = worker.make_glb("Design a two-story house", "architecture", None, layout)
        size = struct.unpack_from("<I", payload, 12)[0]
        data = json.loads(payload[20:20 + size])
        names = {node["name"] for node in data["nodes"]}
        self.assertGreater(len(data["meshes"]), 100)
        self.assertGreater(vertices, 2_000)
        self.assertGreater(triangles, 1_000)
        self.assertTrue(any("glass pane" in name for name in names))
        self.assertTrue(any("wooden door" in name for name in names))
        self.assertIn("Stair tread 01", names)
        self.assertIn("Balcony front railing", names)
        groups = {node["name"]: node for node in data["nodes"]}
        child_names = {data["nodes"][index]["name"] for index in groups["Residential House"]["children"]}
        self.assertIn("Ground Floor", child_names)
        self.assertIn("Upper Floor", child_names)
        position_accessors = [data["accessors"][node["mesh"] * 2] for node in data["nodes"] if "mesh" in node]
        minimum = [min(accessor["min"][axis] for accessor in position_accessors) for axis in range(3)]
        maximum = [max(accessor["max"][axis] for accessor in position_accessors) for axis in range(3)]
        self.assertAlmostEqual(maximum[0]-minimum[0], 12.4, places=3)
        self.assertAlmostEqual(maximum[1], 7.015, places=3)
        self.assertAlmostEqual(minimum[1], -.36, places=3)
        # The front entrance wall is segmented above and beside the opening;
        # no concrete wall section crosses the door's clear rectangle.
        by_name = {node["name"]: node for node in data["nodes"]}
        for name, node in by_name.items():
            if not name.startswith("ground front wall section"):
                continue
            bounds = data["accessors"][node["mesh"]*2]
            z_min,z_max=bounds["min"][2],bounds["max"][2]
            if z_min < 5.13 and z_max > 4.87:
                x_overlap=bounds["min"][0] < .5 and bounds["max"][0] > -.5
                y_overlap=bounds["min"][1] < 2.34 and bounds["max"][1] > .24
                self.assertFalse(x_overlap and y_overlap, name)
        glass = next(material for material in data["materials"] if "glass" in material["name"].casefold())
        self.assertEqual(glass["alphaMode"], "BLEND")
        self.assertLess(glass["pbrMetallicRoughness"]["baseColorFactor"][3], 1)

    def test_architecture_rejects_missing_rooms_instead_of_falling_back_to_boxes(self):
        with self.assertRaisesRegex(RuntimeError, "all nine planned rooms"):
            worker.architecture_plan({"schemaVersion":2,"footprint":[12,10],"storyHeight":3.2,"rooms":[]})

    def test_room_aware_furnishing_modes_create_editable_components(self):
        layout = {"schemaVersion":2,"footprint":[12,10],"storyHeight":3.2,"rooms":[
            {"name":"Living Room","floor":"ground","x":-3,"z":-2.5,"width":5.8,"depth":4.8},
            {"name":"Dining Room","floor":"ground","x":2,"z":-2.5,"width":4.0,"depth":4.8},
            {"name":"Kitchen","floor":"ground","x":-3,"z":2.5,"width":5.8,"depth":4.8},
            {"name":"Ground Bathroom","floor":"ground","x":2,"z":2.5,"width":4.0,"depth":4.8},
            {"name":"Bedroom 1","floor":"upper","x":-4.0,"z":-2.5,"width":3.5,"depth":4.8},
            {"name":"Bedroom 2","floor":"upper","x":0.0,"z":-2.5,"width":4.0,"depth":4.8},
            {"name":"Bedroom 3","floor":"upper","x":4.0,"z":-2.5,"width":3.8,"depth":4.8},
            {"name":"Upper Bathroom","floor":"upper","x":-4.0,"z":2.5,"width":3.5,"depth":2.5},
            {"name":"Stair Landing","floor":"upper","x":0.0,"z":2.5,"width":4.0,"depth":1.2},
        ]}
        unfurnished = worker.architecture_plan(layout, "unfurnished")
        partial = worker.architecture_plan(layout, "partially furnished")
        full = worker.architecture_plan(layout, "fully furnished")
        unfurnished_names = {item["name"] for item in unfurnished}
        partial_names = {item["name"] for item in partial}
        full_names = {item["name"] for item in full}
        self.assertFalse(any("Sofa" in name for name in unfurnished_names))
        self.assertTrue(any("Sofa" in name for name in partial_names))
        self.assertFalse(any("Dining table" in name for name in partial_names))
        for expected in ("Sofa", "Coffee table", "Base cabinets", "Sink", "Cooktop", "Refrigerator", "Bed frame", "Wardrobe", "Dining table", "Toilet", "Shower enclosure"):
            self.assertTrue(any(expected in name for name in full_names), expected)
        self.assertGreater(len(full), len(partial))
        for item in full:
            if " · " not in item["name"]:
                continue
            room_name = item["parent"]
            room = next(value for value in layout["rooms"] if value["name"] == room_name)
            x, z = item["offset"][0], item["offset"][2]
            width, depth = item["scale"][0]*2, item["scale"][2]*2
            self.assertGreaterEqual(x-width/2, room["x"]-room["width"]/2-1e-6, item["name"])
            self.assertLessEqual(x+width/2, room["x"]+room["width"]/2+1e-6, item["name"])
            self.assertGreaterEqual(z-depth/2, room["z"]-room["depth"]/2-1e-6, item["name"])
            self.assertLessEqual(z+depth/2, room["z"]+room["depth"]/2+1e-6, item["name"])
        with self.assertRaisesRegex(RuntimeError, "Furnishing mode"):
            worker.architecture_plan(layout, "auto-install-furniture")
        glb, _, _ = worker.make_glb("Modern fully furnished residence", "architecture", None, layout)
        json_size = struct.unpack_from("<I", glb, 12)[0]
        document = json.loads(glb[20:20+json_size])
        reachable = set()
        pending = list(document["scenes"][0]["nodes"])
        while pending:
            index = pending.pop()
            if index in reachable:
                continue
            reachable.add(index)
            pending.extend(document["nodes"][index].get("children", []))
        furniture_nodes = [index for index, node in enumerate(document["nodes"]) if " · " in node.get("name", "")]
        self.assertGreater(len(furniture_nodes), 20)
        self.assertTrue(all(index in reachable for index in furniture_nodes), "every generated furniture component must be attached to the exported scene graph")

    def test_model_scene_spec_builds_separate_editable_pbr_meshes(self):
        plan = {"objects": [
            {"name":"Robot body","primitive":"box","dimensions":[2.0,1.0,1.2],"position":[0,0,0],"rotation":[0,0,0],"color":[0.2,0.4,0.8],"metallic":0.7,"roughness":0.25,"parent":None},
            {"name":"Left wheel","primitive":"cylinder","dimensions":[0.5,0.5,0.3],"position":[-0.8,-0.5,0],"rotation":[1.57,0,0],"color":[0.05,0.05,0.05],"metallic":0.1,"roughness":0.9,"parent":"Robot body"},
        ]}
        payload, vertices, triangles = worker.make_glb("delivery robot", "object", MockGpu(), plan)
        size = struct.unpack_from("<I", payload, 12)[0]
        data = json.loads(payload[20:20 + size])
        self.assertEqual(vertices, 72)
        self.assertGreater(triangles, 0)
        self.assertEqual([mesh["name"] for mesh in data["meshes"]], ["Robot body", "Left wheel"])
        self.assertEqual(data["materials"][0]["pbrMetallicRoughness"]["metallicFactor"], 0.7)
        self.assertEqual(data["materials"][1]["pbrMetallicRoughness"]["roughnessFactor"], 0.9)
        self.assertEqual(data["nodes"][0]["children"], [1])
        self.assertEqual(data["scenes"][0]["nodes"], [0])

    def test_job_processing_writes_glb(self):
        import tempfile
        with tempfile.TemporaryDirectory() as folder:
            old_gpu, old_dir = worker.GPU, worker.OUT_DIR
            try:
                worker.GPU = None
                worker.OUT_DIR = Path(folder)
                job_id = "f79180b0-58c2-4f83-af1d-826630e3132b"
                worker.JOBS[job_id] = {
                    "id": job_id, "status": "queued", "error": None, "output": None,
                    "_prompt": "Generate a house", "_mode": "architecture",
                    "_scene_spec": {"objects":[
                        {"name":"Building","primitive":"box","dimensions":[2,2,2],"position":[0,0,0],"rotation":[0,0,0],"color":[0.4,0.5,0.6],"metallic":0.2,"roughness":0.7,"parent":None},
                    ]},
                }
                worker.process_job(job_id)
                result = worker.JOBS[job_id]
                self.assertEqual(result["status"], "completed")
                self.assertGreater(result["output"]["vertices"], 0)
                self.assertEqual(result["output"]["geometry_backend"], "cpu")
                self.assertTrue((worker.OUT_DIR / (job_id + ".glb")).read_bytes().startswith(b"glTF"))
            finally:
                worker.GPU, worker.OUT_DIR = old_gpu, old_dir
                worker.JOBS.clear()
                worker.KEYS.clear()

    def test_http_requires_native_bridge_and_supports_mesh_download(self):
        import tempfile
        from http.server import ThreadingHTTPServer
        with tempfile.TemporaryDirectory() as folder:
            old_gpu, old_dir = worker.GPU, worker.OUT_DIR
            worker.GPU, worker.OUT_DIR = None, Path(folder)
            server = ThreadingHTTPServer(("127.0.0.1", 0), worker.Handler)
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            try:
                def request(method, path, body=None, headers=None):
                    c = http.client.HTTPConnection("127.0.0.1", server.server_port, timeout=5)
                    c.request(method, path, body=body, headers=headers or {})
                    response = c.getresponse()
                    status, data = response.status, json.loads(response.read())
                    c.close()
                    return status, data

                status, _ = request("GET", "/health")
                self.assertEqual(status, 403)
                status, _ = request("GET", "/health", headers={"Origin":"https://malicious.test",
                                                                "X-OpenMindAI-Desktop":"1"})
                self.assertEqual(status, 403)
                status, data = request("GET", "/health", headers={"X-OpenMindAI-Desktop":"1"})
                self.assertEqual(status, 200)
                self.assertFalse(data["gpu_active"])
                self.assertTrue(data["worker_ready"])
                self.assertTrue(data["cpu_ready"])
                request_body = json.dumps({
                    "request_key":"contract-test","prompt":"A procedural planet sphere",
                    "mode":"object",
                })
                status, result = request("POST", "/jobs", body=request_body,
                    headers={"X-OpenMindAI-Desktop":"1","Content-Type":"application/json"})
                self.assertEqual(status, 202)
                job_id = result["id"]
                # Same request key may be retried after a dropped reply; it
                # must return the original job rather than trigger another run.
                duplicate_status, duplicate = request("POST", "/jobs", body=request_body,
                    headers={"X-OpenMindAI-Desktop":"1","Content-Type":"application/json"})
                self.assertEqual(duplicate_status, 202)
                self.assertEqual(duplicate["id"], job_id)
                self.assertEqual(len(worker.JOBS), 1)
                for _ in range(200):
                    _, result = request("GET", "/jobs/" + job_id,
                                        headers={"X-OpenMindAI-Desktop":"1"})
                    if result["status"] == "completed": break
                    time.sleep(0.005)
                self.assertEqual(result["status"], "completed", result.get("error"))
                self.assertEqual(result["output"]["geometry_backend"], "cpu")
                status, mesh = request("GET", "/jobs/" + job_id + "/mesh",
                                       headers={"X-OpenMindAI-Desktop":"1"})
                self.assertEqual(status, 200)
                self.assertTrue(base64.b64decode(mesh["data_base64"]).startswith(b"glTF"))
            finally:
                server.shutdown()
                server.server_close()
                thread.join(timeout=5)
                worker.GPU, worker.OUT_DIR = old_gpu, old_dir
                worker.JOBS.clear()
                worker.KEYS.clear()


if __name__ == "__main__":
    unittest.main()
