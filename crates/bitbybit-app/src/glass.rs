//! Liquid Glass rendered natively.
//!
//! A CPU port of the fragment shader (`FS_GLASS`) of LiquidGlass by Youssef Bouane
//! (<https://github.com/ybouane/liquidglass>, MIT, `third-party/liquidglass/LICENSE`): a rounded-rectangle signed
//! distance field with a half-circle bevel height field, dual-surface refraction (IOR 1.5) of what lies behind the
//! glass, chromatic aberration along the surface normal, Fresnel, Blinn-Phong specular, rim light, an inner stroke
//! that is brighter at the top, and a two-part (outer + contact) drop shadow.
//!
//! Slint has no custom shaders, so the shader runs here and Slint shows the result as images:
//! - **Panes** (sidebar, page panes, dialogs, toasts) stay put while their content scrolls, so they are rendered
//!   whole, at their window position, refracting the backdrop exactly as the web library refracts its captured page.
//!   They are rendered on a worker thread and cached by geometry; until an image is ready the previous one (or the
//!   flat Slint glass underneath) is shown.
//! - **Frames** (cards, buttons, selectors) move with scrolling, so they get the lighting part only (bevel, Fresnel,
//!   specular, strokes, shadow), which does not depend on position: eight slices rendered once per look and scale and
//!   stretched by Slint.
//!
//! The backdrop (`Backdrop`) is also rendered here, so what the panes refract is exactly what is shown around them.
//! Like the library's default (`blurAmount: 0`) there is no separate blur pass: the backdrop is already smooth.

use crate::theme::{Rgba, Tokens};
use slint::{Rgba8Pixel, SharedPixelBuffer};
use std::sync::Arc;

type Rgb = [f32; 3];

fn rgb(c: Rgba) -> Rgb {
    [f32::from(c.0) / 255.0, f32::from(c.1) / 255.0, f32::from(c.2) / 255.0]
}

fn alpha(c: Rgba) -> f32 {
    f32::from(c.3) / 255.0
}

fn mix3(a: Rgb, b: Rgb, t: f32) -> Rgb {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// GLSL `smoothstep`, including the reversed form (`e0 > e1`) the shader relies on.
fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Signed distance to a rounded rectangle of half size `(bx, by)` and corner radius `r`, centred at the origin.
pub fn rounded_rect_sdf(px: f32, py: f32, bx: f32, by: f32, r: f32) -> f32 {
    let qx = px.abs() - bx + r;
    let qy = py.abs() - by + r;
    qx.max(qy).min(0.0) + (qx.max(0.0).hypot(qy.max(0.0))) - r
}

/// Height of the half-circle bevel at distance `d` inside the edge (`z` = bevel radius).
pub fn bevel_height(d: f32, z: f32) -> f32 {
    if d <= 0.0 {
        0.0
    } else if d >= z {
        z
    } else {
        (d * (2.0 - d / z) * z).max(0.0).sqrt()
    }
}

/// The window backdrop: a deep two-stop gradient lit by soft glows (the look's light sources). Sampled from a
/// half-resolution field so refraction lookups stay cheap.
pub struct Backdrop {
    /// Logical size of the window.
    pub width: f32,
    pub height: f32,
    cols: usize,
    rows: usize,
    field: Vec<Rgb>,
}

/// Field resolution relative to logical pixels (the backdrop has no detail finer than this).
const FIELD_SCALE: f32 = 0.5;

struct Glow {
    x: f32,
    y: f32,
    radius: f32,
    color: Rgb,
    strength: f32,
}

impl Backdrop {
    pub fn new(look: &Look, width: f32, height: f32) -> Self {
        let (width, height) = (width.max(1.0), height.max(1.0));
        let cols = ((width * FIELD_SCALE).ceil() as usize).max(2);
        let rows = ((height * FIELD_SCALE).ceil() as usize).max(2);
        let span = width.max(height);
        // Light sources: cool cyan behind the navigation (top left), royal blue on the right, a low teal wash.
        let glows = [
            Glow {
                x: 0.10 * width,
                y: 0.06 * height,
                radius: 0.62 * span,
                color: look.glow[0].0,
                strength: look.glow[0].1,
            },
            Glow {
                x: 0.92 * width,
                y: 0.18 * height,
                radius: 0.55 * span,
                color: look.glow[1].0,
                strength: look.glow[1].1,
            },
            Glow {
                x: 0.55 * width,
                y: 1.05 * height,
                radius: 0.70 * span,
                color: look.glow[2].0,
                strength: look.glow[2].1,
            },
        ];
        let mut field = Vec::with_capacity(cols * rows);
        for row in 0..rows {
            for col in 0..cols {
                let x = col as f32 / FIELD_SCALE;
                let y = row as f32 / FIELD_SCALE;
                // Gradient axis tilted like the old CSS 160deg backdrop.
                let t = ((0.82 * y / height) + (0.18 * (1.0 - x / width))).clamp(0.0, 1.0);
                let mut c = mix3(look.bg_top, look.bg_bottom, smoothstep(0.0, 0.85, t));
                for g in &glows {
                    let d = (x - g.x).hypot(y - g.y) / g.radius;
                    let f = (-(d * d) * 3.2).exp() * g.strength;
                    c = mix3(c, g.color, f);
                }
                field.push(c);
            }
        }
        Self { width, height, cols, rows, field }
    }

    /// Bilinear sample at a logical position (clamped to the window).
    pub fn sample(&self, x: f32, y: f32) -> Rgb {
        let fx = (x * FIELD_SCALE).clamp(0.0, (self.cols - 1) as f32);
        let fy = (y * FIELD_SCALE).clamp(0.0, (self.rows - 1) as f32);
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.cols - 1), (y0 + 1).min(self.rows - 1));
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let at = |c: usize, r: usize| self.field[r * self.cols + c];
        mix3(mix3(at(x0, y0), at(x1, y0), tx), mix3(at(x0, y1), at(x1, y1), tx), ty)
    }

    /// The backdrop as an image at `scale` × logical size, with ±½ LSB ordered dithering against banding.
    pub fn render(&self, scale: f32) -> SharedPixelBuffer<Rgba8Pixel> {
        let w = ((self.width * scale).round() as u32).max(1);
        let h = ((self.height * scale).round() as u32).max(1);
        let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(w, h);
        const BAYER: [f32; 16] = [0., 8., 2., 10., 12., 4., 14., 6., 3., 11., 1., 9., 15., 7., 13., 5.];
        let threads = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get).clamp(1, 4);
        let rows = (h as usize).div_ceil(threads).max(1);
        std::thread::scope(|scope| {
            for (c, chunk) in buffer.make_mut_slice().chunks_mut(rows * w as usize).enumerate() {
                scope.spawn(move || {
                    for (j, px) in chunk.iter_mut().enumerate() {
                        let i = c * rows * w as usize + j;
                        let (ix, iy) = ((i % w as usize) as u32, (i / w as usize) as u32);
                        let c = self.sample((ix as f32 + 0.5) / scale, (iy as f32 + 0.5) / scale);
                        let d = (BAYER[((iy % 4) * 4 + ix % 4) as usize] / 16.0 - 0.47) / 255.0;
                        let q = |v: f32| ((v + d).clamp(0.0, 1.0) * 255.0).round() as u8;
                        *px = Rgba8Pixel { r: q(c[0]), g: q(c[1]), b: q(c[2]), a: 255 };
                    }
                });
            }
        });
        buffer
    }
}

