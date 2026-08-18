use tauri::image::Image;

const W: u32 = 64;
const H: u32 = 64;
const BG: (u8, u8, u8, u8) = (28, 30, 38, 255);
const FG: (u8, u8, u8, u8) = (255, 214, 80, 255);

/// 5x7 点阵字体（仅数字/小数点/¥）
fn glyph(ch: char) -> Option<[&'static str; 7]> {
    let g = match ch {
        '¥' | 'Y' => [
            "01010", "01010", "11111", "01010", "01010", "10001", "10001",
        ],
        '0' => ["01110", "10001", "10011", "10101", "11001", "10001", "01110"],
        '1' => ["00100", "01100", "00100", "00100", "00100", "00100", "01110"],
        '2' => ["01110", "10001", "00001", "00010", "00100", "01000", "11111"],
        '3' => ["11111", "00010", "00100", "00010", "00001", "10001", "01110"],
        '4' => ["00010", "00110", "01010", "10010", "11111", "00010", "00010"],
        '5' => ["11111", "10000", "11110", "00001", "00001", "10001", "01110"],
        '6' => ["00110", "01000", "10000", "11110", "10001", "10001", "01110"],
        '7' => ["11111", "00001", "00010", "00100", "01000", "01000", "01000"],
        '8' => ["01110", "10001", "10001", "01110", "10001", "10001", "01110"],
        '9' => ["01110", "10001", "10001", "01111", "00001", "00010", "01100"],
        '.' => ["00000", "00000", "00000", "00000", "00000", "00100", "00100"],
        _ => return None,
    };
    Some(g)
}

/// 将文字渲染为 64x64 RGBA 图标
pub fn render_icon(text: &str) -> Image {
    let mut buf = vec![0u8; (W * H * 4) as usize];
    for i in 0..(W * H) as usize {
        let o = i * 4;
        buf[o] = BG.0;
        buf[o + 1] = BG.1;
        buf[o + 2] = BG.2;
        buf[o + 3] = BG.3;
    }

    let chars: Vec<char> = text.chars().collect();
    let len = chars.len() as u32;
    if len == 0 {
        return Image::from_rgba(buf, W, H).expect("render icon");
    }
    let scale = if len <= 4 { 2 } else { 1 };
    let gx = 5 * scale;
    let gy = 7 * scale;
    let gap = scale;
    let total_w = gx * len + gap * len.saturating_sub(1);
    let start_x = if total_w < W { (W - total_w) / 2 } else { 0 };
    let start_y = (H - gy) / 2;

    for (i, &ch) in chars.iter().enumerate() {
        if let Some(g) = glyph(ch) {
            let ox = start_x + i as u32 * (gx + gap);
            for r in 0..7u32 {
                let row = g[r as usize];
                for c in 0..5u32 {
                    if row.as_bytes()[c as usize] == b'1' {
                        for dy in 0..scale {
                            for dx in 0..scale {
                                let x = ox + c * scale + dx;
                                let y = start_y + r * scale + dy;
                                if x < W && y < H {
                                    let o = (y * W + x) as usize * 4;
                                    buf[o] = FG.0;
                                    buf[o + 1] = FG.1;
                                    buf[o + 2] = FG.2;
                                    buf[o + 3] = FG.3;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Image::from_rgba(buf, W, H).expect("render icon")
}
