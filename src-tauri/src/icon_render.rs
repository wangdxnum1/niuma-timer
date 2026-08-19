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

fn sd_round_box(x: f64, y: f64, cx: f64, cy: f64, hw: f64, hh: f64, r: f64) -> f64 {
    let dx = (x - cx).abs() - (hw - r);
    let dy = (y - cy).abs() - (hh - r);
    let ox = dx.max(0.0);
    let oy = dy.max(0.0);
    (ox * ox + oy * oy).sqrt() + dx.max(dy).min(0.0) - r
}

fn sd_ring(x: f64, y: f64, cx: f64, cy: f64, ro: f64, ri: f64) -> f64 {
    let d = ((x - cx) * (x - cx) + (y - cy) * (y - cy)).sqrt();
    (d - (ro + ri) * 0.5).abs() - (ro - ri) * 0.5
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

/// 静态托盘图标：金砖工牌（金色圆角工牌卡 + 顶部夹子 + 卡内计时表盘）
/// 设计意图：上班挂工牌，工牌里在计时 —— 打工人身份 + 时间=金钱
pub fn static_icon() -> Image<'static> {
    let mut buf = vec![0u8; (W * H * 4) as usize];
    for y in 0..H {
        for x in 0..W {
            let px = x as f64 + 0.5;
            let py = y as f64 + 0.5;
            let mut c = BG;

            // 夹子横条（金）
            let a = cov(sd_round_box(px, py, 32.0, 12.5, 10.0, 2.5, 2.0));
            if a > 0.0 {
                c = blend(c, GOLD, a);
            }
            // 夹子扣（金，向下凸出形成夹头）
            let a = cov(sd_circle(px, py, 32.0, 15.5, 3.5));
            if a > 0.0 {
                c = blend(c, GOLD, a);
            }
            // 金砖工牌卡（金，实心圆角矩形）
            let a = cov(sd_round_box(px, py, 32.0, 38.5, 17.0, 17.5, 8.0));
            if a > 0.0 {
                c = blend(c, GOLD, a);
            }
            // 卡内表盘：深色圆环（负空间，刻在金色上）
            let a = cov(sd_ring(px, py, 32.0, 38.0, 7.5, 5.4));
            if a > 0.0 {
                c = blend(c, BG, a);
            }
            // 12 点刻度：深色环上的金色缺口
            let ring = cov(sd_ring(px, py, 32.0, 38.0, 7.5, 5.4));
            let tick = cov(sd_circle(px, py, 32.0, 31.6, 1.4));
            let a = ring * tick;
            if a > 0.0 {
                c = blend(c, GOLD, a);
            }
            // 指针（金，指向 12 点）
            let a = cov(sd_capsule(px, py, 32.0, 33.2, 32.0, 37.8, 0.9));
            if a > 0.0 {
                c = blend(c, GOLD, a);
            }
            // 表盘中心轴点（金）
            let a = cov(sd_circle(px, py, 32.0, 38.0, 1.7));
            if a > 0.0 {
                c = blend(c, GOLD, a);
            }

            // 深色圆底裁剪
            let a_out = cov(sd_circle(px, py, CX, CY, R));
            let o = (y * W + x) as usize * 4;
            buf[o] = (c.0 * 255.0) as u8;
            buf[o + 1] = (c.1 * 255.0) as u8;
            buf[o + 2] = (c.2 * 255.0) as u8;
            buf[o + 3] = (a_out * 255.0) as u8;
        }
    }
    Image::new_owned(buf, W, H)
}
