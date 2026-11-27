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
import sys
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
DIAGNOSTICS = {}

KERNEL = """
__kernel void transform(__global const float *source, __global float *output,
    const float sx, const float sy, const float sz,
    const float tx, const float ty, const float tz,
    const float rx, const float ry, const float rz) {
    int i = get_global_id(0);
    float x = source[i * 3] * sx;
    float y = source[i * 3 + 1] * sy;
    float z = source[i * 3 + 2] * sz;
    float cx = cos(rx), sxr = sin(rx), cy = cos(ry), syr = sin(ry);
    float cz = cos(rz), szr = sin(rz);
    float y1 = y * cx - z * sxr, z1 = y * sxr + z * cx;
    float x2 = x * cy + z1 * syr, z2 = -x * syr + z1 * cy;
    output[i * 3] = x2 * cz - y1 * szr + tx;
    output[i * 3 + 1] = x2 * szr + y1 * cz + ty;
    output[i * 3 + 2] = z2 + tz;
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
                offset: tuple[float, float, float], rotation: tuple[float, float, float]) -> list[float]:
        cl, np = self.cl, self.np
        source = np.asarray(coordinates, dtype=np.float32).reshape(-1, 3)
        output = np.empty_like(source)
        access = cl.mem_flags
        src = cl.Buffer(self.context, access.READ_ONLY | access.COPY_HOST_PTR, hostbuf=source)
        dst = cl.Buffer(self.context, access.WRITE_ONLY, output.nbytes)
        self.program.transform(self.queue, (len(coordinates),), None, src, dst,
            *[np.float32(v) for v in (*scale, *offset, *rotation)])
        cl.enqueue_copy(self.queue, output, dst).wait()
        return output.reshape(-1).tolist()


def probe_gpu():
    """Initialize OpenCL only after independently reporting its prerequisites."""
    details = {"python": sys.executable, "python_version": sys.version.split()[0],
               "python_bits": struct.calcsize("P") * 8, "numpy": None,
               "pyopencl": None, "opencl_platforms": [], "opencl_devices": [],
               "cpu_ready": True, "gpu_ready": False, "error": None}
    numpy_module, cl_module = None, None
    try:
        import numpy as numpy_module
        details["numpy"] = numpy_module.__version__
    except Exception as exc:
        details["numpy_error"] = f"{type(exc).__name__}: {exc}"
    try:
        import pyopencl as cl_module
        details["pyopencl"] = getattr(cl_module, "VERSION_TEXT", "available")
        platforms = cl_module.get_platforms()
        details["opencl_platforms"] = [p.name.strip() for p in platforms]
        devices = [d for p in platforms for d in p.get_devices(device_type=cl_module.device_type.GPU)]
        details["opencl_devices"] = [d.name.strip() for d in devices]
    except Exception as exc:
        details["opencl_error"] = f"{type(exc).__name__}: {exc}"
    if numpy_module is None:
        details["error"] = f"NumPy unavailable: {details['numpy_error']}"
    elif cl_module is None:
        details["error"] = f"PyOpenCL unavailable: {details.get('opencl_error', 'import failed')}"
    elif not details["opencl_devices"]:
        details["error"] = "PyOpenCL is installed, but no OpenCL GPU device is available"
    else:
        try:
            backend = GPUBackend()
            details["gpu_ready"] = True
            details["device"] = backend.name
            return backend, details
        except Exception as exc:
            details["error"] = f"OpenCL device or kernel initialization failed: {type(exc).__name__}: {exc}"
    return None, details

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

def cone_geometry(segments=24):
    verts, indices = [(0.0, 1.0, 0.0)], []
    for i in range(segments):
        a = 2 * math.pi * i / segments
        verts.append((math.cos(a), -1.0, math.sin(a)))
    verts.append((0.0, -1.0, 0.0))
    bottom = len(verts) - 1
    for i in range(segments):
        j = (i + 1) % segments
        indices.extend((0, i + 1, j + 1, bottom, j + 1, i + 1))
    return verts, indices

def scene_plan(prompt: str, mode: str, scene_spec=None):
    if mode == "architecture" and isinstance(scene_spec, dict) and scene_spec.get("schemaVersion") == 2:
        lower = prompt.casefold()
        furnishing = "unfurnished" if "unfurnished" in lower or "no furniture" in lower else "partially furnished" if "partially furnished" in lower or "partial furnishing" in lower else "fully furnished" if "furnish" in lower or "furniture" in lower else "unfurnished"
        return architecture_plan(scene_spec, furnishing)
    if isinstance(scene_spec, dict) and isinstance(scene_spec.get("objects"), list):
        primitives = {"box": box_geometry, "cylinder": cylinder_geometry, "sphere": sphere_geometry,
                      "cone": cone_geometry, "wheel": torus_geometry}
        plan = []
        for item in scene_spec["objects"]:
            if not isinstance(item, dict) or item.get("primitive") not in primitives:
                raise RuntimeError("Native scene plan contains an unsupported primitive")
            dimensions = item.get("dimensions")
            position = item.get("position")
            color = item.get("color")
            rotation = item.get("rotation")
            if any(not isinstance(vector, list) or len(vector) != 3 for vector in (dimensions, position, color, rotation)):
                raise RuntimeError("Native scene plan contains a malformed transform")
            if not all(isinstance(value, (int, float)) and math.isfinite(value) for vector in (dimensions, position, color, rotation) for value in vector):
                raise RuntimeError("Native scene plan contains non-finite numbers")
            plan.append({"name": item["name"], "geometry": primitives[item["primitive"]](),
                "scale": tuple(value / 2 for value in dimensions), "offset": tuple(position),
                "rgb": tuple(color), "metallic": float(item["metallic"]),
                "roughness": float(item["roughness"]), "rotation": tuple(rotation),
                "parent": item.get("parent")})
        if not 1 <= len(plan) <= 16:
            raise RuntimeError("Native scene plan must contain 1 to 16 objects")
        return plan
    # Safe, deterministic geometry selection only. Never execute prompt as code.
    lower = prompt.casefold()
    if mode == "architecture":
        return [{"name":name,"geometry":geometry,"scale":scale,"offset":offset,"rgb":rgb,
                 "metallic":0.15,"roughness":0.6,"rotation":(0,0,0),"parent":None}
                for name, geometry, scale, offset, rgb in [
            ("Foundation", box_geometry(), (2.5,.12,2.0), (0,0,0), (0.40,0.48,0.57)),
            ("Main building", box_geometry(), (1.8,1.45,1.35), (0,1.55,0), (0.69,0.76,0.87)),
            ("Roof", box_geometry(), (2.0,.16,1.5), (0,3.13,0), (0.20,0.34,0.54)),
            ("Entrance", box_geometry(), (.40,.79,.16), (0,.78,1.45), (0.30,0.34,0.38))]]
    if any(word in lower for word in ("ring", "torus", "donut")):
        geometry, name = torus_geometry(), "Ring"
    elif any(word in lower for word in ("box", "cube", "crate", "robot", "car", "vehicle")):
        geometry, name = box_geometry(), "Object"
    elif any(word in lower for word in ("tower", "tube", "cylinder", "column")):
        geometry, name = cylinder_geometry(), "Cylinder"
    else:
        geometry, name = sphere_geometry(), "Object"
    return [{"name":name,"geometry":geometry,"scale":(1,1,1),"offset":(0,0,0),"rgb":(.37,.59,.94),
             "metallic":0.15,"roughness":0.6,"rotation":(0,0,0),"parent":None}]

def architecture_plan(spec, furnishing_mode="unfurnished"):
    """Turn the AI-authored room layout into individually editable house parts.

    Room locations and sizes come from the validated model response. This
    adapter supplies construction geometry (including segmented wall openings)
    without executing model code or substituting a prebuilt house asset.
    Coordinates use metres, with Y vertical and the front facade at +Z.
    """
    if spec.get("footprint") != [12, 10] or not 2.8 <= float(spec.get("storyHeight", 0)) <= 3.8:
        raise RuntimeError("Architectural plan must use a 12 by 10 metre footprint and a plausible story height")
    rooms = spec.get("rooms")
    if not isinstance(rooms, list) or len(rooms) != 9:
        raise RuntimeError("Architectural plan must contain all nine planned rooms")
    if furnishing_mode not in ("unfurnished", "partially furnished", "fully furnished"):
        raise RuntimeError("Furnishing mode must be unfurnished, partially furnished or fully furnished")
    required = {"ground": ("living", "dining", "kitchen", "bathroom"),
                "upper": ("bedroom", "bathroom")}
    by_floor = {"ground": [], "upper": []}
    names = set()
    for room in rooms:
        if not isinstance(room, dict):
            raise RuntimeError("Architectural room entries must be objects")
        name, floor = room.get("name"), room.get("floor")
        if not isinstance(name, str) or not name.strip() or name in names or floor not in by_floor:
            raise RuntimeError("Architectural room has an invalid or duplicate name/floor")
        names.add(name)
        x, z, width, depth = (float(room.get(key, float("nan"))) for key in ("x", "z", "width", "depth"))
        if not all(math.isfinite(v) for v in (x, z, width, depth)) or not (0 < width <= 12 and 0 < depth <= 10):
            raise RuntimeError("Architectural room has invalid real-world bounds")
        if x-width/2 < -6.01 or x+width/2 > 6.01 or z-depth/2 < -5.01 or z+depth/2 > 5.01:
            raise RuntimeError("Architectural room extends beyond the 12 by 10 metre envelope")
        room = {"name":name, "floor":floor, "x":x, "z":z, "width":width, "depth":depth}
        by_floor[floor].append(room)
    ground_names = " ".join(r["name"].casefold() for r in by_floor["ground"])
    upper_names = " ".join(r["name"].casefold() for r in by_floor["upper"])
    if any(word not in ground_names for word in required["ground"]):
        raise RuntimeError("AI plan omitted a required ground-floor room")
    if sum("bedroom" in r["name"].casefold() for r in by_floor["upper"]) < 3 or not all(word in upper_names for word in required["upper"]) or "landing" not in upper_names:
        raise RuntimeError("AI plan must contain three upper bedrooms, a bathroom and a stair landing")

    plan = []
    used_names = set(names)
    colors = {"concrete": (.56,.57,.55), "paint": (.83,.81,.75), "wood": (.39,.21,.11),
              "glass": (.38,.72,.84), "metal": (.18,.20,.22), "floor": (.69,.61,.48),
              "roof": (.25,.29,.32)}
    def box(name, dims, pos, floor, color="paint", metallic=0.0, roughness=.72):
        original = name
        suffix = 2
        while name in used_names:
            name = f"{original} {suffix}"
            suffix += 1
        used_names.add(name)
        plan.append({"name":name, "geometry":box_geometry(), "scale":tuple(v/2 for v in dims),
            "offset":tuple(pos), "rgb":colors[color], "metallic":metallic,
            "roughness":roughness, "rotation":(0,0,0), "parent":floor})

    story_height = float(spec["storyHeight"])
    wall_h = story_height - .35
    wall_t = .24
    box("Foundation", (12.4,.36,10.4), (0,-.18,0), "Residential House", "concrete")
    box("Ground floor slab", (12,.24,10), (0,.12,0), "Ground Floor", "concrete")
    box("Upper floor slab", (12,.26,10), (0,story_height,0), "Upper Floor", "concrete")
    box("Flat roof slab", (12.2,.28,10.2), (0,2*story_height+.05,0), "Roof", "roof")

    def wall_run(label, axis, fixed, low, high, base, openings, parent):
        # openings: (centre, width, sill, height, kind); partition openings use kind=door.
        along_cuts = {low, high}
        vertical_cuts = {base, base+wall_h}
        for center, width, sill, height, _kind in openings:
            along_cuts.update((max(low, center-width/2), min(high, center+width/2)))
            vertical_cuts.update((max(base,base+sill), min(base+wall_h,base+sill+height)))
        along = sorted(v for v in along_cuts if low-1e-6 <= v <= high+1e-6)
        vertical = sorted(v for v in vertical_cuts if base-1e-6 <= v <= base+wall_h+1e-6)
        index = 0
        for a, b in zip(along, along[1:]):
            if b-a < .015: continue
            mid = (a+b)/2
            for y0, y1 in zip(vertical, vertical[1:]):
                if y1-y0 < .015: continue
                ym = (y0+y1)/2
                if any(abs(mid-center) < width/2-.005 and base+sill-.005 < ym < base+sill+height+.005
                       for center,width,sill,height,_kind in openings):
                    continue
                dims = (b-a, y1-y0, wall_t) if axis == "x" else (wall_t, y1-y0, b-a)
                pos = ((a+b)/2, (y0+y1)/2, fixed) if axis == "x" else (fixed, (y0+y1)/2, (a+b)/2)
                box(f"{label} wall section {index+1}", dims, pos, parent, "concrete")
                index += 1
        for center,width,sill,height,kind in openings:
            # Frame pieces are separate parts; the wall mesh is absent in the opening.
            frame = .07
            floor_center = base+sill
            top = floor_center+height
            if axis == "x":
                box(f"{label} {kind} frame left", (frame,height,wall_t*1.3), (center-width/2+frame/2,(floor_center+top)/2,fixed), parent, "wood" if kind=="door" else "metal", .45 if kind!="door" else 0)
                box(f"{label} {kind} frame right", (frame,height,wall_t*1.3), (center+width/2-frame/2,(floor_center+top)/2,fixed), parent, "wood" if kind=="door" else "metal", .45 if kind!="door" else 0)
                box(f"{label} {kind} frame head", (width,frame,wall_t*1.3), (center,top-frame/2,fixed), parent, "wood" if kind=="door" else "metal", .45 if kind!="door" else 0)
                if kind == "window":
                    box(f"{label} window frame sill", (width,frame,wall_t*1.3), (center,floor_center+frame/2,fixed), parent, "metal", .45,.35)
                    box(f"{label} glass pane", (width-2*frame,height-2*frame,.035), (center,(floor_center+top)/2,fixed), parent, "glass", .05,.16)
                else: box(f"{label} wooden door leaf", (width-2*frame,height-frame,.045), (center,(floor_center+top-frame)/2,fixed), parent, "wood")
            else:
                box(f"{label} {kind} frame left", (wall_t*1.3,height,frame), (fixed,(floor_center+top)/2,center-width/2+frame/2), parent, "wood" if kind=="door" else "metal", .45 if kind!="door" else 0)
                box(f"{label} {kind} frame right", (wall_t*1.3,height,frame), (fixed,(floor_center+top)/2,center+width/2-frame/2), parent, "wood" if kind=="door" else "metal", .45 if kind!="door" else 0)
                box(f"{label} {kind} frame head", (wall_t*1.3,frame,width), (fixed,top-frame/2,center), parent, "wood" if kind=="door" else "metal", .45 if kind!="door" else 0)
                if kind == "window":
                    box(f"{label} window frame sill", (wall_t*1.3,frame,width), (fixed,floor_center+frame/2,center), parent, "metal", .45,.35)
                    box(f"{label} glass pane", (.035,height-2*frame,width-2*frame), (fixed,(floor_center+top)/2,center), parent, "glass", .05,.16)
                else: box(f"{label} wooden door leaf", (.045,height-frame,width-2*frame), (fixed,(floor_center+top-frame)/2,center), parent, "wood")

    for floor, base, group in (("ground",.24,"Ground Floor"),("upper",story_height+.26,"Upper Floor")):
        # Each room footprint comes from Qwen. Thin floor panels make room
        # selection tangible without hiding the structural slabs.
        room_floor_y = .24 if floor == "ground" else story_height + .18
        for room in by_floor[floor]:
            box(f"{room['name']} floor", (room["width"]-.06,.035,room["depth"]-.06), (room["x"],room_floor_y,room["z"]), group, "floor")
        front = [(0,1.0,0,2.1,"door")] if floor == "ground" else []
        front.extend([(-4.0,1.45,.9,1.25,"window"),(4.0,1.45,.9,1.25,"window")])
        back = [(-3.7,1.4,.9,1.25,"window"),(3.7,1.4,.9,1.25,"window")]
        side = [(0,1.3,.9,1.2,"window")]
        wall_run(f"{floor} front", "x", 5, -6, 6, base, front, group)
        wall_run(f"{floor} rear", "x", -5, -6, 6, base, back, group)
        wall_run(f"{floor} left", "z", -6, -5, 5, base, side, group)
        wall_run(f"{floor} right", "z", 6, -5, 5, base, side, group)

        partitions = {}
        for room in by_floor[floor]:
            x1,x2 = room["x"]-room["width"]/2, room["x"]+room["width"]/2
            z1,z2 = room["z"]-room["depth"]/2, room["z"]+room["depth"]/2
            for axis,fixed,lo,hi in (("z",x1,z1,z2),("z",x2,z1,z2),("x",z1,x1,x2),("x",z2,x1,x2)):
                if (axis=="z" and abs(abs(fixed)-6)<.08) or (axis=="x" and abs(abs(fixed)-5)<.08): continue
                key=(axis,round(fixed,2))
                partitions.setdefault(key,[]).append((lo,hi))
        partition_index = 0
        for (axis,fixed), intervals in partitions.items():
            intervals.sort()
            merged=[]
            for lo,hi in intervals:
                if merged and lo <= merged[-1][1]+.08:
                    merged[-1]=(merged[-1][0],max(merged[-1][1],hi))
                else:
                    merged.append((lo,hi))
            for lo,hi in merged:
                # Place one separate door in each AI room span while assembling
                # each shared partition exactly once to avoid z-fighting seams.
                centers=sorted({round((a+b)/2,3) for a,b in intervals if a >= lo-.08 and b <= hi+.08})
                openings=[]
                for center in centers:
                    span=next((b-a for a,b in intervals if abs((a+b)/2-center)<.01),hi-lo)
                    width=min(.9,max(.5,span-.12))
                    openings.append((center,width,0,2.05,"door"))
                partition_index+=1
                wall_run(f"{floor} partition {partition_index}",axis,fixed,lo,hi,base,openings,group)

    # Connecting dog-leg stair, 12 individually selectable concrete treads.
    landing = next(room for room in by_floor["upper"] if "landing" in room["name"].casefold())
    stair_x = landing["x"]
    stair_top_z = landing["z"] - landing["depth"] / 2 + .15
    for step in range(12):
        box(f"Stair tread {step+1:02d}",(1.05,.22,.30),(stair_x,.30+step*(story_height-.30)/11,stair_top_z-(11-step)*.30),"Staircase","concrete")
    box("Stair landing",(landing["width"],.18,landing["depth"]),(stair_x,story_height+.07,landing["z"]),"Staircase","concrete")
    # Front upper balcony, connected at the upper floor edge, with separate rails.
    balcony_floor = story_height + .20
    railing_y = story_height + .80
    box("Balcony slab",(4.2,.18,1.55),(0,balcony_floor,5.72),"Upper Floor","concrete")
    box("Balcony front railing",(4.2,.08, .08),(0,railing_y,6.47),"Upper Floor","metal",.75,.32)
    for side_x in (-2.06,2.06): box("Balcony side railing",(.08,.08,1.5),(side_x,railing_y,5.72),"Upper Floor","metal",.75,.32)
    for i in range(17): box(f"Balcony baluster {i+1:02d}",(.035,.58,.035),(-2+i*.25,story_height+.48,6.47),"Upper Floor","metal",.75,.32)
    # Roof parapet, entrance steps and supporting balcony columns.
    for x in (-5.9,5.9): box("Roof parapet side",(.2,.45,10),(x,2*story_height+.39,0),"Roof","concrete")
    for z in (-4.9,4.9): box("Roof parapet front rear",(12,.45,.2),(0,2*story_height+.39,z),"Roof","concrete")
    for step in range(3): box(f"Entrance step {step+1}",(1.5,.14,.36),(0,.07-step*.14,5.18+step*.34),"Ground Floor","concrete")
    for x in (-1.8,1.8): box("Balcony support column",(.24,story_height-.15,.24),(x,story_height/2,6.35),"Ground Floor","concrete")

    if furnishing_mode != "unfurnished":
        def furniture(room, name, dims, x, z, color="wood", metallic=0.0, y_offset=0.0):
            width, height, depth = dims
            if width > room["width"]-.16 or depth > room["depth"]-.16:
                return False
            x = min(room["x"]+room["width"]/2-width/2-.08, max(room["x"]-room["width"]/2+width/2+.08, x))
            z = min(room["z"]+room["depth"]/2-depth/2-.08, max(room["z"]-room["depth"]/2+depth/2+.08, z))
            base_y = .24 if room["floor"] == "ground" else story_height + .18
            box(f"{room['name']} · {name}", dims, (x,base_y+y_offset+height/2,z), room["name"], color, metallic)
            return True

        for room in rooms:
            label = room["name"].casefold()
            width, depth = room["width"], room["depth"]
            x1, x2 = room["x"]-width/2, room["x"]+width/2
            z1, z2 = room["z"]-depth/2, room["z"]+depth/2
            cx, cz = room["x"], room["z"]
            primary = furnishing_mode == "fully furnished" or any(kind in label for kind in ("living", "kitchen", "bedroom"))
            if "living" in label and primary:
                furniture(room,"Sofa",(min(2.3,width-.3),.78,.88),cx,z2-.58)
                furniture(room,"Coffee table",(min(1.0,width*.30),.42,.56),cx,cz-.20)
                furniture(room,"TV console",(min(1.7,width-.4),.55,.32),cx,z1+.34)
                furniture(room,"Floor lamp",(.22,1.55,.22),x2-.22,z2-.22,"metal",.55)
                if furnishing_mode == "fully furnished": furniture(room,"Book shelf",(.34,1.65,.85),x1+.28,cz)
            elif "kitchen" in label and primary:
                cabinet_width=min(2.2,width-.25)
                furniture(room,"Base cabinets",(cabinet_width,.88,.58),cx,z1+.38)
                furniture(room,"Countertop",(cabinet_width+.04,.08,.62),cx,z1+.38,"concrete")
                furniture(room,"Wall cabinets",(cabinet_width,.72,.34),cx,z1+.48,"paint",y_offset=1.24)
                furniture(room,"Sink",(.52,.13,.38),cx-cabinet_width*.28,z1+.38,"metal",.6)
                furniture(room,"Cooktop",(.62,.10,.48),cx+cabinet_width*.27,z1+.38,"metal",.7)
                furniture(room,"Refrigerator",(.72,1.85,.72),x2-.46,z2-.46,"metal",.25)
            elif "dining" in label and furnishing_mode == "fully furnished":
                table_width=min(1.7,width-.4)
                table_depth=min(.95,depth-.4)
                furniture(room,"Dining table",(table_width,.76,table_depth),cx,cz)
                for index, (dx,dz) in enumerate(((-(table_width/2+.31),0),(table_width/2+.31,0),(0,-(table_depth/2+.30)),(0,table_depth/2+.30)),1):
                    furniture(room,f"Dining chair {index}",(.42,.82,.44),cx+dx,cz+dz,"paint")
            elif "bedroom" in label and primary:
                bed_width=min(1.45,width-.3)
                bed_depth=min(2.0,depth-.35)
                furniture(room,"Bed frame",(bed_width,.34,bed_depth),cx,cz-.22,"wood")
                furniture(room,"Mattress",(bed_width-.06,.22,bed_depth-.06),cx,cz-.22,"paint")
                furniture(room,"Wardrobe",(.62,1.9,min(1.1,depth-.25)),x2-.40,cz,"wood")
                if furnishing_mode == "fully furnished":
                    furniture(room,"Bedside table",(.42,.52,.42),x1+.30,cz-.4)
                    if width >= 3.2: furniture(room,"Study desk",(min(1.15,width*.36),.74,.55),cx,z2-.36)
            elif "bathroom" in label and furnishing_mode == "fully furnished":
                furniture(room,"Vanity and sink",(.62,.84,.46),x1+.39,z1+.36,"paint")
                furniture(room,"Toilet",(.42,.44,.66),x2-.34,z1+.44,"paint")
                furniture(room,"Shower enclosure",(.82,1.95,.82),x2-.50,z2-.50,"glass")
            elif "landing" in label and furnishing_mode == "fully furnished":
                furniture(room,"Landing light",(.25,.20,.25),cx,cz,"metal",.7)
    # Root and floor hierarchy is represented with empty glTF nodes.
    for entry in plan: entry["parent"] = entry["parent"] or "Residential House"
    return plan

def cpu_transform(coordinates, scale, offset, rotation):
    rx, ry, rz = rotation
    cx, sx = math.cos(rx), math.sin(rx)
    cy, sy = math.cos(ry), math.sin(ry)
    cz, sz = math.cos(rz), math.sin(rz)
    result = []
    for x, y, z in coordinates:
        x, y, z = x * scale[0], y * scale[1], z * scale[2]
        y1, z1 = y * cx - z * sx, y * sx + z * cx
        x2, z2 = x * cy + z1 * sy, -x * sy + z1 * cy
        result.extend((x2 * cz - y1 * sz + offset[0], x2 * sz + y1 * cz + offset[1], z2 + offset[2]))
    return result

def make_glb(prompt: str, mode: str, gpu: GPUBackend | None = None, scene_spec=None):
    """Produce editable GLB triangle meshes using OpenCL when ready, otherwise CPU."""
    data = bytearray()
    views, accessors, meshes, nodes, materials = [], [], [], [], []
    total_vertices, total_triangles = 0, 0
    def buffer(data_bytes: bytes, target: int):
        while len(data)%4: data.append(0)
        offset = len(data)
        data.extend(data_bytes)
        views.append({"buffer":0,"byteOffset":offset,"byteLength":len(data_bytes),"target":target})
        return len(views)-1
    plan = scene_plan(prompt, mode, scene_spec)
    for entry in plan:
        name, (verts, indices) = entry["name"], entry["geometry"]
        scale, offset, rgb = entry["scale"], entry["offset"], entry["rgb"]
        xyz = gpu.compute(verts, scale, offset, entry["rotation"]) if gpu else cpu_transform(verts, scale, offset, entry["rotation"])
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
        is_glass = "glass" in name.casefold() or "pane" in name.casefold()
        materials.append({"name": name, "pbrMetallicRoughness":{
            "baseColorFactor":[*rgb,.38 if is_glass else 1],"metallicFactor":entry["metallic"],"roughnessFactor":entry["roughness"]},
            "alphaMode":"BLEND" if is_glass else "OPAQUE", "doubleSided":True})
        meshes.append({"name":name,"primitives":[{
            "attributes":{"POSITION":position_accessor},"indices":index_accessor,"material":material_index,
            "mode":4,
        }]})
        total_vertices += len(verts)
        total_triangles += len(indices)//3
    while len(data)%4: data.append(0)
    # Construct the editable glTF hierarchy. Referenced parents without meshes
    # become empty group nodes (for the generated building/floors).
    parent_names = {entry.get("parent") for entry in plan if entry.get("parent")}
    group_names = sorted(parent_names - {entry["name"] for entry in plan})
    nodes = [{"name": name} for name in group_names]
    node_by_name = {name: index for index, name in enumerate(group_names)}
    mesh_nodes = []
    for index, entry in enumerate(plan):
        node_index = len(nodes)
        nodes.append({"name":entry["name"],"mesh":index})
        node_by_name[entry["name"]] = node_index
        mesh_nodes.append(node_index)
    roots = []
    for entry, node_index in zip(plan, mesh_nodes):
        parent = entry.get("parent")
        if parent is None:
            roots.append(node_index)
        else:
            parent_index = node_by_name.get(parent)
            if parent_index is None or parent_index == node_index:
                raise RuntimeError("Native scene plan has an invalid parent")
            nodes[parent_index].setdefault("children", []).append(node_index)
    room_group_parents = {}
    if mode == "architecture" and isinstance(scene_spec, dict):
        room_group_parents = {
            room["name"]: ("Ground Floor" if room.get("floor") == "ground" else "Upper Floor")
            for room in scene_spec.get("rooms", []) if isinstance(room, dict) and isinstance(room.get("name"), str)
        }
    for index, name in enumerate(group_names):
        if name == "Residential House" or (name not in parent_names and name not in room_group_parents):
            roots.append(index)
        elif name in node_by_name:
            parent = "Residential House" if name in {"Ground Floor","Upper Floor","Roof","Staircase"} else room_group_parents.get(name)
            if parent:
                nodes[node_by_name[parent]].setdefault("children", []).append(index)
    if not roots:
        raise RuntimeError("Native scene plan hierarchy has no root")
    document = {
        "asset":{"version":"2.0","generator":"OpenMindAI Local 3D Studio"},
        "scene":0,"scenes":[{"nodes":roots}],
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
        scene_spec = job.pop("_scene_spec", None)
    try:
        backend = GPU
        glb, vertices, triangles = make_glb(prompt,mode,backend,scene_spec)
        json_length = struct.unpack_from("<I", glb, 12)[0]
        glb_document = json.loads(glb[20:20+json_length])
        OUT_DIR.mkdir(mode=0o700,parents=True,exist_ok=True)
        filename = str(OUT_DIR / (job_id+".glb"))
        Path(filename).write_bytes(glb)
        with LOCK:
            if job["status"] == "cancel_requested":
                Path(filename).unlink(missing_ok=True)
                job["status"] = "cancelled"
            else:
                job["status"] = "completed"
                job["output"] = {"filename":filename,"vertices":vertices,"triangles":triangles,"objects":len(glb_document["meshes"]),"format":"glb",
                                  "geometry_backend":"opencl" if backend else "cpu"}
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
                                "worker_ready":True,"cpu_ready":True,
                                "geometry_backend":"opencl" if GPU else "cpu",
                                "engine":"procedural triangle mesh","diagnostics":DIAGNOSTICS,
                                "error":GPU_ERROR})
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
            prompt = data.get("prompt","")
            mode = data.get("mode")
            key = data.get("request_key")
            scene_spec = data.get("scene_spec")
            if not isinstance(prompt,str) or not 5<=len(prompt)<=8000 or mode not in ("object","architecture"):
                return self.send_json(422,{"message":"Invalid prompt or scene mode"})
            if not isinstance(key,str) or not 1<=len(key)<=80 or not all(c.isalnum() or c in "_:-" for c in key):
                return self.send_json(422,{"message":"Invalid idempotency key"})
            with LOCK:
                if key in KEYS: return self.send_json(202,JOBS[KEYS[key]])
                if len(JOBS)>=256: return self.send_json(429,{"message":"Worker queue is full"})
                job_id = str(uuid.uuid4())
                job = {"id":job_id,"status":"queued","error":None,"output":None,"_prompt":prompt,"_mode":mode,"_scene_spec":scene_spec}
                JOBS[job_id] = job
                KEYS[key] = job_id
                queued_response = {k:v for k,v in job.items() if not k.startswith("_")}
            threading.Thread(target=process_job,args=(job_id,),daemon=True).start()
            return self.send_json(202,queued_response)
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
    GPU, DIAGNOSTICS = probe_gpu()
    GPU_ERROR = DIAGNOSTICS.get("error")
    print("OpenMindAI Local 3D geometry backend:", GPU.name if GPU else "CPU", flush=True)
    if GPU_ERROR: print("Optional OpenCL diagnostics:", GPU_ERROR, flush=True)
    print("Listening on http://127.0.0.1:"+str(PORT),flush=True)
    ThreadingHTTPServer((HOST,PORT),Handler).serve_forever()
