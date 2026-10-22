//! Software rasterizer for display lists.
//!
//! Draws rectangles, anti-aliased system TrueType text (with a 5x7 ASCII bitmap
//! fallback), into a top-down RGBA framebuffer.

#[cfg(windows)]
mod font;

use flux_gpu::{GpuColor, sample_linear_gradient, sdf_box_shadow_alpha, sdf_rounded_box};
use flux_image::Image;
use flux_paint::{DisplayItem, DisplayList};
pub use flux_style::Color;
use flux_ttf::TtfFont;

const PAPER: Color = Color {
    r: 255,
    g: 255,
    b: 255,
    a: 255,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Framebuffer {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    clips: Vec<(i32, i32, i32, i32)>,
}

impl Framebuffer {
    pub fn new(width: u32, height: u32, clear: Color) -> Self {
        let count = (width as usize).saturating_mul(height as usize);
        let mut pixels = Vec::with_capacity(count * 4);
        for _ in 0..count {
            pixels.extend_from_slice(&[clear.r, clear.g, clear.b, clear.a]);
        }
        Self {
            width,
            height,
            pixels,
            clips: Vec::new(),
        }
    }

    pub fn push_clip(&mut self, x: f32, y: f32, width: f32, height: f32) {
        let mut x0 = (x.floor() as i32).max(0);
        let mut y0 = (y.floor() as i32).max(0);
        let mut x1 = ((x + width.max(0.0)).ceil() as i32).min(self.width as i32);
        let mut y1 = ((y + height.max(0.0)).ceil() as i32).min(self.height as i32);
        if let Some(&(px0, py0, px1, py1)) = self.clips.last() {
            x0 = x0.max(px0);
            y0 = y0.max(py0);
            x1 = x1.min(px1);
            y1 = y1.min(py1);
        }
        self.clips.push((x0, y0, x1, y1));
    }

    pub fn pop_clip(&mut self) {
        self.clips.pop();
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn pixel(&self, x: u32, y: u32) -> Option<Color> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let offset = ((y as usize) * (self.width as usize) + (x as usize)) * 4;
        Some(Color {
            r: self.pixels[offset],
            g: self.pixels[offset + 1],
            b: self.pixels[offset + 2],
            a: self.pixels[offset + 3],
        })
    }

    pub fn set_pixel(&mut self, x: i32, y: i32, color: Color) {
        if x < 0 || y < 0 || color.a == 0 {
            return;
        }
        if let Some(&(cx0, cy0, cx1, cy1)) = self.clips.last() {
            if x < cx0 || x >= cx1 || y < cy0 || y >= cy1 {
                return;
            }
        }
        let (x, y) = (x as u32, y as u32);
        if x >= self.width || y >= self.height {
            return;
        }
        let offset = ((y as usize) * (self.width as usize) + (x as usize)) * 4;
        if color.a == 255 {
            self.pixels[offset] = color.r;
            self.pixels[offset + 1] = color.g;
            self.pixels[offset + 2] = color.b;
            self.pixels[offset + 3] = 255;
            return;
        }
        let alpha = color.a as u16;
        let inv = 255 - alpha;
        self.pixels[offset] =
            ((color.r as u16 * alpha + self.pixels[offset] as u16 * inv) / 255) as u8;
        self.pixels[offset + 1] =
            ((color.g as u16 * alpha + self.pixels[offset + 1] as u16 * inv) / 255) as u8;
        self.pixels[offset + 2] =
            ((color.b as u16 * alpha + self.pixels[offset + 2] as u16 * inv) / 255) as u8;
        self.pixels[offset + 3] = 255;
    }

    pub fn fill_rect(&mut self, x: f32, y: f32, width: f32, height: f32, color: Color) {
        if width <= 0.0 || height <= 0.0 || color.a == 0 {
            return;
        }
        let x0 = (x.floor() as i32).max(0);
        let y0 = (y.floor() as i32).max(0);
        let x1 = ((x + width).ceil() as i32).min(self.width as i32);
        let y1 = ((y + height).ceil() as i32).min(self.height as i32);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        for py in y0..y1 {
            for px in x0..x1 {
                self.set_pixel(px, py, color);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn fill_rounded_rect(
        &mut self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        radius: f32,
        border_width: f32,
        fill_color: Color,
        border_color: Color,
    ) {
        if width <= 0.0 || height <= 0.0 {
            return;
        }
        let x0 = (x.floor() as i32).max(0);
        let y0 = (y.floor() as i32).max(0);
        let x1 = ((x + width).ceil() as i32).min(self.width as i32);
        let y1 = ((y + height).ceil() as i32).min(self.height as i32);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let half_w = width * 0.5;
        let half_h = height * 0.5;
        let cx = x + half_w;
        let cy = y + half_h;

        for py in y0..y1 {
            for px in x0..x1 {
                let sx = px as f32 + 0.5;
                let sy = py as f32 + 0.5;
                let dist = sdf_rounded_box(sx - cx, sy - cy, half_w, half_h, radius);
                let outer_alpha = (0.5 - dist).clamp(0.0, 1.0);
                if outer_alpha <= 0.0 {
                    continue;
                }
                let mut c = fill_color;
                if border_width > 0.0 {
                    let inner_dist = dist + border_width;
                    let border_mix = (0.5 + inner_dist).clamp(0.0, 1.0);
                    let inv = 1.0 - border_mix;
                    c = Color {
                        r: ((c.r as f32) * inv + (border_color.r as f32) * border_mix).round()
                            as u8,
                        g: ((c.g as f32) * inv + (border_color.g as f32) * border_mix).round()
                            as u8,
                        b: ((c.b as f32) * inv + (border_color.b as f32) * border_mix).round()
                            as u8,
                        a: ((c.a as f32) * inv + (border_color.a as f32) * border_mix).round()
                            as u8,
                    };
                }
                c.a = ((c.a as f32) * outer_alpha).round() as u8;
                self.set_pixel(px, py, c);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn fill_box_shadow(
        &mut self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        offset_x: f32,
        offset_y: f32,
        blur_radius: f32,
        spread_radius: f32,
        radius: f32,
        color: Color,
    ) {
        if width <= 0.0 || height <= 0.0 || color.a == 0 {
            return;
        }
        let blur = blur_radius.max(0.0);
        let sx_rect = x + offset_x - spread_radius;
        let sy_rect = y + offset_y - spread_radius;
        let sw = (width + spread_radius * 2.0).max(1.0);
        let sh = (height + spread_radius * 2.0).max(1.0);
        let sr = (radius + spread_radius).max(0.0);

        let x0 = ((sx_rect - blur).floor() as i32).max(0);
        let y0 = ((sy_rect - blur).floor() as i32).max(0);
        let x1 = ((sx_rect + sw + blur).ceil() as i32).min(self.width as i32);
        let y1 = ((sy_rect + sh + blur).ceil() as i32).min(self.height as i32);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let half_w = sw * 0.5;
        let half_h = sh * 0.5;
        let cx = sx_rect + half_w;
        let cy = sy_rect + half_h;

        for py in y0..y1 {
            for px in x0..x1 {
                let sample_x = px as f32 + 0.5;
                let sample_y = py as f32 + 0.5;
                let dist = sdf_rounded_box(sample_x - cx, sample_y - cy, half_w, half_h, sr);
                let alpha = sdf_box_shadow_alpha(dist, blur);
                if alpha <= 0.0 {
                    continue;
                }
                let mut c = color;
                c.a = ((c.a as f32) * alpha).round() as u8;
                self.set_pixel(px, py, c);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn fill_linear_gradient_rect(
        &mut self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        radius: f32,
        border_width: f32,
        angle_deg: f32,
        start_color: Color,
        end_color: Color,
        border_color: Color,
    ) {
        if width <= 0.0 || height <= 0.0 {
            return;
        }
        let x0 = (x.floor() as i32).max(0);
        let y0 = (y.floor() as i32).max(0);
        let x1 = ((x + width).ceil() as i32).min(self.width as i32);
        let y1 = ((y + height).ceil() as i32).min(self.height as i32);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let half_w = width * 0.5;
        let half_h = height * 0.5;
        let cx = x + half_w;
        let cy = y + half_h;
        let gpu_start = GpuColor::rgba(start_color.r, start_color.g, start_color.b, start_color.a);
        let gpu_end = GpuColor::rgba(end_color.r, end_color.g, end_color.b, end_color.a);

        for py in y0..y1 {
            for px in x0..x1 {
                let sx = px as f32 + 0.5;
                let sy = py as f32 + 0.5;
                let rel_x = sx - cx;
                let rel_y = sy - cy;
                let dist = sdf_rounded_box(rel_x, rel_y, half_w, half_h, radius);
                let outer_alpha = (0.5 - dist).clamp(0.0, 1.0);
                if outer_alpha <= 0.0 {
                    continue;
                }
                let grad = sample_linear_gradient(
                    rel_x, rel_y, half_w, half_h, angle_deg, gpu_start, gpu_end,
                );
                let mut c = Color {
                    r: grad.r,
                    g: grad.g,
                    b: grad.b,
                    a: grad.a,
                };
                if border_width > 0.0 {
                    let inner_dist = dist + border_width;
                    let border_mix = (0.5 + inner_dist).clamp(0.0, 1.0);
                    let inv = 1.0 - border_mix;
                    c = Color {
                        r: ((c.r as f32) * inv + (border_color.r as f32) * border_mix).round()
                            as u8,
                        g: ((c.g as f32) * inv + (border_color.g as f32) * border_mix).round()
                            as u8,
                        b: ((c.b as f32) * inv + (border_color.b as f32) * border_mix).round()
                            as u8,
                        a: ((c.a as f32) * inv + (border_color.a as f32) * border_mix).round()
                            as u8,
                    };
                }
                c.a = ((c.a as f32) * outer_alpha).round() as u8;
                self.set_pixel(px, py, c);
            }
        }
    }

    pub fn draw_ttf_glyph(
        &mut self,
        x: f32,
        y: f32,
        ch: char,
        font_size: f32,
        color: Color,
        font: &TtfFont,
    ) -> Option<f32> {
        let bmp = font.rasterize_glyph(ch, font_size)?;
        let baseline_y = y + (font.ascender as f32) * font_size / (font.units_per_em.max(1) as f32);
        let gx0 = (x.round() as i32) + bmp.bearing_x;
        let gy0 = (baseline_y.round() as i32) - bmp.bearing_y;
        for py in 0..bmp.height {
            for px in 0..bmp.width {
                let cov = bmp.coverage[(py * bmp.width + px) as usize];
                if cov == 0 {
                    continue;
                }
                let mut c = color;
                c.a = ((u16::from(c.a) * u16::from(cov) + 127) / 255) as u8;
                self.set_pixel(gx0 + px as i32, gy0 + py as i32, c);
            }
        }
        Some(bmp.advance_px)
    }

    pub fn draw_image(
        &mut self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        border_radius: f32,
        image: &Image,
    ) {
        if width <= 0.0 || height <= 0.0 || image.width == 0 || image.height == 0 {
            return;
        }
        let dst_w = width.round().max(1.0) as u32;
        let dst_h = height.round().max(1.0) as u32;
        let scaled = image.resize_bilinear(dst_w, dst_h);
        let half_w = (dst_w as f32) * 0.5;
        let half_h = (dst_h as f32) * 0.5;
        let base_x = x.round() as i32;
        let base_y = y.round() as i32;

        for iy in 0..dst_h {
            for ix in 0..dst_w {
                let Some(rgba) = scaled.pixel(ix, iy) else {
                    continue;
                };
                let mut alpha = rgba[3] as f32;
                if border_radius > 0.0 {
                    let dist = sdf_rounded_box(
                        (ix as f32 + 0.5) - half_w,
                        (iy as f32 + 0.5) - half_h,
                        half_w,
                        half_h,
                        border_radius,
                    );
                    let mask = (0.5 - dist).clamp(0.0, 1.0);
                    alpha *= mask;
                }
                if alpha <= 0.5 {
                    continue;
                }
                self.set_pixel(
                    base_x + ix as i32,
                    base_y + iy as i32,
                    Color {
                        r: rgba[0],
                        g: rgba[1],
                        b: rgba[2],
                        a: alpha.round() as u8,
                    },
                );
            }
        }
    }

    pub fn draw_text(&mut self, x: f32, y: f32, text: &str, font_size: f32, color: Color) {
        if font_size <= 0.0 || color.a == 0 {
            return;
        }
        #[cfg(windows)]
        if font::draw_system_text(self, x, y, text, font_size, color) {
            return;
        }
        self.draw_bitmap_text(x, y, text, font_size, color);
    }

    pub fn draw_bitmap_text(&mut self, x: f32, y: f32, text: &str, font_size: f32, color: Color) {
        if font_size <= 0.0 || color.a == 0 {
            return;
        }
        let scale = ((font_size / 12.0).floor() as i32).max(1);
        let advance = font_size * 0.5;
        let glyph_height = 7 * scale;
        let line_height = (font_size * 1.25).round() as i32;
        let pad_y = ((line_height - glyph_height) / 2).max(0);
        let base_y = y.round() as i32 + pad_y;

        for (index, ch) in text.chars().enumerate() {
            let char_x = (x + index as f32 * advance).round() as i32;
            let rows = glyph(ch);
            for (row_idx, row_bits) in rows.iter().copied().enumerate() {
                for col_idx in 0..5 {
                    if (row_bits >> (4 - col_idx)) & 1 == 1 {
                        let gx = char_x + col_idx * scale;
                        let gy = base_y + (row_idx as i32) * scale;
                        for sy in 0..scale {
                            for sx in 0..scale {
                                self.set_pixel(gx + sx, gy + sy, color);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Convert top-down RGBA scanlines into bottom-up BGRA scanlines for GDI.
    pub fn to_bgra_bottom_up(&self) -> Vec<u8> {
        let stride = (self.width as usize) * 4;
        let mut out = vec![0_u8; self.pixels.len()];
        for row in 0..(self.height as usize) {
            let src_row = (self.height as usize - 1 - row) * stride;
            let dst_row = row * stride;
            for col in 0..(self.width as usize) {
                let s = src_row + col * 4;
                let d = dst_row + col * 4;
                out[d] = self.pixels[s + 2];
                out[d + 1] = self.pixels[s + 1];
                out[d + 2] = self.pixels[s];
                out[d + 3] = self.pixels[s + 3];
            }
        }
        out
    }

    /// Encode the framebuffer as a binary PPM (`P6`) image.
    pub fn to_ppm(&self) -> Vec<u8> {
        let header = format!("P6\n{} {}\n255\n", self.width, self.height);
        let mut out =
            Vec::with_capacity(header.len() + (self.width as usize) * (self.height as usize) * 3);
        out.extend_from_slice(header.as_bytes());
        for chunk in self.pixels.chunks_exact(4) {
            out.push(chunk[0]);
            out.push(chunk[1]);
            out.push(chunk[2]);
        }
        out
    }
}

pub fn rasterize(list: &DisplayList, width: u32, height: u32, scroll_y: f32) -> Framebuffer {
    let mut fb = Framebuffer::new(width, height, PAPER);
    let mut fixed_depth = 0usize;
    for item in &list.items {
        let dy = if fixed_depth > 0 { 0.0 } else { scroll_y };
        match item {
            DisplayItem::PushClip {
                x,
                y,
                width,
                height,
            } => {
                fb.push_clip(*x, *y - dy, *width, *height);
            }
            DisplayItem::PopClip => {
                fb.pop_clip();
            }
            DisplayItem::PushFixed => {
                fixed_depth += 1;
            }
            DisplayItem::PopFixed => {
                fixed_depth = fixed_depth.saturating_sub(1);
            }
            DisplayItem::BoxShadow {
                x,
                y,
                width,
                height,
                offset_x,
                offset_y,
                blur_radius,
                spread_radius,
                radius,
                color,
            } => {
                fb.fill_box_shadow(
                    *x,
                    *y - dy,
                    *width,
                    *height,
                    *offset_x,
                    *offset_y,
                    *blur_radius,
                    *spread_radius,
                    *radius,
                    *color,
                );
            }
            DisplayItem::LinearGradientRect {
                x,
                y,
                width,
                height,
                radius,
                border_width,
                angle_deg,
                start_color,
                end_color,
                border_color,
            } => {
                fb.fill_linear_gradient_rect(
                    *x,
                    *y - dy,
                    *width,
                    *height,
                    *radius,
                    *border_width,
                    *angle_deg,
                    *start_color,
                    *end_color,
                    *border_color,
                );
            }
            DisplayItem::Rect {
                x,
                y,
                width,
                height,
                color,
            } => {
                fb.fill_rect(*x, *y - dy, *width, *height, *color);
            }
            DisplayItem::RoundedRect {
                x,
                y,
                width,
                height,
                radius,
                border_width,
                fill_color,
                border_color,
            } => {
                fb.fill_rounded_rect(
                    *x,
                    *y - dy,
                    *width,
                    *height,
                    *radius,
                    *border_width,
                    *fill_color,
                    *border_color,
                );
            }
            DisplayItem::Image {
                x,
                y,
                width,
                height,
                border_radius,
                image,
            } => {
                fb.draw_image(*x, *y - dy, *width, *height, *border_radius, image);
            }
            DisplayItem::Text {
                x,
                y,
                text,
                color,
                font_size,
            } => {
                fb.draw_text(*x, *y - dy, text, *font_size, *color);
            }
        }
    }
    fb
}

fn glyph(ch: char) -> [u8; 7] {
    match ch {
        ' ' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        '!' => [0x04, 0x04, 0x04, 0x04, 0x04, 0x00, 0x04],
        '"' => [0x0a, 0x0a, 0x0a, 0x00, 0x00, 0x00, 0x00],
        '#' => [0x0a, 0x1f, 0x0a, 0x0a, 0x1f, 0x0a, 0x00],
        '$' => [0x04, 0x0f, 0x14, 0x0e, 0x05, 0x1e, 0x04],
        '%' => [0x18, 0x19, 0x02, 0x04, 0x08, 0x13, 0x03],
        '&' => [0x0c, 0x12, 0x14, 0x08, 0x15, 0x12, 0x0d],
        '\'' => [0x04, 0x04, 0x08, 0x00, 0x00, 0x00, 0x00],
        '(' => [0x02, 0x04, 0x08, 0x08, 0x08, 0x04, 0x02],
        ')' => [0x08, 0x04, 0x02, 0x02, 0x02, 0x04, 0x08],
        '*' => [0x00, 0x04, 0x15, 0x0e, 0x15, 0x04, 0x00],
        '+' => [0x00, 0x04, 0x04, 0x1f, 0x04, 0x04, 0x00],
        ',' => [0x00, 0x00, 0x00, 0x00, 0x04, 0x04, 0x08],
        '-' => [0x00, 0x00, 0x00, 0x1f, 0x00, 0x00, 0x00],
        '.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0x04],
        '/' => [0x01, 0x02, 0x02, 0x04, 0x08, 0x08, 0x10],
        '0' => [0x0e, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0e],
        '1' => [0x04, 0x0c, 0x04, 0x04, 0x04, 0x04, 0x0e],
        '2' => [0x0e, 0x11, 0x01, 0x06, 0x08, 0x10, 0x1f],
        '3' => [0x1f, 0x02, 0x04, 0x06, 0x01, 0x11, 0x0e],
        '4' => [0x02, 0x06, 0x0a, 0x12, 0x1f, 0x02, 0x02],
        '5' => [0x1f, 0x10, 0x1e, 0x01, 0x01, 0x11, 0x0e],
        '6' => [0x06, 0x08, 0x10, 0x1e, 0x11, 0x11, 0x0e],
        '7' => [0x1f, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0e, 0x11, 0x11, 0x0e, 0x11, 0x11, 0x0e],
        '9' => [0x0e, 0x11, 0x11, 0x0f, 0x01, 0x02, 0x0c],
        ':' => [0x00, 0x04, 0x04, 0x00, 0x04, 0x04, 0x00],
        ';' => [0x00, 0x04, 0x04, 0x00, 0x04, 0x04, 0x08],
        '<' => [0x02, 0x04, 0x08, 0x10, 0x08, 0x04, 0x02],
        '=' => [0x00, 0x00, 0x1f, 0x00, 0x1f, 0x00, 0x00],
        '>' => [0x08, 0x04, 0x02, 0x01, 0x02, 0x04, 0x08],
        '?' => [0x0e, 0x11, 0x01, 0x02, 0x04, 0x00, 0x04],
        '@' => [0x0e, 0x11, 0x17, 0x15, 0x17, 0x10, 0x0e],
        'A' => [0x0e, 0x11, 0x11, 0x1f, 0x11, 0x11, 0x11],
        'B' => [0x1e, 0x11, 0x11, 0x1e, 0x11, 0x11, 0x1e],
        'C' => [0x0e, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0e],
        'D' => [0x1c, 0x12, 0x11, 0x11, 0x11, 0x12, 0x1c],
        'E' => [0x1f, 0x10, 0x10, 0x1e, 0x10, 0x10, 0x1f],
        'F' => [0x1f, 0x10, 0x10, 0x1e, 0x10, 0x10, 0x10],
        'G' => [0x0e, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0f],
        'H' => [0x11, 0x11, 0x11, 0x1f, 0x11, 0x11, 0x11],
        'I' => [0x0e, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0e],
        'J' => [0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0c],
        'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1f],
        'M' => [0x11, 0x1b, 0x15, 0x15, 0x11, 0x11, 0x11],
        'N' => [0x11, 0x19, 0x15, 0x13, 0x11, 0x11, 0x11],
        'O' => [0x0e, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0e],
        'P' => [0x1e, 0x11, 0x11, 0x1e, 0x10, 0x10, 0x10],
        'Q' => [0x0e, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0d],
        'R' => [0x1e, 0x11, 0x11, 0x1e, 0x14, 0x12, 0x11],
        'S' => [0x0f, 0x10, 0x10, 0x0e, 0x01, 0x01, 0x1e],
        'T' => [0x1f, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0e],
        'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0a, 0x04],
        'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0a],
        'X' => [0x11, 0x11, 0x0a, 0x04, 0x0a, 0x11, 0x11],
        'Y' => [0x11, 0x11, 0x0a, 0x04, 0x04, 0x04, 0x04],
        'Z' => [0x1f, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1f],
        '[' => [0x0e, 0x08, 0x08, 0x08, 0x08, 0x08, 0x0e],
        '\\' => [0x10, 0x08, 0x08, 0x04, 0x02, 0x02, 0x01],
        ']' => [0x0e, 0x02, 0x02, 0x02, 0x02, 0x02, 0x0e],
        '^' => [0x04, 0x0a, 0x11, 0x00, 0x00, 0x00, 0x00],
        '_' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1f],
        '`' => [0x08, 0x04, 0x02, 0x00, 0x00, 0x00, 0x00],
        'a' => [0x00, 0x00, 0x0e, 0x01, 0x0f, 0x11, 0x0f],
        'b' => [0x10, 0x10, 0x1e, 0x11, 0x11, 0x11, 0x1e],
        'c' => [0x00, 0x00, 0x0e, 0x10, 0x10, 0x11, 0x0e],
        'd' => [0x01, 0x01, 0x0f, 0x11, 0x11, 0x11, 0x0f],
        'e' => [0x00, 0x00, 0x0e, 0x11, 0x1f, 0x10, 0x0e],
        'f' => [0x06, 0x08, 0x1e, 0x08, 0x08, 0x08, 0x08],
        'g' => [0x00, 0x0f, 0x11, 0x11, 0x0f, 0x01, 0x0e],
        'h' => [0x10, 0x10, 0x1e, 0x11, 0x11, 0x11, 0x11],
        'i' => [0x04, 0x00, 0x0c, 0x04, 0x04, 0x04, 0x0e],
        'j' => [0x02, 0x00, 0x06, 0x02, 0x02, 0x12, 0x0c],
        'k' => [0x10, 0x10, 0x12, 0x14, 0x18, 0x14, 0x12],
        'l' => [0x0c, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0e],
        'm' => [0x00, 0x00, 0x1a, 0x15, 0x15, 0x11, 0x11],
        'n' => [0x00, 0x00, 0x1e, 0x11, 0x11, 0x11, 0x11],
        'o' => [0x00, 0x00, 0x0e, 0x11, 0x11, 0x11, 0x0e],
        'p' => [0x00, 0x1e, 0x11, 0x11, 0x1e, 0x10, 0x10],
        'q' => [0x00, 0x0f, 0x11, 0x11, 0x0f, 0x01, 0x01],
        'r' => [0x00, 0x00, 0x16, 0x19, 0x10, 0x10, 0x10],
        's' => [0x00, 0x00, 0x0f, 0x10, 0x0e, 0x01, 0x1e],
        't' => [0x08, 0x08, 0x1e, 0x08, 0x08, 0x09, 0x06],
        'u' => [0x00, 0x00, 0x11, 0x11, 0x11, 0x13, 0x0d],
        'v' => [0x00, 0x00, 0x11, 0x11, 0x11, 0x0a, 0x04],
        'w' => [0x00, 0x00, 0x11, 0x11, 0x15, 0x15, 0x0a],
        'x' => [0x00, 0x00, 0x11, 0x0a, 0x04, 0x0a, 0x11],
        'y' => [0x00, 0x11, 0x11, 0x11, 0x0f, 0x01, 0x0e],
        'z' => [0x00, 0x00, 0x1f, 0x02, 0x04, 0x08, 0x1f],
        '{' => [0x02, 0x04, 0x04, 0x08, 0x04, 0x04, 0x02],
        '|' => [0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        '}' => [0x08, 0x04, 0x04, 0x02, 0x04, 0x04, 0x08],
        '~' => [0x00, 0x00, 0x08, 0x15, 0x02, 0x00, 0x00],
        _ => [0x1f, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1f],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_a_rectangle_and_leaves_outside_pixels_alone() {
        let red = Color {
            r: 220,
            g: 30,
            b: 40,
            a: 255,
        };
        let list = DisplayList {
            items: vec![DisplayItem::Rect {
                x: 2.0,
                y: 3.0,
                width: 4.0,
                height: 5.0,
                color: red,
            }],
        };
        let fb = rasterize(&list, 16, 16, 0.0);
        assert_eq!(fb.pixel(2, 3), Some(red));
        assert_eq!(fb.pixel(5, 7), Some(red));
        assert_eq!(fb.pixel(1, 3), Some(PAPER));
        assert_eq!(fb.pixel(6, 7), Some(PAPER));
    }

    #[test]
    fn draws_glyph_pixels_and_respects_vertical_scroll() {
        let ink = Color {
            r: 10,
            g: 20,
            b: 30,
            a: 255,
        };
        let list = DisplayList {
            items: vec![DisplayItem::Text {
                x: 4.0,
                y: 20.0,
                text: "Flux".to_string(),
                color: ink,
                font_size: 16.0,
            }],
        };
        let scrolled = rasterize(&list, 64, 32, 16.0);
        let lit = (0..32)
            .flat_map(|y| (0..64).map(move |x| (x, y)))
            .any(|(x, y)| scrolled.pixel(x, y) == Some(ink));
        assert!(lit);

        let bgra = scrolled.to_bgra_bottom_up();
        assert_eq!(bgra.len(), 64 * 32 * 4);
    }

    #[test]
    fn clips_overflow_and_keeps_fixed_items_pinned_on_scroll() {
        let blue = Color {
            r: 20,
            g: 100,
            b: 220,
            a: 255,
        };
        let gold = Color {
            r: 232,
            g: 195,
            b: 122,
            a: 255,
        };
        let list = DisplayList {
            items: vec![
                DisplayItem::PushClip {
                    x: 0.0,
                    y: 0.0,
                    width: 4.0,
                    height: 4.0,
                },
                DisplayItem::Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 10.0,
                    height: 10.0,
                    color: blue,
                },
                DisplayItem::PopClip,
                DisplayItem::PushFixed,
                DisplayItem::Rect {
                    x: 8.0,
                    y: 2.0,
                    width: 4.0,
                    height: 4.0,
                    color: gold,
                },
                DisplayItem::PopFixed,
            ],
        };
        let fb = rasterize(&list, 16, 16, 50.0);
        // Scrolled clipped rect is scrolled out of view, while fixed rect stays at (8, 2)
        assert_eq!(fb.pixel(9, 3), Some(gold));
        let fb_no_scroll = rasterize(&list, 16, 16, 0.0);
        assert_eq!(fb_no_scroll.pixel(2, 2), Some(blue));
        assert_eq!(fb_no_scroll.pixel(6, 6), Some(PAPER));
    }

    #[test]
    fn renders_box_shadows_and_linear_gradients_via_flux_gpu() {
        let list = DisplayList {
            items: vec![
                DisplayItem::BoxShadow {
                    x: 4.0,
                    y: 4.0,
                    width: 8.0,
                    height: 8.0,
                    offset_x: 0.0,
                    offset_y: 2.0,
                    blur_radius: 4.0,
                    spread_radius: 0.0,
                    radius: 2.0,
                    color: Color {
                        r: 0,
                        g: 0,
                        b: 0,
                        a: 180,
                    },
                },
                DisplayItem::LinearGradientRect {
                    x: 0.0,
                    y: 0.0,
                    width: 16.0,
                    height: 8.0,
                    radius: 0.0,
                    border_width: 0.0,
                    angle_deg: 90.0,
                    start_color: Color {
                        r: 255,
                        g: 0,
                        b: 0,
                        a: 255,
                    },
                    end_color: Color {
                        r: 0,
                        g: 0,
                        b: 255,
                        a: 255,
                    },
                    border_color: Color::BLACK,
                },
            ],
        };
        let fb = rasterize(&list, 20, 20, 0.0);
        let left = fb.pixel(1, 4).unwrap();
        let right = fb.pixel(14, 4).unwrap();
        assert!(left.r > right.r);
        assert!(right.b > left.b);
        let shadow_px = fb.pixel(8, 13).unwrap();
        assert!(shadow_px.r < 255);
    }
}
