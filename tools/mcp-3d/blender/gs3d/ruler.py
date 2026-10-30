"""The ruler: every size question is asked against the thing that walks — your player.

Numbers come from `tools/mcp-3d/ruler.json` (or the file in `GS3D_RULER`) so the probes,
the render cameras and your game agree. Edit that file when your game's player size or
camera changes (the `configure` MCP tool does it for you and restarts Blender).

Units are metres. Blender is Z-up; the game (three.js) is Y-up with -z forward. Blender +X =
game +x, Blender +Y = game -z, Blender +Z = game +y. glTF export with +Y up converts back.
"""

import json
import os

_HERE = os.path.dirname(os.path.abspath(__file__))
PATH = os.environ.get("GS3D_RULER") or os.path.normpath(os.path.join(_HERE, "..", "..", "ruler.json"))

DEFAULTS = {
    # Orb Arena's player: a 0.5 m-radius ball (crates/game/src/lib.rs PLAYER_RADIUS).
    "avatar_radius": 0.5,
    "avatar_height": 1.0,
    # packages/engine/src/three.tsx <FollowCamera> + GameCanvas: 60° FOV, 7.5 m, ~29° down.
    "camera_fov_deg": 60.0,
    "camera_dist_min": 5.0,
    "camera_dist": 7.5,
    "camera_dist_max": 14.0,
    "camera_pitch_deg": 29.0,
    "camera_pitch_far_deg": 45.0,
    # Grid used for level layout (metres per tile).
    "tile_m": 1.0,
    # Highest ledge the player can step/jump onto without a ramp (metres).
    "step_up": 0.4,
}


def load(path: str = PATH) -> dict:
    cfg = dict(DEFAULTS)
    try:
        with open(path) as f:
            cfg.update({k: v for k, v in json.load(f).items() if k in DEFAULTS})
    except FileNotFoundError:
        pass
    return cfg


CFG = load()

AVATAR_RADIUS = float(CFG["avatar_radius"])
AVATAR_HEIGHT = float(CFG["avatar_height"])
AH = AVATAR_HEIGHT

TILE_M = float(CFG["tile_m"])

CAMERA_FOV_DEG = float(CFG["camera_fov_deg"])
ZOOM_MIN = float(CFG["camera_dist_min"])
ZOOM_DEFAULT = float(CFG["camera_dist"])
ZOOM_MAX = float(CFG["camera_dist_max"])
PITCH_NEAR_DEG = float(CFG["camera_pitch_deg"])
PITCH_FAR_DEG = float(CFG["camera_pitch_far_deg"])

PASSABLE_WIDTH = AVATAR_RADIUS * 2 + 0.3 * AH
PASSABLE_HEIGHT = AVATAR_HEIGHT + 0.3 * AH
STEP_UP = float(CFG["step_up"])


def ah(metres: float) -> float:
    return metres / AH


def fmt_ah(metres: float) -> str:
    return f"{metres / AH:.2f}ah"


# Proportion bands for whole things (a part of an assembly is exempt), written for a human-
# sized 1.7 m avatar and scaled to yours, so a door always fits the thing that walks through
# it. Wide on purpose: they catch scale confusion (a lamp taller than the house), not realism.
_REF_AH = 1.7
_BANDS_REF = {
    # tag: (min height, max height, note)
    "door": (1.9, 3.4, "a door the avatar walks through: 1.1–2 ah"),
    "gate": (2.6, 7.0, "a gate"),
    "window": (0.55, 2.2, "a window"),
    "storey": (2.4, 4.4, "one storey of a building"),
    "lamp": (1.9, 4.6, "a lamp post: above the avatar, below the eaves"),
    "counter": (0.8, 1.25, "a counter at chest height"),
    "bench": (0.4, 0.6, "a seat"),
    "rail": (0.8, 1.25, "a railing"),
    "house": (4.0, 16.0, "a house: 2.4–9 ah"),
    "shop": (4.0, 16.0, "a shop"),
    "hall": (9.0, 30.0, "a hall: the tallest thing around"),
    "tower": (10.0, 44.0, "a tower / landmark"),
    "wall": (3.2, 9.0, "a defensive wall"),
    "tree": (3.5, 24.0, "a tree"),
    "sign": (0.35, 1.6, "a sign"),
    "stall": (2.4, 4.6, "a stall under its canopy"),
    "crate": (0.4, 1.6, "a crate the avatar can push or hop on"),
    "pickup": (0.2, 0.9, "a pickup: smaller than the avatar, readable from the camera"),
    "prop": (0.2, 3.0, "a generic prop"),
}
_K = AH / _REF_AH
BANDS = {tag: (round(lo * _K, 2), round(hi * _K, 2), what) for tag, (lo, hi, what) in _BANDS_REF.items()}
