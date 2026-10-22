# SPDX-License-Identifier: GPL-2.0-only

"""Bake the far low-beam formula into Godot's spotlight projector angular space.

Run with Python + NumPy: python tools/bake_headlight_projector.py --preview
Use --check to verify the existing PNG instead of writing it. No engine is run.
The calibration assumes a working projector matrix (requires shadows in the
supplied Godot 4.7.2 source); it is not a measurement of a rendered frame.
"""

import argparse
from pathlib import Path
import re
import struct
import zlib

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
TEXTURE = ROOT / "godot/assets/textures/vehicles/headlight_projector.png"
WIDTH, HEIGHT = 512, 1024
ANGLE_DEG = 60.0  # Godot's half-angle; square projection despite texture aspect.
RANGE_M = 60.0
RADIAL_DECAY = 2.0
ANGLE_ATTENUATION = 0.02  # Engine uses its reciprocal: rim exponent 50.
PATTERN_PEAK = 1.18
ALBEDO = 0.25  # Linear diffuse reflectance, not an sRGB byte value.
LAMP_HEIGHT_M = 0.5
LAMP_HALF_SPACING_M = 0.55
SPOT_FORWARD_OFFSET_M = 0.15
INTENSITY_CD = 30000.0
ENERGY_PER_LUX = 0.012


def smoothstep(edge0, edge1, value):
    t = np.clip((value - edge0) / (edge1 - edge0), 0.0, 1.0)
    return t * t * (3.0 - 2.0 * t)


def pattern(spread, vertical):
    # Exact core/fore/cut-off expression from vehicle_headlight_beam.gdshader.
    drop = vertical + 0.021
    core = np.exp(-drop * drop / 0.0009) * np.exp(-spread * spread * 2.5)
    fore = 0.18 * np.exp(-drop * drop / 0.0196) * np.exp(-spread * spread * 1.2)
    return (core + fore) * smoothstep(0.006, -0.006, vertical)


def srgb_encode(linear):
    return np.where(linear <= 0.0031308, linear * 12.92,
                    1.055 * np.maximum(linear, 0.0) ** (1.0 / 2.4) - 0.055)


def srgb_decode(srgb):
    return np.where(srgb <= 0.04045, srgb / 12.92,
                    ((srgb + 0.055) / 1.055) ** 2.4)


def projector_pixels():
    tangent = np.tan(np.deg2rad(ANGLE_DEG))
    # Engine bias maps X/Y to 0..1; projector_rect reverses Y in the atlas.
    # PNG top is positive local Y (above horizon), bottom is negative Y.
    spread = (2.0 * (np.arange(WIDTH) + 0.5) / WIDTH - 1.0) * tangent
    vertical = (1.0 - 2.0 * (np.arange(HEIGHT) + 0.5) / HEIGHT) * tangent
    linear = pattern(spread[None, :], vertical[:, None]) / PATTERN_PEAK
    grey = np.rint(srgb_encode(linear) * 255.0).astype(np.uint8)
    # Opaque grey RGB avoids single-channel sampler swizzle ambiguity. The atlas
    # decodes RGB from sRGB, then multiplies by alpha (implicitly 1 here).
    return np.repeat(grey[:, :, None], 3, axis=2)


def png_bytes(rgb):
    height, width, channels = rgb.shape
    assert channels == 3 and rgb.dtype == np.uint8

    def chunk(kind, data):
        return (struct.pack(">I", len(data)) + kind + data
                + struct.pack(">I", zlib.crc32(kind + data)))

    rows = b"".join(b"\0" + row.tobytes() for row in rgb)
    return (b"\x89PNG\r\n\x1a\n"
            + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
            + chunk(b"sRGB", b"\0")
            + chunk(b"IDAT", zlib.compress(rows, 9)) + chunk(b"IEND", b""))


def sample_projector(rgb, spread, vertical):
    # Linear LOD0 reference sampling of decoded texels. The default engine filter
    # also uses mipmaps; GPU derivatives/atlas packing are not modelled here.
    tangent = np.tan(np.deg2rad(ANGLE_DEG))
    px = (0.5 + spread / (2.0 * tangent)) * WIDTH - 0.5
    py = (0.5 - vertical / (2.0 * tangent)) * HEIGHT - 0.5
    ix, iy = np.floor(px).astype(int), np.floor(py).astype(int)
    fx, fy = px - ix, py - iy
    texels = srgb_decode(rgb[:, :, 0].astype(float) / 255.0)

    def at(x, y):
        return texels[np.clip(y, 0, HEIGHT - 1), np.clip(x, 0, WIDTH - 1)]

    return ((at(ix, iy) * (1.0 - fx) + at(ix + 1, iy) * fx) * (1.0 - fy)
            + (at(ix, iy + 1) * (1.0 - fx) + at(ix + 1, iy + 1) * fx) * fy)


