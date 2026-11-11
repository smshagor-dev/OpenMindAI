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

    def test_architecture_has_four_named_meshes(self):
        payload, _, _ = worker.make_glb("Design a house", "architecture", MockGpu())
        size = struct.unpack_from("<I", payload, 12)[0]
        data = json.loads(payload[20:20 + size])
        self.assertEqual(len(data["meshes"]), 4)
        self.assertEqual([node["name"] for node in data["nodes"]],
                         ["Foundation", "Main building", "Roof", "Entrance"])

    def test_job_processing_writes_glb(self):
        import tempfile
        with tempfile.TemporaryDirectory() as folder:
            old_gpu, old_dir = worker.GPU, worker.OUT_DIR
            try:
                worker.GPU = MockGpu()
                worker.OUT_DIR = Path(folder)
                job_id = "f79180b0-58c2-4f83-af1d-826630e3132b"
                worker.JOBS[job_id] = {
                    "id": job_id, "status": "queued", "error": None, "output": None,
                    "_prompt": "Generate a house", "_mode": "architecture",
                }
                worker.process_job(job_id)
                result = worker.JOBS[job_id]
                self.assertEqual(result["status"], "completed")
                self.assertGreater(result["output"]["vertices"], 0)
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
            worker.GPU, worker.OUT_DIR = MockGpu(), Path(folder)
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
                self.assertTrue(data["gpu_active"])
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
