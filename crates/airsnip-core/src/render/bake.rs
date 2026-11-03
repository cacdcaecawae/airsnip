// SPDX-License-Identifier: GPL-3.0-only
//! 导出烘焙（OUT-1/OUT-2）：把标注对象栅格化进裁剪结果图，
//! 以及聚光（Glow）的「整图暗化 + 区域恢复原图亮度」效果。

use tiny_skia::{Pixmap, Transform};

use crate::editor::{Kind, Object, Point};

use super::mask;

/// 导出烘焙（OUT-1/OUT-2）：把标注对象栅格化进裁剪结果图。
/// `origin` 是裁剪区域在覆盖层坐标系中的左上角；对象坐标减去它即为裁剪图坐标。
/// 存在聚光（Glow）对象时：整图先暗化，再把各 Glow 区域恢复为原图亮度（聚光效果），
/// 与屏幕显示一致；其余对象照常绘制。
pub fn bake_objects(pixmap: &mut Pixmap, objects: &[Object], origin: (i32, i32)) {
    let transform = Transform::from_translate(-origin.0 as f32, -origin.1 as f32);
    if objects.iter().any(|o| matches!(o.kind, Kind::Glow { .. })) {
        // 备份原图，暗化后用于恢复 Glow 区域
        let backup = pixmap.clone();
        dim_all(pixmap, 255 - 120);
        for obj in objects {
            if let Kind::Glow { a, b, corner } = &obj.kind {
                restore_rect(pixmap, &backup, *a, *b, *corner, origin);
            }
        }
    }
    for obj in objects {
        if matches!(obj.kind, Kind::Glow { .. }) {
            continue; // 已在上面处理，不重复绘制
        }
        super::draw_object_with(pixmap, obj, transform);
    }
}

/// 全图暗化：RGB 通道乘 `k/256`（与覆盖层暗化同系数，M3）。
fn dim_all(pixmap: &mut Pixmap, k: u32) {
    for px in pixmap.data_mut().chunks_exact_mut(4) {
        px[0] = ((px[0] as u32 * k) >> 8) as u8;
        px[1] = ((px[1] as u32 * k) >> 8) as u8;
        px[2] = ((px[2] as u32 * k) >> 8) as u8;
    }
}

/// 从备份图把 Glow 区域（对角两点，经 origin 平移）恢复到目标图。
/// `corner > 0` 时四角按圆角遮罩恢复，与屏幕显示一致；圆角按**完整**（未裁剪）对象矩形计算。
fn restore_rect(dst: &mut Pixmap, src: &Pixmap, a: Point, b: Point, corner: f32, origin: (i32, i32)) {
    let (x0f, y0f) = (a.x.min(b.x), a.y.min(b.y));
    let (x1f, y1f) = (a.x.max(b.x), a.y.max(b.y));
    let (w, h) = (dst.width() as i32, dst.height() as i32);
    let x0 = (x0f.floor() as i32 - origin.0).clamp(0, w);
    let y0 = (y0f.floor() as i32 - origin.1).clamp(0, h);
    let x1 = (x1f.ceil() as i32 - origin.0).clamp(0, w);
    let y1 = (y1f.ceil() as i32 - origin.1).clamp(0, h);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let region = (x0, y0, x1, y1);
    // 圆角矩形在裁剪图坐标里的位置（先平移再判圆角）
    let rect = (
        x0f - origin.0 as f32,
        y0f - origin.1 as f32,
        x1f - origin.0 as f32,
        y1f - origin.1 as f32,
    );
    let r = mask::clamp_radius(rect, corner);
    if r < 0.5 {
        for row in y0..y1 {
            let off = (row as usize * dst.width() as usize + x0 as usize) * 4;
            let len = ((x1 - x0) * 4) as usize;
            dst.data_mut()[off..off + len].copy_from_slice(&src.data()[off..off + len]);
        }
        return;
    }
    let buf = mask::copy_region(src, region);
    mask::blend_compact(dst, &buf, region, rect, r);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::Style;

    fn canvas(w: u32, h: u32) -> Pixmap {
        let mut p = Pixmap::new(w, h).unwrap();
        p.fill(tiny_skia::Color::from_rgba8(255, 255, 255, 255));
        p
    }

    /// 聚光圆角：烘焙后四角保持暗化，中心恢复原图亮度，且裁剪原点正确平移。
    #[test]
    fn glow_corner_stays_dim_in_bake() {
        let mut p = canvas(60, 60);
        let obj = Object {
            kind: Kind::Glow {
                a: Point::new(100.0, 100.0),
                b: Point::new(140.0, 140.0),
                corner: 15.0,
            },
            style: Style::new([0, 0, 0, 255], 2.0),
        };
        // 裁剪原点 (80,80) → 聚光区落在裁剪图 (20,20)-(60,60)
        bake_objects(&mut p, &[obj], (80, 80));

        let at = |p: &Pixmap, x: u32, y: u32| p.data()[((y * p.width() + x) * 4) as usize];
        assert!(at(&p, 40, 40) > 200, "中心应恢复原图亮度（白）");
        assert!(at(&p, 20, 20) < 150, "左上圆角处应仍是暗化值");
        assert!(at(&p, 40, 20) > 200, "上直边应恢复原图亮度");
    }
}
