use tauri::image::Image;

const W: u32 = 64;
const H: u32 = 64;
const CX: f64 = 32.0;
const CY: f64 = 32.0;
const R: f64 = 30.0;

type RGB = (f64, f64, f64);

/// 品牌金色（延续前端 #ffd650）
const GOLD: RGB = (1.0, 214.0 / 255.0, 80.0 / 255.0);
/// 深色圆底（与前端深色主题一致）
const BG: RGB = (21.0 / 255.0, 24.0 / 255.0, 31.0 / 255.0);

// ---------- SDF（有符号距离场），1px 抗锯齿 ----------

fn sd_circle(x: f64, y: f64, cx: f64, cy: f64, r: f64) -> f64 {
    let dx = x - cx;
    let dy = y - cy;
    (dx * dx + dy * dy).sqrt() - r
}

fn sd_capsule(x: f64, y: f64, ax: f64, ay: f64, bx: f64, by: f64, r: f64) -> f64 {
    let pax = x - ax;
    let pay = y - ay;
    let bax = bx - ax;
    let bay = by - ay;
    let h = ((pax * bax + pay * bay) / (bax * bax + bay * bay)).clamp(0.0, 1.0);
    let dx = pax - bax * h;
    let dy = pay - bay * h;
    (dx * dx + dy * dy).sqrt() - r
}

/// SDF 距离 -> 覆盖率（边缘 1px 渐变）
fn cov(d: f64) -> f64 {
    (0.5 - d).clamp(0.0, 1.0)
}

fn blend(base: RGB, over: RGB, a: f64) -> RGB {
    (
        over.0 * a + base.0 * (1.0 - a),
        over.1 * a + base.1 * (1.0 - a),
        over.2 * a + base.2 * (1.0 - a),
    )
}

/// 静态托盘图标：金币指针（金色硬币外盘 + 深色内盘 + 12 点刻度 + 粗指针 + 中心轴点）
/// 设计意图：时间 = 金钱 —— 金币外圈暗示「钱」，盘面 + 指针暗示「时间」，双关明确、结构极简。
pub fn static_icon() -> Image<'static> {
    let mut buf = vec![0u8; (W * H * 4) as usize];
    for y in 0..H {
        for x in 0..W {
            let px = x as f64 + 0.5;
            let py = y as f64 + 0.5;
            let mut c = BG;

            // 金币外盘（金，实心大圆）
            let a = cov(sd_circle(px, py, 32.0, 32.0, 24.0));
            if a > 0.0 {
                c = blend(c, GOLD, a);
            }
            // 深色内盘（硬币边，负空间）
            let a = cov(sd_circle(px, py, 32.0, 32.0, 17.0));
            if a > 0.0 {
                c = blend(c, BG, a);
            }
            // 12 点刻度（金，粗短）
            let a = cov(sd_capsule(px, py, 32.0, 19.0, 32.0, 22.5, 2.0));
            if a > 0.0 {
                c = blend(c, GOLD, a);
            }
            // 指针（金，粗，指向约 2 点钟）
            let a = cov(sd_capsule(px, py, 32.0, 32.0, 44.0, 22.0, 3.5));
            if a > 0.0 {
                c = blend(c, GOLD, a);
            }
            // 中心轴点（金）
            let a = cov(sd_circle(px, py, 32.0, 32.0, 3.8));
            if a > 0.0 {
                c = blend(c, GOLD, a);
            }

            // 深色圆底裁剪
            let a_out = cov(sd_circle(px, py, CX, CY, R));
            // 输出预乘 alpha（premultiplied）：Windows 托盘 HICON 按预乘混合，
            // straight alpha 会让 0-alpha 像素残留深色 RGB、半透明边缘偏亮，
            // 在部分机器/主题下表现为白色底或亮边。
            let o = (y * W + x) as usize * 4;
            let a = a_out;
            buf[o] = (c.0 * a * 255.0) as u8;
            buf[o + 1] = (c.1 * a * 255.0) as u8;
            buf[o + 2] = (c.2 * a * 255.0) as u8;
            buf[o + 3] = (a * 255.0) as u8;
        }
    }
    Image::new_owned(buf, W, H)
}
