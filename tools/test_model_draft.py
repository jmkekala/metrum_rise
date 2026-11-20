# SPDX-License-Identifier: GPL-3.0-or-later
"""The model draft command (`TOOLS-33`). The unit tests run a stand-in for Godot and need no
build. `GodotDraftTests` drafts a generated model end to end through Godot and the game library;
it runs only with METRUM_MODEL_DRAFT_GODOT=1, which the gate's Godot chain sets after the build.
Every pack it writes is a temporary fixture: no test points `--mods` at the player's packs."""

import hashlib
import json
import os
from pathlib import Path
import stat
import struct
import sys
import tempfile
import unittest
import zlib

import model_draft

PACK = "drafttest"
ASSET = "building.residential.drafted_house"
NAME = "test_house"
BOUNDS = ([-5.35, 0.0, -6.84], [5.35, 9.5625, 6.84])

FAKE_GODOT = """#!/usr/bin/env python3
import json, os, sys
request, report = sys.argv[sys.argv.index("--") + 1:]
seen = {"request": json.load(open(request)), "xdg": os.environ["XDG_DATA_HOME"],
        "copied": sorted(os.listdir(json.load(open(request))["mods"]))}
json.dump({"status": "drafted", "summary": "fake summary", "seen": seen}, open(report, "w"))
sys.exit(int(os.environ.get("FAKE_GODOT_EXIT", "0")))
"""


def png(width, height, rgb):
    """A minimal RGB PNG."""
    rows = b"".join(b"\x00" + bytes(rgb) * width for _ in range(height))

    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))


