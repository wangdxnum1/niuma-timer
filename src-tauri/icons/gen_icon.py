#!/usr/bin/env python3
"""生成牛马计时器图标 icon.png（64x64，含 ¥ 字样）。
仅依赖 Python 标准库（zlib / struct / datetime）。
"""
import os
import zlib
import struct

W = H = 64
SCALE = 4  # 每个点阵像素放大倍数

# 5x7 点阵字体（仅 ¥ 与数字 / 小数点，用于图标）
FONT = {
    "Y": [  # 用 Y 形近似 ¥
        "01010",
        "01010",
        "11111",
        "01010",
        "01010",
        "10001",
        "10001",
    ],
    "0": ["01110","10001","10011","10101","11001","10001","01110"],
    "1": ["00100","01100","00100","00100","00100","00100","01110"],
    "2": ["01110","10001","00001","00010","00100","01000","11111"],
    "3": ["11111","00010","00100","00010","00001","10001","01110"],
    "4": ["00010","00110","01010","10010","11111","00010","00010"],
    "5": ["11111","10000","11110","00001","00001","10001","01110"],
    "6": ["00110","01000","10000","11110","10001","10001","01110"],
    "7": ["11111","00001","00010","00100","01000","01000","01000"],
    "8": ["01110","10001","10001","01110","10001","10001","01110"],
    "9": ["01110","10001","10001","01111","00001","00010","01100"],
    ".": ["00000","00000","00000","00000","00000","00100","00100"],
}

BG = (28, 30, 38, 255)
FG = (255, 214, 80, 255)  # 金黄


def draw_text(grid, text, scale, color):
    gx = 5 * scale
    gy = 7 * scale
    total_w = gx * len(text)
    start_x = (W - total_w) // 2
    start_y = (H - gy) // 2
    for i, ch in enumerate(text):
        glyph = FONT.get(ch)
        if glyph is None:
            continue
        ox = start_x + i * gx
        for r in range(7):
            row = glyph[r]
            for c in range(5):
                if row[c] == "1":
                    for dy in range(scale):
                        for dx in range(scale):
                            x = ox + c * scale + dx
                            y = start_y + r * scale + dy
                            if 0 <= x < W and 0 <= y < H:
                                grid[y][x] = color
    return grid


def to_png(path):
    grid = [[BG for _ in range(W)] for _ in range(H)]
    grid = draw_text(grid, "Y", SCALE, FG)
    raw = bytearray()
    for y in range(H):
        raw.append(0)  # filter type 0
        for x in range(W):
            r, g, b, a = grid[y][x]
            raw += bytes((r, g, b, a))
    compressed = zlib.compress(bytes(raw), 9)

    def chunk(tag, data):
        out = struct.pack(">I", len(data)) + tag + data
        crc = zlib.crc32(tag + data) & 0xFFFFFFFF
        out += struct.pack(">I", crc)
        return out

    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", W, H, 8, 6, 0, 0, 0))
    png += chunk(b"IDAT", compressed)
    png += chunk(b"IEND", b"")
    with open(path, "wb") as f:
        f.write(png)
    print("wrote", path, os.path.getsize(path), "bytes")


if __name__ == "__main__":
    here = os.path.dirname(os.path.abspath(__file__))
    to_png(os.path.join(here, "icon.png"))
