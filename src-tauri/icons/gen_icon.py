#!/usr/bin/env python3
"""生成牛马计时器程序图标（金币指针）：icon.png (512x512) + icon.ico (16~256 多尺寸)。
与 src-tauri/src/icon_render.rs 的 SDF 算法一致，仅依赖 Python 标准库。
设计：金色硬币外盘 + 深色内盘 + 12 点刻度 + 粗指针 + 中心轴点 —— 时间=金钱双关。
"""
import os
import zlib
import struct
import math

GOLD = (1.0, 214.0 / 255.0, 80.0 / 255.0)
BG = (21.0 / 255.0, 24.0 / 255.0, 31.0 / 255.0)


def sd_circle(x, y, cx, cy, r):
    return math.hypot(x - cx, y - cy) - r


def sd_round_box(x, y, cx, cy, hw, hh, r):
    dx = abs(x - cx) - (hw - r)
    dy = abs(y - cy) - (hh - r)
    ox, oy = max(dx, 0.0), max(dy, 0.0)
    return math.hypot(ox, oy) + min(max(dx, dy), 0.0) - r


def sd_ring(x, y, cx, cy, ro, ri):
    d = math.hypot(x - cx, y - cy)
    return abs(d - (ro + ri) * 0.5) - (ro - ri) * 0.5


def sd_capsule(x, y, ax, ay, bx, by, r):
    pax, pay = x - ax, y - ay
    bax, bay = bx - ax, by - ay
    denom = bax * bax + bay * bay
    h = 0.0 if denom == 0 else max(0.0, min(1.0, (pax * bax + pay * bay) / denom))
    dx, dy = pax - bax * h, pay - bay * h
    return math.hypot(dx, dy) - r


def cov(d):
    return max(0.0, min(1.0, 0.5 - d))


def render_px(px, py):
    c = BG

    def blend(over, a):
        nonlocal c
        if a > 0:
            c = (over[0] * a + c[0] * (1 - a), over[1] * a + c[1] * (1 - a), over[2] * a + c[2] * (1 - a))

    blend(GOLD, cov(sd_circle(px, py, 32.0, 32.0, 24.0)))              # 金币外盘
    blend(BG,   cov(sd_circle(px, py, 32.0, 32.0, 17.0)))              # 深色内盘（硬币边）
    blend(GOLD, cov(sd_capsule(px, py, 32.0, 19.0, 32.0, 22.5, 2.0)))  # 12 点刻度
    blend(GOLD, cov(sd_capsule(px, py, 32.0, 32.0, 44.0, 22.0, 3.5)))  # 指针（指向约 2 点钟）
    blend(GOLD, cov(sd_circle(px, py, 32.0, 32.0, 3.8)))               # 中心轴点
    a_out = cov(sd_circle(px, py, 32.0, 32.0, 30.0))                   # 圆底
    return c + (a_out,)


def render_rgba(size, ss=2):
    """渲染 size×size RGBA，超采样 ss×ss 抗锯齿，返回展平字节"""
    flat = bytearray()
    for py_ in range(size):
        for px_ in range(size):
            rs = gs = bs = as_ = 0
            for sy in range(ss):
                for sx in range(ss):
                    lx = (px_ + (sx + 0.5) / ss) * 64.0 / size
                    ly = (py_ + (sy + 0.5) / ss) * 64.0 / size
                    r, g, b, a = render_px(lx, ly)
                    rs += r; gs += g; bs += b; as_ += a
            n = ss * ss
            flat += bytes((int(rs / n * 255), int(gs / n * 255), int(bs / n * 255), int(as_ / n * 255)))
    return bytes(flat)


def write_png(path, size, rgba):
    def chunk(tag, data):
        out = struct.pack(">I", len(data)) + tag + data
        return out + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    raw = bytearray()
    for y in range(size):
        raw.append(0)
        raw += rgba[y * size * 4:(y + 1) * size * 4]
    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(bytes(raw), 9))
    png += chunk(b"IEND", b"")
    with open(path, "wb") as f:
        f.write(png)
    return png


def write_ico(path, pngs):
    """pngs: list[(size, png_bytes)]，打包成 Windows ICO（PNG 压缩格式，Win Vista+）"""
    header = struct.pack("<HHH", 0, 1, len(pngs))
    offset = 6 + 16 * len(pngs)
    entries = bytearray()
    blob = bytearray()
    for size, png in pngs:
        b = 0 if size >= 256 else size
        entries += struct.pack("<BBBBHHII", b, b, 0, 0, 1, 32, len(png), offset)
        offset += len(png)
        blob += png
    with open(path, "wb") as f:
        f.write(header + bytes(entries) + bytes(blob))


if __name__ == "__main__":
    here = os.path.dirname(os.path.abspath(__file__))
    # icon.png：512x512 大图（超采样 1 即可，SDF 连续）
    png_512 = write_png(os.path.join(here, "icon.png"), 512, render_rgba(512, ss=1))
    # icon.ico：16/24/32/48/64/128/256 多尺寸
    ico_pngs = []
    for s in (16, 24, 32, 48, 64, 128, 256):
        ico_pngs.append((s, write_png(os.path.join(here, f"_tmp_{s}.png"), s, render_rgba(s, ss=2))))
    write_ico(os.path.join(here, "icon.ico"), ico_pngs)
    for s, _ in ico_pngs:
        os.remove(os.path.join(here, f"_tmp_{s}.png"))
    print("icon.png:", os.path.getsize(os.path.join(here, "icon.png")), "bytes")
    print("icon.ico:", os.path.getsize(os.path.join(here, "icon.ico")), "bytes")
