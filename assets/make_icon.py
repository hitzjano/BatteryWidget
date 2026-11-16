#!/usr/bin/env python3
"""Regenerates assets/icon.ico (requires Pillow). The icon is a plain battery glyph."""
from pathlib import Path

from PIL import Image, ImageDraw

SIZE = 256
GREEN = (46, 184, 114, 255)
DARK = (27, 31, 36, 255)
WHITE = (255, 255, 255, 255)

image = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
draw = ImageDraw.Draw(image)
draw.rounded_rectangle((8, 8, 248, 248), radius=56, fill=DARK)
# Battery body, terminal and a 70 % charge.
draw.rounded_rectangle((44, 84, 196, 172), radius=20, outline=WHITE, width=12)
draw.rounded_rectangle((200, 108, 216, 148), radius=6, fill=WHITE)
draw.rounded_rectangle((64, 104, 150, 152), radius=8, fill=GREEN)

target = Path(__file__).with_name("icon.ico")
image.save(target, sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (256, 256)])
print(f"wrote {target}")
