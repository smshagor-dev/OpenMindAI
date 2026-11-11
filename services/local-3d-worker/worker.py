#!/usr/bin/env python3
"""Optional local GPU procedural 3D worker for OpenMindAI Desktop.

Requires a functioning OpenCL GPU device and: pip install numpy pyopencl
Creates actual manifold-ish polygonal GLB 2.0 meshes; this is NOT a learned
text-to-3D generator and does not claim photographic/model quality.
Only listens on 127.0.0.1; browser Origin headers are rejected.
"""
from __future__ import annotations

import base64
import json
import math
import os
from pathlib import Path
import struct
import threading
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

HOST, PORT = "127.0.0.1", 8765
OUT_DIR = Path(os.environ.get("OPENMINDAI_3D_OUTPUT_DIR", str(Path.home() / "OpenMindAI" / "3DStudio")))
JOBS: dict[str, dict] = {}
KEYS: dict[str, str] = {}
LOCK = threading.Lock()
GPU = None
GPU_ERROR = None

KERNEL = """
__kernel void transform(__global const float *source, __global float *output,
    const float sx, const float sy, const float sz,
    const float tx, const float ty, const float tz, const float rotation) {
    int i = get_global_id(0);
    float x = source[i * 3] * sx;
    float y = source[i * 3 + 1] * sy;
    float z = source[i * 3 + 2] * sz;
    float c = cos(rotation), s = sin(rotation);
    output[i * 3] = x * c - z * s + tx;
    output[i * 3 + 1] = y + ty;
    output[i * 3 + 2] = x * s + z * c + tz;
}
"""

class GPUBackend:
    def __init__(self):
        import numpy as np
        import pyopencl as cl
        devices = [device for platform in cl.get_platforms()
                   for device in platform.get_devices(device_type=cl.device_type.GPU)]
        if not devices:
            raise RuntimeError("No OpenCL GPU device is installed")
        self.cl, self.np = cl, np
        self.device = devices[0]
        self.context = cl.Context(devices=[self.device])
        self.queue = cl.CommandQueue(self.context)
        self.program = cl.Program(self.context, KERNEL).build()
        self.name = self.device.name.strip()

    def compute(self, coordinates: list[tuple[float, float, float]],
                scale: tuple[float, float, float],
                offset: tuple[float, float, float], angle: float) -> list[float]:
        cl, np = self.cl, self.np
        source = np.asarray(coordinates, dtype=np.float32).reshape(-1, 3)
        output = np.empty_like(source)
        access = cl.mem_flags
        src = cl.Buffer(self.context, access.READ_ONLY | access.COPY_HOST_PTR, hostbuf=source)
        dst = cl.Buffer(self.context, access.WRITE_ONLY, output.nbytes)
        self.program.transform(self.queue, (len(coordinates),), None, src, dst,
            *[np.float32(v) for v in (*scale, *offset, angle)])
        cl.enqueue_copy(self.queue, output, dst).wait()
        return output.reshape(-1).tolist()

def sphere_geometry(segments=24, rings=12):
    vertices, indices = [], []
    for row in range(rings + 1):
        theta = math.pi * row / rings
        for column in range(segments + 1):
            phi = 2 * math.pi * column / segments
            vertices.append((math.sin(theta) * math.cos(phi),
                             math.cos(theta), math.sin(theta) * math.sin(phi)))
    for row in range(rings):
        for column in range(segments):
            a = row * (segments + 1) + column
            b = a + segments + 1
            indices.extend((a, b, a + 1, a + 1, b, b + 1))
    return vertices, indices

def torus_geometry(major=28, minor=14):
    vertices, indices = [], []
    for i in range(major + 1):
        a = i * 2 * math.pi / major
        for j in range(minor + 1):
            b = j * 2 * math.pi / minor
            r = 1.0 + .33 * math.cos(b)
            vertices.append((r * math.cos(a), .33 * math.sin(b), r * math.sin(a)))
    for i in range(major):
        for j in range(minor):
            a = i * (minor + 1) + j
            b = a + minor + 1
            indices.extend((a, b, a + 1, a + 1, b, b + 1))
    return vertices, indices

def box_geometry():
    faces = [
        ((0, 0, 1), [(-1,-1,1),(1,-1,1),(1,1,1),(-1,1,1)]),
        ((0, 0,-1), [(1,-1,-1),(-1,-1,-1),(-1,1,-1),(1,1,-1)]),
        ((1, 0, 0), [(1,-1,1),(1,-1,-1),(1,1,-1),(1,1,1)]),
        ((-1,0, 0), [(-1,-1,-1),(-1,-1,1),(-1,1,1),(-1,1,-1)]),
        ((0, 1, 0), [(-1,1,1),(1,1,1),(1,1,-1),(-1,1,-1)]),
        ((0,-1, 0), [(-1,-1,-1),(1,-1,-1),(1,-1,1),(-1,-1,1)]),
    ]
    verts, indices = [], []
    for _, corners in faces:
        n = len(verts)
        verts.extend(corners)
        indices.extend((n,n+1,n+2,n,n+2,n+3))
    return verts, indices