def box_glb(material):
    """A box over BOUNDS in one material whose maps are the export's PNGs, as one GLB."""
    low, high = BOUNDS
    corners = [[(low, high)[(i >> axis) & 1][axis] for axis in range(3)] for i in range(8)]
    faces = [(0, 2, 3, 1), (4, 5, 7, 6), (0, 1, 5, 4), (2, 6, 7, 3), (0, 4, 6, 2), (1, 3, 7, 5)]
    indices = [i for a, b, c, d in faces for i in (a, b, c, a, c, d)]
    positions = b"".join(struct.pack("<3f", *corner) for corner in corners)
    index_bytes = struct.pack(f"<{len(indices)}H", *indices)
    binary = positions + index_bytes
    while len(binary) % 4:
        binary += b"\x00"
    gltf = {
        "asset": {"version": "2.0"},
        "scene": 0, "scenes": [{"nodes": [0]}], "nodes": [{"mesh": 0}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "indices": 1, "material": 0}]}],
        "materials": [{"name": material,
                       "pbrMetallicRoughness": {"baseColorTexture": {"index": 0},
                                                "metallicRoughnessTexture": {"index": 1}},
                       "normalTexture": {"index": 2}}],
        "textures": [{"source": 0}, {"source": 1}, {"source": 2}],
        "images": [{"uri": f"{NAME}_albedo_red.png"}, {"uri": f"{NAME}_orm.png"}, {"uri": f"{NAME}_normal.png"}],
        "buffers": [{"byteLength": len(binary)}],
        "bufferViews": [{"buffer": 0, "byteOffset": 0, "byteLength": len(positions), "target": 34962},
                        {"buffer": 0, "byteOffset": len(positions), "byteLength": len(index_bytes), "target": 34963}],
        "accessors": [{"bufferView": 0, "componentType": 5126, "count": 8, "type": "VEC3",
                       "min": list(low), "max": list(high)},
                      {"bufferView": 1, "componentType": 5123, "count": len(indices), "type": "SCALAR"}],
    }
    text = json.dumps(gltf).encode()
    while len(text) % 4:
        text += b" "
    total = 12 + 8 + len(text) + 8 + len(binary)
    return (struct.pack("<III", 0x46546C67, 2, total) + struct.pack("<II", len(text), 0x4E4F534A) + text
            + struct.pack("<II", len(binary), 0x004E4942) + binary)


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def write_fixture(root):
    """A models export with its sidecar, a role, and a packs folder with one empty pack."""
    export = root / "models/export"
    export.mkdir(parents=True)
    (root / "models/preview.png").write_bytes(png(16, 10, (90, 120, 160)))
    maps = {f"{NAME}_orm.png": (255, 128, 0), f"{NAME}_normal.png": (128, 128, 255)}
    variants = {"red": (180, 40, 30), "blue": (40, 60, 170)}
    for variant, rgb in variants.items():
        maps[f"{NAME}_albedo_{variant}.png"] = rgb
    for file, rgb in maps.items():
        (export / file).write_bytes(png(4, 4, rgb))
    for tier in range(4):
        (export / f"{NAME}_lod{tier}.glb").write_bytes(box_glb(NAME))
    ref = lambda file: {"file": file, "sha256": sha256(export / file)}
    sidecar = {
        "schema": "metrum-model-facts", "version": 1, "name": NAME, "path": f"buildings/residential/{NAME}",
        "source": {"commit": "0" * 40, "dirty": False, "blender": "5.2.0"}, "license": "CC-BY-4.0",
        "bounds": {"min": BOUNDS[0], "max": BOUNDS[1]},
        "footprint": [[5.0, -5.1], [5.0, 5.1], [-5.0, 5.1], [-5.0, -5.1]],
        "storeys": [{"floor_y_m": 0.7, "area_m2": 86.6}, {"floor_y_m": 3.8, "area_m2": 58.0}],
        "dwellings": 1,
        "doors": [{"id": "front", "kind": "entrance", "position": [-1.2, 0.7, 5.1], "facing": [0.0, 0.0, 1.0],
                   "width_m": 0.9, "height_m": 2.1, "main": True}],
        "lods": [dict(ref(f"{NAME}_lod{tier}.glb"), triangles=12) for tier in range(4)],
        "textures": {"orm": ref(f"{NAME}_orm.png"), "normal": ref(f"{NAME}_normal.png")},
        "variants": [{"id": variant, "albedo": f"{NAME}_albedo_{variant}.png",
                      "sha256": sha256(export / f"{NAME}_albedo_{variant}.png")} for variant in variants],
        "default_variant": "red",
        "preview": {"file": "../preview.png", "sha256": sha256(root / "models/preview.png")},
        "hero": False,
    }
    (export / f"{NAME}.model.json").write_text(json.dumps(sidecar, indent=2))
    (root / "role.json").write_text(json.dumps({
        "pack_id": PACK, "asset_id": ASSET, "display_name": "Drafted House",
        "zone_type": "residential", "density": "low", "level": 1}))
    pack = root / "mods" / PACK
    pack.mkdir(parents=True)
    (pack / "pack.toml").write_text(
        f'pack_id = "{PACK}"\nschema_version = 1\ndisplay_name = "Draft Test"\nversion = "0.1.0"\n'
        'author = "Tester"\nlicense = "CC-BY-4.0"\n')
    return export / f"{NAME}.model.json", sidecar


def tree(folder):
    """Every file under `folder` with its bytes."""
    return {path.relative_to(folder): path.read_bytes() for path in sorted(Path(folder).rglob("*")) if path.is_file()}


class CommandTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.sidecar, _ = write_fixture(self.root)
        self.godot = self.root / "fake-godot"
        self.godot.write_text(FAKE_GODOT)
        self.godot.chmod(self.godot.stat().st_mode | stat.S_IEXEC)

    def tearDown(self):
        self.temp.cleanup()

    def draft(self, *extra):
        report = self.root / "report.json"
        status = model_draft.main(["--sidecar", str(self.sidecar), "--role", str(self.root / "role.json"),
                                   "--mods", str(self.root / "mods"), "--godot", str(self.godot),
                                   "--profile-root", str(self.root), "--report", str(report), *extra])
        return status, json.loads(report.read_text()) if report.is_file() else None

    def test_mods_is_required(self):
        with self.assertRaises(SystemExit), open(os.devnull, "w") as quiet:
            stderr, sys.stderr = sys.stderr, quiet
            try:
                model_draft.main(["--sidecar", "s", "--role", "r"])
            finally:
                sys.stderr = stderr

    def test_godot_runs_in_a_throwaway_profile_on_a_copy_of_the_pack(self):
        before = tree(self.root / "mods")
        status, report = self.draft()
        self.assertEqual(status, 0)
        seen = report["seen"]
        self.assertTrue(Path(seen["xdg"]).is_relative_to(self.root))
        self.assertTrue(Path(seen["request"]["mods"]).is_relative_to(self.root))
        self.assertEqual(seen["copied"], [PACK])
        self.assertIsNone(seen["request"]["write_mods"])
        self.assertEqual(seen["request"]["role"]["asset_id"], ASSET)
        self.assertEqual(tree(self.root / "mods"), before)
        # The profile is gone after the run.
        self.assertEqual(sorted(p.name for p in self.root.iterdir() if p.name.startswith("metrum-model-draft.")), [])

    def test_write_names_the_real_packs_folder_and_the_status_passes_through(self):
        os.environ["FAKE_GODOT_EXIT"] = "2"
        try:
            status, report = self.draft("--write")
        finally:
            del os.environ["FAKE_GODOT_EXIT"]
        self.assertEqual(status, 2)
        self.assertEqual(report["seen"]["request"]["write_mods"], os.path.abspath(self.root / "mods"))

    def test_a_pack_id_that_is_a_path_is_refused(self):
        with self.assertRaises(ValueError):
            model_draft.copy_pack(self.root / "mods", "../mods", self.root / "copy")
        with self.assertRaises(ValueError):
            model_draft.copy_pack(self.root / "mods", ".hidden", self.root / "copy")


@unittest.skipUnless(os.environ.get("METRUM_MODEL_DRAFT_GODOT") == "1", "needs Godot and the built library")
class GodotDraftTests(unittest.TestCase):
    """A generated model drafted through Godot into a temporary packs folder."""

    def test_draft_write_and_stale_report(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            sidecar_path, sidecar = write_fixture(root)
            mods = root / "mods"
            report_path = root / "report.json"

            def draft(*extra):
                status = model_draft.main(["--sidecar", str(sidecar_path), "--role", str(root / "role.json"),
                                           "--mods", str(mods), "--profile-root", str(root),
                                           "--report", str(report_path), *extra])
                return status, json.loads(report_path.read_text())

            status, report = draft()
            self.assertEqual((status, report["status"]), (0, "drafted"), report)
            self.assertEqual(report["acceptance"]["failures"], [], report)
            self.assertFalse((mods / PACK / "assets").exists(), "a draft without --write writes nothing")

            status, report = draft("--write")
            self.assertEqual((status, report["status"]), (0, "written"), report)
            asset = mods / PACK / "assets" / ASSET
            for file in ("asset.toml", "thumbnail.webp", f"{NAME}_lod3.glb", f"colours/{NAME}_albedo_blue.png"):
                self.assertTrue((asset / file).is_file(), file)
            self.assertTrue((mods / PACK / "model_sources" / f"{ASSET}.json").is_file())
            installed = tree(mods)

            status, report = draft("--write")
            self.assertEqual((status, report["status"], report["stale"]), (0, "existing", []), report)
            sidecar["doors"][0]["position"][0] = -2.0
            sidecar["dwellings"] = 2
            sidecar_path.write_text(json.dumps(sidecar))
            status, report = draft("--write")
            self.assertEqual((status, report["status"]), (2, "existing"), report)
            checks = {line["check"] for line in report["stale"]}
            self.assertTrue({"facts", "entrance", "capacity"} <= checks, report)
            self.assertEqual(tree(mods), installed, "an existing asset is never overwritten")


if __name__ == "__main__":
    unittest.main()
