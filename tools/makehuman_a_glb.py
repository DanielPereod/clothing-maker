#!/usr/bin/env python3
"""Convierte la malla base de MakeHuman (base.obj, CC0) en el maniquí de la
aplicación: assets/maniqui.glb.

    curl -LO https://raw.githubusercontent.com/makehumancommunity/makehuman/master/makehuman/data/3dobjs/base.obj
    python3 tools/makehuman_a_glb.py base.obj assets/maniqui.glb

Se queda solo con el grupo «body» (sin pestañas, ojos ni geometría de
ayuda), pasa de decímetros a metros, apoya los pies en y = 0, centra la
pelvis en x = z = 0 y guarda las articulaciones como nodos «joint-…» para
calcular las zonas de colocación. Solo usa la biblioteca estándar.
"""

import json
import struct
import sys

JOINTS = [
    "neck", "head", "pelvis", "spine-1", "spine-2", "spine-3", "spine-4",
    "l-clavicle", "l-shoulder", "l-elbow", "l-hand", "l-upper-leg", "l-knee", "l-ankle",
    "r-clavicle", "r-shoulder", "r-elbow", "r-hand", "r-upper-leg", "r-knee", "r-ankle",
]
SCALE = 0.1  # MakeHuman trabaja en decímetros.


def main(src, dst):
    vertices, groups, group = [], {}, None
    with open(src) as f:
        for line in f:
            if line.startswith("v "):
                vertices.append([float(x) * SCALE for x in line.split()[1:4]])
            elif line.startswith("g "):
                group = line.split()[1]
            elif line.startswith("f "):
                face = [int(t.split("/")[0]) - 1 for t in line.split()[1:]]
                groups.setdefault(group, []).append(face)

    def centroid(name):
        ids = {i for face in groups[name] for i in face}
        return [sum(vertices[i][k] for i in ids) / len(ids) for k in range(3)]

    body = groups["body"]
    used = sorted({i for face in body for i in face})
    remap = {old: new for new, old in enumerate(used)}
    pelvis = centroid("joint-pelvis")
    floor = min(vertices[i][1] for i in used)
    offset = [-pelvis[0], -floor, -pelvis[2]]
    move = lambda p: [p[k] + offset[k] for k in range(3)]

    positions = [move(vertices[i]) for i in used]
    triangles = []
    for face in body:
        f = [remap[i] for i in face]
        for k in range(1, len(f) - 1):
            triangles.append([f[0], f[k], f[k + 1]])

    pos_bytes = b"".join(struct.pack("<3f", *p) for p in positions)
    idx_bytes = b"".join(struct.pack("<3I", *t) for t in triangles)
    lo = [min(p[k] for p in positions) for k in range(3)]
    hi = [max(p[k] for p in positions) for k in range(3)]
    joints = [{"name": f"joint-{j}", "translation": move(centroid(f"joint-{j}"))} for j in JOINTS]

    gltf = {
        "asset": {
            "version": "2.0",
            "generator": "tools/makehuman_a_glb.py",
            "copyright": "MakeHuman base mesh hm08, CC0 (Data Collection AB, Joel Palmius, Jonas Hauquier)",
        },
        "scene": 0,
        "scenes": [{"nodes": list(range(len(joints) + 1))}],
        "nodes": [{"name": "Maniquí", "mesh": 0}] + joints,
        "meshes": [{"name": "body", "primitives": [{"attributes": {"POSITION": 0}, "indices": 1}]}],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": len(positions), "type": "VEC3", "min": lo, "max": hi},
            {"bufferView": 1, "componentType": 5125, "count": len(triangles) * 3, "type": "SCALAR"},
        ],
        "bufferViews": [
            {"buffer": 0, "byteOffset": 0, "byteLength": len(pos_bytes), "target": 34962},
            {"buffer": 0, "byteOffset": len(pos_bytes), "byteLength": len(idx_bytes), "target": 34963},
        ],
        "buffers": [{"byteLength": len(pos_bytes) + len(idx_bytes)}],
    }
    js = json.dumps(gltf, separators=(",", ":"), ensure_ascii=False).encode()
    js += b" " * (-len(js) % 4)
    binary = pos_bytes + idx_bytes
    binary += b"\0" * (-len(binary) % 4)
    total = 12 + 8 + len(js) + 8 + len(binary)
    with open(dst, "wb") as f:
        f.write(struct.pack("<III", 0x46546C67, 2, total))
        f.write(struct.pack("<II", len(js), 0x4E4F534A) + js)
        f.write(struct.pack("<II", len(binary), 0x004E4942) + binary)
    print(f"{len(positions)} vértices, {len(triangles)} triángulos, altura {hi[1]:.3f} m")


if __name__ == "__main__":
    main(*sys.argv[1:3])
