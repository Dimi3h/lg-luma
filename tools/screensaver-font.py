"""Subset Inter (display optical size, weight 500) to Basic Latin for the screensaver page.

The page shows only digits, a colon and English or Dutch day and month names, so printable
ASCII covers it. Output is a plain TrueType file: no WOFF2/Brotli dependency is needed.
"""
import sys
from pathlib import Path
from fontTools import subset
from fontTools.ttLib import TTFont

source, target = Path(sys.argv[1]), Path(sys.argv[2])
font = TTFont(source)
options = subset.Options()
options.layout_features = ["kern", "tnum", "liga", "calt"]
options.name_IDs = ["*"]
options.notdef_outline = True
subsetter = subset.Subsetter(options)
subsetter.populate(unicodes=range(0x20, 0x7F))
subsetter.subset(font)
target.parent.mkdir(parents=True, exist_ok=True)
font.save(target)
