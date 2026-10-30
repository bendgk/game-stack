"""gs3d — game-stack's headless Blender kit: measured modelling in the game's own look.

Import in `run_python` as `from gs3d import *`: geometry (`geo`), parts and paint
(`Model`, `to_object`), the look and cameras (`look`), probes (`probe.lint`, `probe.gate`),
rigging (`rig`), export (`export.glb`), the ruler (`ruler`) and the architecture kit
(`kit`).
"""

from . import export, geo, kit, look, probe, rig, ruler  # noqa: F401
from .model import Model, hex_rgb, to_object  # noqa: F401

__all__ = ["export", "geo", "kit", "look", "probe", "rig", "ruler", "Model", "hex_rgb", "to_object"]