/// One glass material (the shader's uniforms, in logical pixels).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Material {
    pub radius: f32,
    /// Bevel depth (`zRadius`).
    pub z_radius: f32,
    pub refraction: f32,
    pub chroma: f32,
    pub edge_highlight: f32,
    pub specular: f32,
    pub fresnel: f32,
    /// Cool glass tint (0–1).
    pub tint: f32,
    /// bitBybit addition: a calm tint layer over the refracted backdrop so text on the glass stays legible
    /// (lighter at the rim, where the lensing shows).
    pub fill: Rgb,
    pub fill_alpha: f32,
    /// Darkens the backdrop seen through the glass (dialogs over the scrim).
    pub dim: f32,
    pub shadow_alpha: f32,
    pub shadow_spread: f32,
    pub shadow_offset_y: f32,
    /// Room around the panel for the shadow.
    pub pad: f32,
}

/// The colours glass and backdrop are made of, from the look's tokens.
#[derive(Debug, Clone, PartialEq)]
pub struct Look {
    pub dark: bool,
    pub bg_top: Rgb,
    pub bg_bottom: Rgb,
    pub glow: [(Rgb, f32); 3],
    pub glass: (Rgb, f32),
    pub glass_strong: (Rgb, f32),
    pub surface: (Rgb, f32),
    pub shadow: f32,
    pub scrim: f32,
}

impl Look {
    pub fn from_tokens(t: &Tokens) -> Self {
        let pair = |c: Rgba| (rgb(c), alpha(c));
        Self {
            dark: t.dark,
            bg_top: rgb(t.bg_top),
            bg_bottom: rgb(t.bg_bottom),
            glow: [pair(t.glow[0]), pair(t.glow[1]), pair(t.glow[2])],
            glass: pair(t.glass),
            glass_strong: pair(t.glass_strong),
            surface: pair(t.surface),
            shadow: alpha(t.shadow),
            scrim: alpha(t.scrim),
        }
    }

