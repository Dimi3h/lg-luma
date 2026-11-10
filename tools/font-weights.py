"""Freeze the bundled variable Inter font at the web UI's weights/optical sizes."""
import sys
import json
from pathlib import Path
from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont

directory = Path(sys.argv[1])
font = TTFont(directory / "font.ttf")
axes = {axis.axisTag for axis in font["fvar"].axes} if "fvar" in font else set()
metrics = {}
for weight in (350, 400, 450, 500, 550, 600, 650, 800):
    for display in (False, True):
        values = {"wght": weight} if "wght" in axes else {}
        if "opsz" in axes:
            values["opsz"] = 32 if display else 14
        instance = instantiateVariableFont(font, values, inplace=False) if axes else font
        instance.save(directory / f"font-{'display-' if display else ''}{weight}.ttf")
        units = instance["head"].unitsPerEm
        advances = instance["hmtx"].metrics
        metrics[f"{'display-' if display else ''}{weight}"] = {
            chr(codepoint): advances[glyph][0] / units
            for codepoint, glyph in instance.getBestCmap().items()
        }
(directory / "font-metrics.json").write_text(json.dumps(metrics, ensure_ascii=False), encoding="utf8")
