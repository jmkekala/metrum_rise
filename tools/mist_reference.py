# SPDX-License-Identifier: GPL-2.0-only

"""Checks godot/scripts/shaders/weather_mist.glsl's 6-segment integration against a converged reference.

A synthetic world: a lake at 80 m over x < 0 with a 128 m shore ramp of presence, terrain rising
inland. Rays from cameras at 30-900 m in every direction end on the ground, the lake surface or
the sky. The shader's estimate holds presence and level constant per segment (midpoint) and
integrates the height profile in closed form; the reference sums 4000 steps of the true field.
Bank noise is left out: it multiplies both the same way and varies over hundreds of metres.
"""
import math
import random

H = 14.0          # scale height
EXT = 0.016       # extinction per metre at the surface
LAKE = 80.0
REACH = 128.0
CEILING = LAKE + 5 * H
SKY = 12000.0
SEGMENTS = 6


def presence(x):
    if x <= 0.0:
        return 1.0
    t = max(0.0, 1.0 - x / REACH)
    return t * t * (3 - 2 * t)


def ground(x):
    return LAKE - 3.0 if x <= 0 else LAKE + 0.04 * x  # lake bed, then a 4 % rise inland


def at(origin, d, t):
    return [origin[k] + d[k] * t for k in range(3)]


def layer_depth(y0, y1, length, level, scale):
    low, high = min(y0, y1), max(y0, y1)
    if high <= level:
        return 0.0
    rise = high - low
    above = (high - max(low, level)) / rise if rise > 1e-3 else 1.0
    a, b = max(low, level) - level, high - level
    if b - a < 1e-3:
        return length * above * math.exp(-0.5 * (a + b) / scale)
    mean = scale * (math.exp(-a / scale) - math.exp(-b / scale)) / (b - a)
    return length * above * mean


def endpoint(origin, d):
    # March to the ground or the lake surface (the opaque depth may be the bed; the shader clips at
    # the level either way, so ending at the bed checks that clip).
    t = 0.0
    while t < SKY:
        p = at(origin, d, t)
        if p[1] <= ground(p[0]):
            return t
        t += 2.0
    return None


def shader(origin, d, dist):
    t0, t1 = 0.0, dist
    if abs(d[1]) < 1e-5:
        if origin[1] > CEILING:
            return 0.0
    else:
        tc = (CEILING - origin[1]) / d[1]
        if d[1] < 0:
            t0 = max(t0, tc)
        else:
            t1 = min(t1, tc)
    if t1 <= t0:
        return 0.0
    step = (t1 - t0) / SEGMENTS
    tau = 0.0
    for i in range(SEGMENTS):
        a = at(origin, d, t0 + step * i)
        b = at(origin, d, t0 + step * (i + 1))
        mid = 0.5 * (a[0] + b[0])
        p = presence(mid)
        if p > 0:
            tau += p * layer_depth(a[1], b[1], step, LAKE, H)
    return EXT * tau


def reference(origin, d, dist, steps=4000):
    dt = dist / steps
    tau = 0.0
    for i in range(steps):
        p = at(origin, d, dt * (i + 0.5))
        if p[1] >= LAKE:
            tau += presence(p[0]) * math.exp(-(p[1] - LAKE) / H) * dt
    return EXT * tau


rng = random.Random(3)
errors = []
for _ in range(600):
    origin = [rng.uniform(-600, 600), LAKE + rng.uniform(30, 900), 0.0]
    pitch = rng.uniform(-80, 5) * math.pi / 180
    yaw = rng.uniform(0, 2 * math.pi)
    d = [math.cos(pitch) * math.cos(yaw), math.sin(pitch), math.cos(pitch) * math.sin(yaw)]
    hit = endpoint(origin, d)
    dist = hit if hit is not None else SKY
    est, ref = shader(origin, d, dist), reference(origin, d, dist)
    # Compare the mist's opacity, 1 - transmittance, which is what the eye sees.
    errors.append(abs((1 - math.exp(-est)) - (1 - math.exp(-ref))))
errors.sort()
pick = lambda q: errors[min(len(errors) - 1, int(q * len(errors)))]
print("opacity error over 600 rays: mean %.4f, p95 %.4f, p99 %.4f, max %.4f, share above 1/255: %.3f"
      % (sum(errors) / len(errors), pick(0.95), pick(0.99), errors[-1],
         sum(e > 1 / 255 for e in errors) / len(errors)))