    /// Materials of the refracting panes. kind: 0 navigation, 1 page pane, 2 dialog, 3 toast.
    pub fn pane(&self, kind: i32) -> Material {
        let light = !self.dark;
        let shadow = if light { 0.45 } else { 1.0 } * self.shadow;
        let base = Material {
            radius: 18.0,
            z_radius: 16.0,
            refraction: 0.55,
            chroma: 0.06,
            edge_highlight: if light { 0.35 } else { 0.55 },
            specular: if light { 0.25 } else { 0.45 },
            fresnel: if light { 0.6 } else { 0.9 },
            tint: 0.25,
            fill: self.glass.0,
            fill_alpha: self.glass.1 * 0.62,
            dim: 0.0,
            shadow_alpha: shadow * 0.6,
            shadow_spread: 18.0,
            shadow_offset_y: 8.0,
            pad: 32.0,
        };
        match kind {
            0 => Material { radius: 20.0, z_radius: 18.0, ..base },
            2 => Material {
                radius: 22.0,
                z_radius: 20.0,
                fill: self.glass_strong.0,
                fill_alpha: self.glass_strong.1 * 0.9,
                dim: self.scrim,
                shadow_alpha: shadow * 0.9,
                shadow_spread: 28.0,
                shadow_offset_y: 14.0,
                ..base
            },
            3 => Material {
                radius: 16.0,
                z_radius: 14.0,
                fill: self.glass_strong.0,
                fill_alpha: self.glass_strong.1 * 0.82,
                shadow_alpha: shadow * 0.8,
                shadow_spread: 20.0,
                shadow_offset_y: 10.0,
                ..base
            },
            _ => Material {
                fill: self.surface.0,
                fill_alpha: (self.surface.1 * 0.75).min(0.9),
                refraction: 0.45,
                shadow_alpha: shadow * 0.45,
                ..base
            },
        }
    }

    /// Materials of the lighting frames. kind: 0 card, 1 control (32 px pill), 2 selector thumb (28 px pill),
    /// 3 large control (40 px pill).
    pub fn frame(&self, kind: i32) -> Material {
        let light = !self.dark;
        let shadow = if light { 0.5 } else { 1.0 } * self.shadow;
        let base = Material {
            radius: 14.0,
            z_radius: 10.0,
            refraction: 0.0,
            chroma: 0.0,
            edge_highlight: if light { 0.25 } else { 0.4 },
            specular: if light { 0.2 } else { 0.32 },
            fresnel: if light { 0.45 } else { 0.7 },
            tint: 0.0,
            fill: [0.0; 3],
            fill_alpha: 0.0,
            dim: 0.0,
            shadow_alpha: shadow * 0.35,
            shadow_spread: 10.0,
            shadow_offset_y: 3.0,
            pad: 16.0,
        };
        match kind {
            1 => Material {
                radius: 16.0,
                z_radius: 9.0,
                specular: base.specular * 1.4,
                shadow_alpha: shadow * 0.4,
                shadow_spread: 6.0,
                shadow_offset_y: 2.0,
                pad: 10.0,
                ..base
            },
            2 => Material {
                radius: 14.0,
                z_radius: 8.0,
                specular: base.specular * 1.5,
                shadow_alpha: shadow * 0.45,
                shadow_spread: 5.0,
                shadow_offset_y: 1.5,
                pad: 8.0,
                ..base
            },
            3 => Material {
                radius: 20.0,
                z_radius: 11.0,
                specular: base.specular * 1.4,
                shadow_alpha: shadow * 0.5,
                shadow_spread: 10.0,
                shadow_offset_y: 4.0,
                pad: 14.0,
                ..base
            },
            _ => base,
        }
    }
}

/// Key light from the top left, a fill from below, and a soft overhead dome (screen space, y down).
const LIGHTS: [([f32; 3], f32, f32); 3] =
    [([-0.4, -0.9, 1.0], 28.0, 0.7), ([0.3, 0.6, 1.0], 24.0, 0.12), ([0.0, -0.2, 1.0], 6.0, 0.08)];

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-6);
    [v[0] / l, v[1] / l, v[2] / l]
}

/// What lies behind the glass: the backdrop at logical coordinates, or nothing (lighting-only frames).
pub enum Behind<'a> {
    Backdrop { backdrop: &'a Backdrop, origin_x: f32, origin_y: f32 },
    Nothing,
}