def cylinder_geometry(segments=24):
    verts, indices = [], []
    for y in (-1.0,1.0):
        for i in range(segments):
            a = 2 * math.pi * i / segments
            verts.append((math.cos(a), y, math.sin(a)))
    for i in range(segments):
        j = (i+1)%segments
        indices.extend((i,j,segments+i,j,segments+j,segments+i))
    for i in range(1,segments-1):
        indices.extend((0,i+1,i))
        indices.extend((segments,segments+i,segments+i+1))
    return verts, indices

def scene_plan(prompt: str, mode: str):
    # Safe, deterministic geometry selection only. Never execute prompt as code.
    lower = prompt.casefold()
    if mode == "architecture":
        return [
            ("Foundation", box_geometry(), (2.5,.12,2.0), (0,0,0), (0.40,0.48,0.57)),
            ("Main building", box_geometry(), (1.8,1.45,1.35), (0,1.55,0), (0.69,0.76,0.87)),
            ("Roof", box_geometry(), (2.0,.16,1.5), (0,3.13,0), (0.20,0.34,0.54)),
            ("Entrance", box_geometry(), (.40,.79,.16), (0,.78,1.45), (0.30,0.34,0.38)),
        ]
    if any(word in lower for word in ("ring", "torus", "donut")):
        geometry, name = torus_geometry(), "Ring"
    elif any(word in lower for word in ("box", "cube", "crate", "robot", "car", "vehicle")):
        geometry, name = box_geometry(), "Object"
    elif any(word in lower for word in ("tower", "tube", "cylinder", "column")):
        geometry, name = cylinder_geometry(), "Cylinder"
    else:
        geometry, name = sphere_geometry(), "Object"
    return [(name, geometry, (1,1,1), (0,0,0), (.37,.59,.94))]

def make_glb(prompt: str, mode: str, gpu: GPUBackend):
    """Produce actual GLB 2.0 triangle mesh, with every vertex transform done on GPU."""
    data = bytearray()
    views, accessors, meshes, nodes, materials = [], [], [], [], []
    total_vertices, total_triangles = 0, 0
    def buffer(data_bytes: bytes, target: int):
        while len(data)%4: data.append(0)
        offset = len(data)
        data.extend(data_bytes)
        views.append({"buffer":0,"byteOffset":offset,"byteLength":len(data_bytes),"target":target})
        return len(views)-1
    for name, (verts, indices), scale, offset, rgb in scene_plan(prompt, mode):
        xyz = gpu.compute(verts, scale, offset, 0)
        if not xyz or not all(math.isfinite(v) for v in xyz):
            raise RuntimeError("GPU returned invalid mesh coordinates")
        pos_view = buffer(struct.pack("<" + "f"*len(xyz), *xyz),34962)
        xyz_triples = list(zip(xyz[::3],xyz[1::3],xyz[2::3]))
        position_accessor = len(accessors)
        accessors.append({
            "bufferView":pos_view,"componentType":5126,"count":len(verts),"type":"VEC3",
            "min":[min(p[axis] for p in xyz_triples) for axis in range(3)],
            "max":[max(p[axis] for p in xyz_triples) for axis in range(3)],
        })
        index_view = buffer(struct.pack("<" + "I"*len(indices), *indices),34963)
        index_accessor = len(accessors)
        accessors.append({"bufferView":index_view,"componentType":5125,"count":len(indices),"type":"SCALAR"})
        material_index = len(materials)
        materials.append({"name": name, "pbrMetallicRoughness":{
            "baseColorFactor":[*rgb,1],"metallicFactor":0.15,"roughnessFactor":0.6},
            "doubleSided":True})
        meshes.append({"name":name,"primitives":[{
            "attributes":{"POSITION":position_accessor},"indices":index_accessor,"material":material_index,
            "mode":4,
        }]})
        nodes.append({"name":name,"mesh":len(meshes)-1})
        total_vertices += len(verts)
        total_triangles += len(indices)//3
    while len(data)%4: data.append(0)
    document = {
        "asset":{"version":"2.0","generator":"OpenMindAI Local OpenCL Studio"},
        "scene":0,"scenes":[{"nodes":list(range(len(nodes)))}],
        "nodes":nodes,"meshes":meshes,"materials":materials,
        "buffers":[{"byteLength":len(data)}],"bufferViews":views,"accessors":accessors,
    }
    json_chunk = json.dumps(document,separators=(",",":"),allow_nan=False).encode()
    json_chunk += b" " * ((-len(json_chunk))%4)
    body = struct.pack("<I4s",len(json_chunk),b"JSON")+json_chunk
    body += struct.pack("<I4s",len(data),b"BIN\x00")+data
    glb = struct.pack("<4sII",b"glTF",2,12+len(body))+body
    return glb, total_vertices, total_triangles