def beam_light(ahead, lateral):
    # Flat road, upward normal, two lens centres at (+/-0.55, 0.5, 0).
    result = np.zeros_like(ahead + lateral, dtype=float)
    for side in (-1.0, 1.0):
        x = lateral - side * LAMP_HALF_SPACING_M
        r2 = np.maximum(ahead * ahead + x * x + LAMP_HEIGHT_M ** 2, 0.01)
        result += (INTENSITY_CD * ENERGY_PER_LUX
                   * pattern(x / ahead, -LAMP_HEIGHT_M / ahead)
                   * (LAMP_HEIGHT_M / np.sqrt(r2)) / r2)
    return result


def spot_light(ahead, lateral, energy, rgb):
    # One centred spot just ahead of the lenses; unpitched because the texture
    # already encodes the 1.2 degree dip. White/red channel only, night=fade=1.
    forward = ahead - SPOT_FORWARD_OFFSET_M
    radius = np.sqrt(forward * forward + lateral * lateral + LAMP_HEIGHT_M ** 2)
    window = np.maximum(1.0 - (radius / RANGE_M) ** 4, 0.0) ** 2
    cosine = np.cos(np.deg2rad(ANGLE_DEG))
    scos = np.maximum(forward / radius, cosine)
    rim = np.maximum(1e-4, (1.0 - scos) / (1.0 - cosine))
    cone = 1.0 - rim ** (1.0 / ANGLE_ATTENUATION)
    projector = sample_projector(rgb, lateral / forward, -LAMP_HEIGHT_M / forward)
    # Nonphysical engine energy *= PI; Lambertian paving_diffuse divides by PI.
    # The final clustered shader multiplies diffuse_light by linear albedo.
    return (energy * projector * window * radius ** -RADIAL_DECAY
            * cone * (LAMP_HEIGHT_M / radius) * ALBEDO)


def calibrate(rgb):
    ahead, lateral = np.meshgrid(np.array([15.0, 20.0, 25.0]),
                                np.array([0.0, 4.0, 8.0]), indexing="ij")
    beam = beam_light(ahead, lateral)
    per_energy = spot_light(ahead, lateral, 1.0, rgb)
    # Equal weight to relative errors, so bright/near samples do not dominate.
    ratio = per_energy / beam
    fitted = np.sum(ratio) / np.sum(ratio * ratio)
    return float(np.rint(fitted / 10.0) * 10.0)


def calibration_table(rgb, energy):
    lines = [" ahead  lateral  beam added  spot * 0.25  spot/beam",
             "   (m)      (m)       (linear red-channel contribution)"]
    for ahead in (10.0, 15.0, 20.0, 25.0, 35.0, 45.0):
        for lateral in (0.0, 4.0, 8.0):
            beam = float(beam_light(np.array(ahead), np.array(lateral)))
            spot = float(spot_light(np.array(ahead), np.array(lateral), energy, rgb))
            lines.append(f"{ahead:6.1f}  {lateral:7.1f}  {beam:10.6f}  {spot:11.6f}  {spot / beam:9.3f}")
    return "\n".join(lines)