/// Renders one glass rectangle of logical size `w`×`h` at `scale`, with `m.pad` of room for the shadow on every
/// side (straight, not premultiplied, RGBA).
pub fn render(m: &Material, behind: &Behind<'_>, w: f32, h: f32, scale: f32) -> SharedPixelBuffer<Rgba8Pixel> {
    let s = scale;
    let (pw, ph) = (w * s, h * s);
    let pad = (m.pad * s).ceil();
    let out_w = ((pw + 2.0 * pad).round() as u32).max(1);
    let out_h = ((ph + 2.0 * pad).round() as u32).max(1);
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(out_w, out_h);
    let (hx, hy) = (pw * 0.5, ph * 0.5);
    let r = (m.radius * s).min(hx).min(hy);
    let z = (m.z_radius * s).max(1.0);
    // Blinn-Phong half vectors (view straight on).
    let lights: Vec<([f32; 3], f32, f32)> = LIGHTS
        .iter()
        .map(|(l, p, k)| {
            let l = normalize(*l);
            (normalize([l[0], l[1], l[2] + 1.0]), *p, *k)
        })
        .collect();
    let e = s.max(1.0);
    let max_d = hx.min(hy);
    let rim_width = match behind {
        Behind::Nothing => (z * 1.2).min(max_d * 0.35),
        Behind::Backdrop { .. } => max_d * 0.35,
    };
    let stroke = 1.25 * s;
    let glow_width = 5.0 * s;
    let spread = (m.shadow_spread * s).max(1.0);
    let off_y = m.shadow_offset_y * s;
    let refr_pow = 1.0 - 1.0 / 1.5;
    // Beyond the bevel the surface is flat: its specular is a constant.
    let flat_spec: f32 = lights.iter().map(|(hv, power, k)| hv[2].max(0.0).powf(*power) * k).sum::<f32>() * m.specular;
    let shade = |ix: u32, iy: u32| -> Rgba8Pixel {
        let lx = ix as f32 + 0.5 - pad - hx;
        let ly = iy as f32 + 0.5 - pad - hy;
        let sdf = rounded_rect_sdf(lx, ly, hx, hy, r);
        // Shadow, also under the anti-aliased edge so there is no seam.
        let shadow = if sdf > -1.5 && m.shadow_alpha > 0.0 {
            let d = (rounded_rect_sdf(lx, ly - off_y, hx, hy, r) - 1.0).max(0.0);
            let outer = (-(d * d) / (spread * spread)).exp() * 0.65;
            let contact = (-d * 0.08 / (spread * 0.04).max(0.01)).exp() * 0.35;
            ((outer + contact) * m.shadow_alpha).min(1.0)
        } else {
            0.0
        };
        if sdf > 0.5 {
            return Rgba8Pixel { r: 0, g: 0, b: 0, a: (shadow * 255.0).round() as u8 };
        }
        let mask = 1.0 - smoothstep(-1.5, 0.5, sdf);
        let inside = -sdf;
        let edge = smoothstep(rim_width, 0.0, inside);
        if inside > z + e + stroke + 1.0 && inside > glow_width {
            // Flat interior: no bevel normal, no dispersion, no strokes (most of the pixels).
            let light = flat_spec + edge * m.edge_highlight * 0.22;
            return match behind {
                Behind::Backdrop { backdrop, origin_x, origin_y } => {
                    let rx = (-lx / hx.max(1.0)) * m.refraction * 4.0 * s;
                    let ry = (-ly / hy.max(1.0)) * m.refraction * 4.0 * s;
                    let mut c = backdrop.sample(
                        (ix as f32 + 0.5 - pad + rx) / s + origin_x,
                        (iy as f32 + 0.5 - pad + ry) / s + origin_y,
                    );
                    c = mix3(c, [0.0; 3], m.dim);
                    c = mix3(c, [c[0] * 0.92, c[1] * 0.95, c[2] * 1.05], m.tint);
                    c = [c[0] * 1.06, c[1] * 1.06, c[2] * 1.06];
                    c = mix3(c, m.fill, m.fill_alpha);
                    let q = |v: f32| ((v + light).clamp(0.0, 1.0) * 255.0).round() as u8;
                    Rgba8Pixel { r: q(c[0]), g: q(c[1]), b: q(c[2]), a: 255 }
                }
                Behind::Nothing => {
                    Rgba8Pixel { r: 255, g: 255, b: 255, a: (light.clamp(0.0, 1.0) * 255.0).round() as u8 }
                }
            };
        }
        // Surface normal from the bevel height field (flat beyond the bevel).
        let (gx, gy, hc) = if inside > z + e {
            (0.0, 0.0, z)
        } else {
            let h = |dx: f32, dy: f32| bevel_height(-rounded_rect_sdf(lx + dx, ly + dy, hx, hy, r), z);
            ((h(e, 0.0) - h(-e, 0.0)) / (2.0 * e), (h(0.0, e) - h(0.0, -e)) / (2.0 * e), bevel_height(inside, z))
        };
        let n = normalize([-gx, -gy, 1.0]);
        let depth = smoothstep(0.0, z, inside);
        let mut col = match behind {
            Behind::Backdrop { backdrop, origin_x, origin_y } => {
                // Dual-surface refraction plus a gentle pull towards the centre (magnification).
                let thick = hc * 2.0 / (z * 2.0).max(1.0);
                let k = refr_pow * (2.0 + thick * 0.5) * m.refraction * 30.0;
                let rx = gx * k + (-lx / hx.max(1.0)) * m.refraction * 4.0 * s * depth;
                let ry = gy * k + (-ly / hy.max(1.0)) * m.refraction * 4.0 * s * depth;
                let ca = m.chroma * 18.0 * s * (edge * 0.7 + 0.3) * 2.0;
                let (cx, cy) = (n[0] * ca, n[1] * ca);
                let bx = (ix as f32 + 0.5 - pad + rx) / s + origin_x;
                let by = (iy as f32 + 0.5 - pad + ry) / s + origin_y;
                let sample = |dx: f32, dy: f32| backdrop.sample(bx + dx / s, by + dy / s);
                let mut c = [sample(cx, cy)[0], sample(0.0, 0.0)[1], sample(-cx, -cy)[2]];
                c = mix3(c, [0.0; 3], m.dim);
                c = mix3(c, [c[0] * 0.92, c[1] * 0.95, c[2] * 1.05], m.tint);
                let lift = 1.0 + 0.06 * depth;
                c = [c[0] * lift, c[1] * lift, c[2] * lift];
                // Legibility layer, lighter at the rim so the lensing stays visible.
                mix3(c, m.fill, m.fill_alpha * (0.35 + 0.65 * depth))
            }
            Behind::Nothing => [0.0; 3],
        };
        let fres = (1.0 - n[2].abs()).powi(4) * m.fresnel;
        let mut spec = 0.0;
        for (hv, power, k) in &lights {
            spec += (n[0] * hv[0] + n[1] * hv[1] + n[2] * hv[2]).max(0.0).powf(*power) * k;
        }
        spec *= m.specular;
        let mut inner = smoothstep(-stroke - 1.0, -stroke, sdf) * (1.0 - smoothstep(-1.0, 0.0, sdf));
        inner *= 0.4 + 0.6 * (0.5 + 0.5 * (-ly / hy.max(1.0)));
        let rim = edge * m.edge_highlight * 0.22;
        let glow = smoothstep(glow_width, 0.0, inside) * m.edge_highlight * 0.15;
        let env = (n[1] * 0.5 + 0.5) * fres * 0.08;
        let light = spec + rim + glow + inner * m.edge_highlight * 0.55 + env;
        match behind {
            Behind::Backdrop { .. } => {
                col = [col[0] + light, col[1] + light, col[2] + light];
                col = mix3(col, [1.0; 3], fres * 0.2);
                // Glass over its own shadow at the anti-aliased edge.
                let a = mask;
                let total = a + shadow * (1.0 - a);
                let q = |v: f32| ((v * a / total.max(1e-4)).clamp(0.0, 1.0) * 255.0).round() as u8;
                Rgba8Pixel { r: q(col[0]), g: q(col[1]), b: q(col[2]), a: (total * 255.0).round() as u8 }
            }
            Behind::Nothing => {
                // Lighting only: white light whose strength is the alpha, over the shadow.
                let la = ((light + fres * 0.2) * mask).clamp(0.0, 1.0);
                let total = la + shadow * (1.0 - la);
                let v = ((la / total.max(1e-4)) * 255.0).round() as u8;
                Rgba8Pixel { r: v, g: v, b: v, a: (total.min(1.0) * 255.0).round() as u8 }
            }
        }
    };
    // Rows in parallel (the worker is otherwise idle while a pane renders).
    let threads = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get).clamp(1, 4);
    let rows = (out_h as usize).div_ceil(threads).max(1);
    let row_len = out_w as usize;
    let shade = &shade;
    std::thread::scope(|scope| {
        for (c, chunk) in buffer.make_mut_slice().chunks_mut(rows * row_len).enumerate() {
            scope.spawn(move || {
                for (j, px) in chunk.iter_mut().enumerate() {
                    let i = c * rows * row_len + j;
                    *px = shade((i % row_len) as u32, (i / row_len) as u32);
                }
            });
        }
    });
    buffer
}