def process_job(job_id):
    with LOCK:
        job = JOBS[job_id]
        if job["status"] == "cancel_requested":
            job["status"] = "cancelled"
            return
        job["status"] = "running"
        prompt, mode = job.pop("_prompt"), job.pop("_mode")
    try:
        if GPU is None: raise RuntimeError(GPU_ERROR or "GPU is unavailable")
        glb, vertices, triangles = make_glb(prompt,mode,GPU)
        OUT_DIR.mkdir(mode=0o700,parents=True,exist_ok=True)
        filename = str(OUT_DIR / (job_id+".glb"))
        Path(filename).write_bytes(glb)
        with LOCK:
            if job["status"] == "cancel_requested":
                Path(filename).unlink(missing_ok=True)
                job["status"] = "cancelled"
            else:
                job["status"] = "completed"
                job["output"] = {"filename":filename,"vertices":vertices,"triangles":triangles,"format":"glb"}
    except Exception as exc:
        with LOCK:
            job["status"] = "failed"
            job["error"] = str(exc)[:200]

class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_): pass
    def send_json(self, code, data):
        raw = json.dumps(data).encode()
        self.send_response(code)
        self.send_header("Content-Type","application/json")
        self.send_header("Cache-Control","no-store")
        self.send_header("X-Content-Type-Options","nosniff")
        self.send_header("Content-Length",str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)
    def authorized(self):
        if self.headers.get("Origin") or self.headers.get("X-OpenMindAI-Desktop") != "1":
            self.send_json(403,{"message":"Only the native desktop bridge may call this worker"})
            return False
        return True
    def do_GET(self):
        if not self.authorized(): return
        if self.path == "/health":
            self.send_json(200,{"gpu_active":GPU is not None,"gpu":GPU.name if GPU else None,
                                "engine":"OpenCL procedural mesh","error":GPU_ERROR})
            return
        parts = self.path.strip("/").split("/")
        if len(parts) in (2,3) and parts[0] == "jobs":
            try: job_id = str(uuid.UUID(parts[1]))
            except ValueError: return self.send_json(404,{"message":"Not found"})
            with LOCK: job = JOBS.get(job_id)
            if not job: return self.send_json(404,{"message":"Job not found"})
            if len(parts)==2: return self.send_json(200,job)
            if parts[2]=="mesh" and job["status"]=="completed":
                path = OUT_DIR/(job_id+".glb")
                if not path.exists(): return self.send_json(410,{"message":"Mesh file missing"})
                return self.send_json(200,{"filename":job_id+".glb",
                    "data_base64":base64.b64encode(path.read_bytes()).decode()})
        return self.send_json(404,{"message":"Not found"})
    def do_POST(self):
        if not self.authorized(): return
        try: n = int(self.headers.get("Content-Length","0"))
        except ValueError: return self.send_json(400,{"message":"Invalid content length"})
        if n < 1 or n > 64000: return self.send_json(413,{"message":"Invalid request size"})
        try: data = json.loads(self.rfile.read(n))
        except (ValueError,UnicodeError): return self.send_json(400,{"message":"Invalid JSON"})
        if self.path == "/jobs":
            if GPU is None: return self.send_json(503,{"message":GPU_ERROR or "GPU is unavailable"})
            prompt = data.get("prompt","")
            mode = data.get("mode")
            key = data.get("request_key")
            if not isinstance(prompt,str) or not 5<=len(prompt)<=8000 or mode not in ("object","architecture"):
                return self.send_json(422,{"message":"Invalid prompt or scene mode"})
            if not isinstance(key,str) or not 1<=len(key)<=80 or not all(c.isalnum() or c in "_:-" for c in key):
                return self.send_json(422,{"message":"Invalid idempotency key"})
            with LOCK:
                if key in KEYS: return self.send_json(202,JOBS[KEYS[key]])
                if len(JOBS)>=256: return self.send_json(429,{"message":"Worker queue is full"})
                job_id = str(uuid.uuid4())
                job = {"id":job_id,"status":"queued","error":None,"output":None,"_prompt":prompt,"_mode":mode}
                JOBS[job_id] = job
                KEYS[key] = job_id
            threading.Thread(target=process_job,args=(job_id,),daemon=True).start()
            return self.send_json(202,{k:v for k,v in job.items() if not k.startswith("_")})
        parts=self.path.strip("/").split("/")
        if len(parts)==3 and parts[0]=="jobs" and parts[2]=="cancel":
            try: job_id = str(uuid.UUID(parts[1]))
            except ValueError: return self.send_json(404,{"message":"Not found"})
            with LOCK:
                job = JOBS.get(job_id)
                if not job: return self.send_json(404,{"message":"Not found"})
                if job["status"] in ("queued","running"): job["status"]="cancel_requested"
                result = {k:v for k,v in job.items() if not k.startswith("_")}
            return self.send_json(200,result)
        return self.send_json(404,{"message":"Not found"})

if __name__ == "__main__":
    try:
        GPU = GPUBackend()
        print("OpenMindAI Local 3D Studio GPU:",GPU.name,flush=True)
    except Exception as error:
        GPU_ERROR = str(error)
        print("GPU unavailable:",GPU_ERROR,flush=True)
    print("Listening on http://127.0.0.1:"+str(PORT),flush=True)
    ThreadingHTTPServer((HOST,PORT),Handler).serve_forever()