def verify(rgb, energy):
    near_source = (ROOT / "godot/scripts/renderers/vehicle_headlights.gd").read_text()
    far_source = (ROOT / "godot/scripts/shaders/vehicle_headlight_beam.gdshader").read_text()
    radius = float(re.search(r"const POOL_RADIUS_M := ([\d.]+)", near_source)[1])
    fade = float(re.search(r"const POOL_FADE_M := ([\d.]+)", near_source)[1])
    far_band = tuple(map(float, re.search(r"head_fade_in_m = vec2\(([\d.]+), ([\d.]+)\)", far_source).groups()))
    assert (radius - fade, radius) == far_band == (70.0, 100.0)
    distances = np.array([50., 70., 77.5, 85., 92.5, 100., 120.])
    near = 1.0 - smoothstep(radius - fade, radius, distances)
    far = smoothstep(*far_band, distances)
    np.testing.assert_allclose(near + far, 1.0, atol=1e-15)
    np.testing.assert_allclose(near[[0, 1, 3, 5, 6]], [1., 1., 0.5, 0., 0.])
    assert np.isfinite(rgb).all() and rgb.shape == (HEIGHT, WIDTH, 3)
    assert np.all(rgb[:HEIGHT // 2 - 2] == 0), "Light leaks above cut-off"
    assert np.array_equal(rgb[:, :, 0], rgb[:, ::-1, 0]), "Asymmetric bake"
    # Decode independent PNG scanlines and CRCs rather than trusting only encode.
    data = TEXTURE.read_bytes()
    assert data[:8] == b"\x89PNG\r\n\x1a\n"
    offset, payload = 8, b""
    while offset < len(data):
        length = struct.unpack_from(">I", data, offset)[0]
        kind = data[offset + 4:offset + 8]
        body = data[offset + 8:offset + 8 + length]
        crc = struct.unpack_from(">I", data, offset + 8 + length)[0]
        assert crc == zlib.crc32(kind + body)
        if kind == b"IHDR":
            assert struct.unpack(">IIBBBBB", body) == (WIDTH, HEIGHT, 8, 2, 0, 0, 0)
        if kind == b"IDAT":
            payload += body
        offset += length + 12
    rows = np.frombuffer(zlib.decompress(payload), np.uint8).reshape(HEIGHT, WIDTH * 3 + 1)
    assert np.all(rows[:, 0] == 0)
    assert np.array_equal(rows[:, 1:].reshape(rgb.shape), rgb), "Stale projector PNG"
    # LOD0 bake accuracy near the core, foreground and cutoff; GPU mip/atlas
    # sampling remains a runtime check. Cutoff spans about 3.55 vertical texels.
    spread, vertical = np.meshgrid(np.linspace(-1.2, 1.2, 101),
                                  np.linspace(-0.25, 0.02, 301))
    exact = pattern(spread, vertical) / PATTERN_PEAK
    sampled = sample_projector(rgb, spread, vertical)
    # At this resolution the sharp cutoff has <3% absolute interpolation error;
    # the road calibration samples below it retain much tighter agreement.
    assert np.max(np.abs(exact - sampled)) < 0.03
    ahead, lateral = np.meshgrid(np.array([15., 20., 25.]), np.array([0., 4., 8.]))
    ratio = spot_light(ahead, lateral, energy, rgb) / beam_light(ahead, lateral)
    assert np.max(np.abs(ratio - 1.0)) < 0.08
    print(f"Checks passed: 70..100 m fade bands, PNG/CRC/determinism, cutoff, symmetry, angular error "
          f"{np.max(np.abs(exact - sampled)):.5f}; calibration ratios "
          f"{ratio.min():.3f}..{ratio.max():.3f}.")


def preview(rgb, energy):
    # Left: far beam; right: working near projector. Ahead increases upward,
    # 1..45 m vertically, lateral -16..16 m horizontally. Both share exposure.
    lateral, ahead = np.meshgrid(np.linspace(-16.0, 16.0, 512), np.linspace(45.0, 1.0, 704))
    panels = []
    for light in (beam_light(ahead, lateral), spot_light(ahead, lateral, energy, rgb)):
        display = srgb_encode(1.0 - np.exp(-2.0 * light))
        panels.append(np.repeat(np.rint(display * 255.0).astype(np.uint8)[:, :, None], 3, axis=2))
    divider = np.full((704, 12, 3), 80, dtype=np.uint8)
    path = ROOT / "HEADLIGHTS_COMPARISON.png"
    path.write_bytes(png_bytes(np.concatenate((panels[0], divider, panels[1]), axis=1)))
    print(f"Wrote {path.relative_to(ROOT)} (left far, right near; same exposure).")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="Verify bake without rewriting it")
    parser.add_argument("--preview", action="store_true", help="Write optional top-down comparison")
    args = parser.parse_args()
    rgb = projector_pixels()
    energy = calibrate(rgb)
    if not args.check:
        TEXTURE.parent.mkdir(parents=True, exist_ok=True)
        TEXTURE.write_bytes(png_bytes(rgb))
        print(f"Wrote {TEXTURE.relative_to(ROOT)} ({WIDTH}x{HEIGHT}, opaque sRGB grey RGB8).")
    print(f"Proposed working-projector settings: range={RANGE_M:g}, angle={ANGLE_DEG:g}, "
          f"decay={RADIAL_DECAY:g}, angle_attenuation={ANGLE_ATTENUATION:g}, ENERGY={energy:g}")
    print(calibration_table(rgb, energy))
    verify(rgb, energy)
    if args.preview:
        preview(rgb, energy)


if __name__ == "__main__":
    main()