/// The eight slices of a lighting frame: corners at their own size, edges one pixel long and stretched.
pub struct FrameSlices {
    /// Logical size of a corner slice (shadow room + rounded part).
    pub corner: f32,
    pub pad: f32,
    pub tl: SharedPixelBuffer<Rgba8Pixel>,
    pub tr: SharedPixelBuffer<Rgba8Pixel>,
    pub bl: SharedPixelBuffer<Rgba8Pixel>,
    pub br: SharedPixelBuffer<Rgba8Pixel>,
    pub top: SharedPixelBuffer<Rgba8Pixel>,
    pub bottom: SharedPixelBuffer<Rgba8Pixel>,
    pub left: SharedPixelBuffer<Rgba8Pixel>,
    pub right: SharedPixelBuffer<Rgba8Pixel>,
}

fn crop(src: &SharedPixelBuffer<Rgba8Pixel>, x: u32, y: u32, w: u32, h: u32) -> SharedPixelBuffer<Rgba8Pixel> {
    let (w, h) = (w.max(1), h.max(1));
    let mut out = SharedPixelBuffer::<Rgba8Pixel>::new(w, h);
    let sw = src.width();
    let s = src.as_slice();
    let o = out.make_mut_slice();
    for row in 0..h {
        for col in 0..w {
            let sx = (x + col).min(sw - 1);
            let sy = (y + row).min(src.height() - 1);
            o[(row * w + col) as usize] = s[(sy * sw + sx) as usize];
        }
    }
    out
}

