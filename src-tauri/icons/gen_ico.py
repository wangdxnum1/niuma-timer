import struct
import os

# 简单稳健方案：把现有 64x64 PNG 直接包进 ICO 容器（Windows Vista+ 支持 PNG-in-ICO）
with open("icon.png", "rb") as f:
    png = f.read()

width = 64
height = 64
ico = bytearray()
ico += struct.pack("<HHH", 0, 1, 1)  # ICONDIR: reserved, type=1, count=1
ico += struct.pack("<BBBBHHII", width & 0xFF, height & 0xFF, 0, 0, 1, 32, len(png), 22)
ico += png

with open("icon.ico", "wb") as f:
    f.write(ico)
print("wrote icon.ico,", len(ico), "bytes")