/// Renders a frame from a reference rectangle and cuts it into slices. `pill`: the height is exactly twice the
/// radius (no vertical edge).
pub fn frame_slices(m: &Material, scale: f32, pill: bool) -> FrameSlices {
    let core = m.radius.max(m.z_radius).ceil();
    let corner = m.pad.ceil() + core;
    let straight = 64.0;
    let w = 2.0 * core + straight;
    let h = if pill { 2.0 * m.radius } else { 2.0 * core + straight };
    let full = render(m, &Behind::Nothing, w, h, scale);
    let c = (corner * scale).round() as u32;
    let (fw, fh) = (full.width(), full.height());
    let mid_x = fw / 2;
    let vertical = fh.saturating_sub(2 * c);
    let edge_h = if pill { 0 } else { vertical };
    FrameSlices {
        corner,
        pad: m.pad.ceil(),
        tl: crop(&full, 0, 0, c, c.min(fh / 2 + 1)),
        tr: crop(&full, fw - c, 0, c, c.min(fh / 2 + 1)),
        bl: crop(&full, 0, fh - c.min(fh / 2 + 1), c, c.min(fh / 2 + 1)),
        br: crop(&full, fw - c, fh - c.min(fh / 2 + 1), c, c.min(fh / 2 + 1)),
        top: crop(&full, mid_x, 0, 1, c.min(fh / 2 + 1)),
        bottom: crop(&full, mid_x, fh - c.min(fh / 2 + 1), 1, c.min(fh / 2 + 1)),
        left: crop(&full, 0, c, c, edge_h),
        right: crop(&full, fw - c, c, c, edge_h),
    }
}

/// A pane request handed to the worker.
pub struct Job {
    pub id: String,
    pub kind: i32,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub window_w: f32,
    pub window_h: f32,
    pub scale: f32,
    pub look: Arc<Look>,
    pub generation: u64,
}

impl Job {
    /// Cache key (positions rounded to half pixels).
    pub fn key(&self) -> String {
        let q = |v: f32| (v * 2.0).round() as i64;
        format!(
            "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
            self.id,
            self.kind,
            q(self.x),
            q(self.y),
            q(self.w),
            q(self.h),
            q(self.window_w),
            q(self.window_h),
            q(self.scale * 8.0),
            self.generation
        )
    }
}

/// Kind of the backdrop job (not a pane material).
const BACKDROP: i32 = 99;

/// The worker: renders the latest request of each pane (older ones for the same pane are dropped) and hands the
/// pixels to the UI thread.
fn worker(rx: std::sync::mpsc::Receiver<Job>, weak: slint::Weak<crate::ui::AppWindow>) {
    let mut field: Option<(String, Arc<Backdrop>)> = None;
    while let Ok(first) = rx.recv() {
        let mut latest = std::collections::HashMap::new();
        latest.insert(first.id.clone(), first);
        while let Ok(job) = rx.try_recv() {
            latest.insert(job.id.clone(), job);
        }
        for job in latest.into_values() {
            let field_key = format!("{}x{}|{}", job.window_w.round(), job.window_h.round(), job.generation);
            let backdrop = match &field {
                Some((k, b)) if *k == field_key => b.clone(),
                _ => {
                    let b = Arc::new(Backdrop::new(&job.look, job.window_w, job.window_h));
                    field = Some((field_key, b.clone()));
                    b
                }
            };
            let buffer = if job.kind == BACKDROP {
                // Logical resolution: the backdrop is smooth, Slint's upscaling loses nothing.
                backdrop.render(job.scale.min(1.0))
            } else {
                let behind = Behind::Backdrop { backdrop: &backdrop, origin_x: job.x, origin_y: job.y };
                render(&job.look.pane(job.kind), &behind, job.w, job.h, job.scale)
            };
            let (id, key, weak) = (job.id.clone(), job.key(), weak.clone());
            let _ = slint::invoke_from_event_loop(move || deliver(&weak, &id, &key, buffer));
        }
    }
}

struct Engine {
    look: Arc<Look>,
    generation: u64,
    enabled: bool,
    /// Scale the frames were rendered at.
    frame_scale: f32,
    cache: std::collections::HashMap<String, slint::Image>,
    /// The last image of each pane: shown (stretched) while the exact one renders, so resizing never flickers.
    last: std::collections::HashMap<String, slint::Image>,
    /// The key last sent to the worker for each pane.
    pending: std::collections::HashMap<String, String>,
    tx: Option<std::sync::mpsc::Sender<Job>>,
}

thread_local! {
    static ENGINE: std::cell::RefCell<Option<Engine>> = const { std::cell::RefCell::new(None) };
}

fn deliver(weak: &slint::Weak<crate::ui::AppWindow>, id: &str, key: &str, buffer: SharedPixelBuffer<Rgba8Pixel>) {
    let Some(ui) = weak.upgrade() else { return };
    let current = ENGINE.with(|e| {
        let mut e = e.borrow_mut();
        let Some(e) = e.as_mut() else { return false };
        // A result for an older look is dropped.
        if !key.ends_with(&format!("|{}", e.generation)) {
            return false;
        }
        let image = slint::Image::from_rgba8(buffer);
        if e.cache.len() > 48 {
            e.cache.clear();
        }
        e.cache.insert(key.to_owned(), image.clone());
        e.last.insert(id.to_owned(), image);
        if e.pending.get(id).is_some_and(|k| k == key) {
            e.pending.remove(id);
        }
        true
    });
    if current {
        use slint::ComponentHandle;
        let g = ui.global::<crate::ui::GlassEngine>();
        g.set_revision(g.get_revision().wrapping_add(1));
    }
}

fn frame_images(f: FrameSlices) -> crate::ui::GlassFrameImages {
    let img = slint::Image::from_rgba8;
    crate::ui::GlassFrameImages {
        tl: img(f.tl),
        tr: img(f.tr),
        bl: img(f.bl),
        br: img(f.br),
        t: img(f.top),
        b: img(f.bottom),
        l: img(f.left),
        r: img(f.right),
        corner: f.corner,
        pad: f.pad,
    }
}

/// Renders the lighting frames for the current look and scale (small: a few hundred pixels square).
fn set_frames(ui: &crate::ui::AppWindow, look: &Look, scale: f32) {
    use slint::ComponentHandle;
    let g = ui.global::<crate::ui::GlassEngine>();
    g.set_card(frame_images(frame_slices(&look.frame(0), scale, false)));
    g.set_control(frame_images(frame_slices(&look.frame(1), scale, true)));
    g.set_thumb(frame_images(frame_slices(&look.frame(2), scale, true)));
    g.set_large(frame_images(frame_slices(&look.frame(3), scale, true)));
}

/// Starts the glass engine. `enabled` false (Windows "Transparency effects" off, or `BITBYBIT_GLASS=0`) keeps the
/// flat Slint glass everywhere.
pub fn install(ui: &crate::ui::AppWindow, tokens: &Tokens, enabled: bool) {
    use slint::ComponentHandle;
    let look = Arc::new(Look::from_tokens(tokens));
    let (tx, rx) = std::sync::mpsc::channel::<Job>();
    let weak = ui.as_weak();
    let tx = std::thread::Builder::new()
        .name("glass".into())
        .spawn(move || worker(rx, weak))
        .map(|_| tx)
        .map_err(|e| log::warn!("Glass rendering is off: no worker thread ({e})"))
        .ok();
    let enabled = enabled && tx.is_some();
    let scale = ui.window().scale_factor();
    ENGINE.with(|e| {
        *e.borrow_mut() = Some(Engine {
            look: look.clone(),
            generation: 1,
            enabled,
            frame_scale: scale,
            cache: Default::default(),
            last: Default::default(),
            pending: Default::default(),
            tx,
        });
    });
    let g = ui.global::<crate::ui::GlassEngine>();
    g.set_enabled(enabled);
    if !enabled {
        return;
    }
    set_frames(ui, &look, scale);
    let weak = ui.as_weak();
    g.on_pane(move |id, kind, x, y, w, h, _revision| request(&weak, id.as_str(), kind, x, y, w, h));
    let weak = ui.as_weak();
    g.on_backdrop(move |w, h, _revision| request(&weak, "backdrop", BACKDROP, 0.0, 0.0, w, h));
}

/// Answers a pane (or backdrop) binding: the exact image when cached, else the pane's previous one (or none: the
/// flat glass shows) while the worker renders it.
fn request(
    weak: &slint::Weak<crate::ui::AppWindow>,
    id: &str,
    kind: i32,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
) -> slint::Image {
    use slint::ComponentHandle;
    let Some(ui) = weak.upgrade() else { return slint::Image::default() };
    if w < 2.0 || h < 2.0 {
        return slint::Image::default();
    }
    let window = ui.window();
    let scale = window.scale_factor();
    let size = window.size().to_logical(scale);
    ENGINE.with(|e| {
        let mut e = e.borrow_mut();
        let Some(e) = e.as_mut() else { return slint::Image::default() };
        if (e.frame_scale - scale).abs() > 0.01 {
            // The window moved to a screen with another scale: frames again, once this binding has finished.
            e.frame_scale = scale;
            let (weak, look) = (weak.clone(), e.look.clone());
            slint::Timer::single_shot(std::time::Duration::ZERO, move || {
                if let Some(ui) = weak.upgrade() {
                    set_frames(&ui, &look, scale);
                }
            });
        }
        let job = Job {
            id: id.to_owned(),
            kind,
            x,
            y,
            w,
            h,
            window_w: size.width,
            window_h: size.height,
            scale,
            look: e.look.clone(),
            generation: e.generation,
        };
        let key = job.key();
        if let Some(image) = e.cache.get(&key) {
            return image.clone();
        }
        if e.pending.get(id) != Some(&key)
            && let Some(tx) = &e.tx
            && tx.send(job).is_ok()
        {
            e.pending.insert(id.to_owned(), key);
        }
        e.last.get(id).cloned().unwrap_or_default()
    })
}

/// A new look: frames now, panes and backdrop again (the old images stay up until the new ones arrive).
pub fn set_look(ui: &crate::ui::AppWindow, tokens: &Tokens) {
    use slint::ComponentHandle;
    let look = Arc::new(Look::from_tokens(tokens));
    let scale = ui.window().scale_factor();
    let enabled = ENGINE.with(|e| {
        let mut e = e.borrow_mut();
        let Some(e) = e.as_mut() else { return false };
        if *e.look == *look {
            return false;
        }
        e.look = look.clone();
        e.generation += 1;
        e.cache.clear();
        e.pending.clear();
        e.frame_scale = scale;
        e.enabled
    });
    if enabled {
        set_frames(ui, &look, scale);
        let g = ui.global::<crate::ui::GlassEngine>();
        g.set_revision(g.get_revision().wrapping_add(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn look() -> Look {
        Look::from_tokens(&crate::theme::dark())
    }

    #[test]
    fn rounded_rect_distance_is_negative_inside_zero_on_the_edge_and_positive_outside() {
        assert!(rounded_rect_sdf(0.0, 0.0, 50.0, 30.0, 10.0) < -29.0);
        assert!(rounded_rect_sdf(50.0, 0.0, 50.0, 30.0, 10.0).abs() < 1e-4);
        assert!(rounded_rect_sdf(60.0, 0.0, 50.0, 30.0, 10.0) > 9.9);
        // The corner is rounded: the box corner is outside by r·(√2 − 1).
        let corner = rounded_rect_sdf(50.0, 30.0, 50.0, 30.0, 10.0);
        assert!((corner - 10.0 * (2f32.sqrt() - 1.0)).abs() < 1e-3, "{corner}");
    }

    #[test]
    fn the_bevel_rises_from_the_edge_to_its_depth() {
        assert_eq!(bevel_height(0.0, 10.0), 0.0);
        assert!(bevel_height(2.0, 10.0) < bevel_height(5.0, 10.0));
        assert_eq!(bevel_height(12.0, 10.0), 10.0);
        assert!((bevel_height(10.0, 10.0) - 10.0).abs() < 1e-4);
    }

    #[test]
    fn a_pane_is_opaque_inside_shadowed_below_and_clear_far_outside() {
        let l = look();
        let backdrop = Backdrop::new(&l, 400.0, 300.0);
        let m = l.pane(1);
        let img =
            render(&m, &Behind::Backdrop { backdrop: &backdrop, origin_x: 50.0, origin_y: 40.0 }, 200.0, 120.0, 1.0);
        let pad = m.pad as u32;
        assert_eq!((img.width(), img.height()), (200 + 2 * pad, 120 + 2 * pad));
        let at = |x: u32, y: u32| img.as_slice()[(y * img.width() + x) as usize];
        assert_eq!(at(pad + 100, pad + 60).a, 255, "inside");
        assert!(at(0, 0).a <= 2, "far corner of the shadow room");
        assert!(at(pad + 100, pad + 120 + 4).a > at(pad + 100, pad - 4).a, "the shadow falls below");
    }

    #[test]
    fn the_rim_catches_more_light_than_the_middle() {
        let l = look();
        let m = l.frame(0);
        let img = render(&m, &Behind::Nothing, 120.0, 80.0, 1.0);
        let pad = m.pad as u32;
        let at = |x: u32, y: u32| img.as_slice()[(y * img.width() + x) as usize];
        let middle = at(pad + 60, pad + 40);
        let top_rim = at(pad + 60, pad + 1);
        assert!(top_rim.a > middle.a + 20, "rim {} vs middle {}", top_rim.a, middle.a);
        assert!(at(pad + 60, pad + 1).a > at(pad + 60, pad + 78).a, "the top stroke is brighter than the bottom");
    }

    #[test]
    fn frame_slices_tile_the_reference_rectangle() {
        let l = look();
        for (kind, pill) in [(0, false), (1, true)] {
            let m = l.frame(kind);
            let f = frame_slices(&m, 2.0, pill);
            let c = (f.corner * 2.0).round() as u32;
            assert_eq!(f.tl.width(), c);
            assert_eq!(f.top.width(), 1);
            assert_eq!(f.left.width(), c);
            if pill {
                assert_eq!(f.tl.height() * 2, (2.0 * (m.radius + m.pad.ceil()) * 2.0) as u32);
            }
        }
    }

    #[test]
    fn the_backdrop_is_deterministic_and_within_range() {
        let l = look();
        let a = Backdrop::new(&l, 640.0, 400.0);
        let b = Backdrop::new(&l, 640.0, 400.0);
        assert_eq!(a.sample(100.0, 100.0), b.sample(100.0, 100.0));
        for (x, y) in [(0.0, 0.0), (639.0, 399.0), (-50.0, 900.0)] {
            assert!(a.sample(x, y).iter().all(|v| (0.0..=1.0).contains(v)));
        }
    }
}
