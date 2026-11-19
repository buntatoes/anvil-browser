// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Display list built from a laid-out box tree.
//!
//! Fourth stage of the pipeline. anvil-layout has already placed boxes.
//! This crate walks that tree into a z-ordered [`DisplayList`] and does not
//! draw pixels. anvil-raster consumes the list.
//!
//! Coordinates are borrowed from layout rects: origin at the top-left of the
//! laid-out tree, x to the right, y downward. Lengths are CSS pixels.
//!
//! Paint order for each box is shadow, then background (gradient, rounded
//! rect, or fill plus square borders), then a clip when overflow is hidden,
//! then image and text, then children from low `z-index` to high. `position:
//! fixed` wraps that range in [`DisplayItem::PushFixed`] / [`DisplayItem::PopFixed`].
//!
//! A sticky box against the viewport also adds a [`Layer`] over its range,
//! so the rasterizer can move it for the scroll position. A transformed box
//! adds a transform layer the same way; items inside it keep their
//! untransformed page coordinates. A box with `opacity` below 1 adds an
//! opacity layer. When one box has several, the sticky layer is outermost,
//! then the transform, then the opacity.
//!
//! [`DisplayList::scaled`] multiplies lengths for zoom. Callers use 50%–250%
//! (`0.5`–`2.5`). The clamp inside that method is wider (`0.25`–`4.0`).
//! There is no node cap here; the list is as long as the box tree.

mod damage;

use anvil_layout::{
    BackgroundRepeat, BorderStyle, Color, Edge, Image, LayoutBox, ObjectFit, Overflow, Position,
};
pub use anvil_layout::{
    GradientKind, GradientStop, LinearGradient, Rect, StickyConstraint, Transform2D,
};
pub use damage::damage_between;

/// One painting command. Positions and sizes are CSS pixels, y downward.
///
/// Colors are copied. [`DisplayItem::Text`] and [`DisplayItem::Image`] own
/// their text and image. Other variants store only numbers and colors.
#[derive(Clone, Debug, PartialEq)]
pub enum DisplayItem {
    /// Intersect the current clip with this rectangle. Paired with [`Self::PopClip`].
    /// The rectangle is the content box (border box inset by the border widths).
    PushClip {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    },
    /// Remove the clip pushed by the matching [`Self::PushClip`].
    PopClip,
    /// Intersect the current clip with a rounded rectangle (`radius` >= 0). Paired
    /// with [`Self::PopRoundedClip`].
    PushRoundedClip {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        radius: f32,
    },
    /// Remove the rounded clip pushed by the matching [`Self::PushRoundedClip`].
    PopRoundedClip,
    /// Mark the start of a `position: fixed` subtree. The rasterizer treats
    /// items until [`Self::PopFixed`] as fixed to the viewport.
    PushFixed,
    /// End the fixed layer opened by the matching [`Self::PushFixed`].
    PopFixed,
    /// A shadow drawn before the box fill. `x`/`y`/`width`/`height` are the
    /// border box. Offsets and blur are not applied here; they are stored for
    /// the rasterizer. `radius` is the box corner radius.
    BoxShadow {
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
    },
    /// A two-stop linear gradient filling the border box. `angle_deg` is the
    /// CSS angle (0 is up, 90 is right) and is not converted to a vector here.
    /// `border_width` is the largest of the four border sides, not a per-side width.
    LinearGradientRect {
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
    },
    /// Multi-stop, radial, conic, repeating gradient, or asymmetric-corner fill.
    /// When `stop_count == 1`, `stops[0].color` paints a solid fill with
    /// independent per-corner `radii` (`[top_left, top_right, bottom_right, bottom_left]`).
    GradientRect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        radii: [f32; 4],
        border_width: f32,
        angle_deg: f32,
        kind: GradientKind,
        stop_count: u8,
        stops: [GradientStop; 4],
        border_color: Color,
    },
    /// An axis-aligned fill, also used for square border strips and underlines.
    Rect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        color: Color,
    },
    /// A filled rounded rectangle. Used when `border-radius` is positive and
    /// the box has no gradient. A zero-alpha fill still carries the border.
    RoundedRect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        radius: f32,
        border_width: f32,
        fill_color: Color,
        border_color: Color,
    },
    /// An image painted in the content box. `image` is an owned clone.
    /// `border_radius` is the box radius minus the top border, floored at 0.
    Image {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        border_radius: f32,
        image: Image,
    },
    /// A text run. `x`/`y` is the top-left of the run in the layout's y-down
    /// space. `text` is owned. The font size is in CSS pixels.
    Text {
        x: f32,
        y: f32,
        text: String,
        color: Color,
        font_size: f32,
    },
}

/// Owned sequence of [`DisplayItem`]s in paint order.
///
/// `layers` marks item ranges that the rasterizer draws as a group. They are
/// sorted by `start`, outer layers before the layers they contain, and two
/// layers either nest or do not overlap. A list with no layers draws exactly
/// as before layers existed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DisplayList {
    pub items: Vec<DisplayItem>,
    pub layers: Vec<Layer>,
}

/// A range `start..end` of [`DisplayList::items`] with one effect.
///
/// The range covers one box and its descendants. For a fixed box it sits
/// inside that box's [`DisplayItem::PushFixed`] / [`DisplayItem::PopFixed`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layer {
    pub start: usize,
    pub end: usize,
    pub effect: LayerEffect,
}

/// What the rasterizer does with a [`Layer`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LayerEffect {
    /// Move the range by [`StickyConstraint::offset`] for the scroll position.
    Sticky(StickyConstraint),
    /// Draw the range through this page-space map, after any enclosing
    /// layers. Clips inside the range are mapped with it; clips outside it
    /// stay screen-aligned.
    Transform(Transform2D),
    /// Draw the range as one group, then blend it at this opacity in
    /// `0..1`, so overlapping items inside do not show through each other.
    Opacity(f32),
    /// Blur the framebuffer backdrop behind `rect` (masked by per-corner `radii`)
    /// before painting the layer's items (`backdrop-filter: blur(...)`).
    BackdropBlur {
        rect: Rect,
        radius: f32,
        radii: [f32; 4],
    },
    /// Blur the rendered group output of this layer (`filter: blur(...)`).
    FilterBlur(f32),
}

impl Layer {
    /// This layer with its geometry scaled by `s`, for zoom.
    pub fn scaled(&self, s: f32) -> Self {
        let effect = match self.effect {
            LayerEffect::Sticky(c) => {
                let sr = |r: Rect| Rect {
                    x: r.x * s,
                    y: r.y * s,
                    width: r.width * s,
                    height: r.height * s,
                };
                let so = |v: Option<f32>| v.map(|v| v * s);
                LayerEffect::Sticky(StickyConstraint {
                    rect: sr(c.rect),
                    margin: Edge {
                        top: c.margin.top * s,
                        right: c.margin.right * s,
                        bottom: c.margin.bottom * s,
                        left: c.margin.left * s,
                    },
                    containing: sr(c.containing),
                    top: so(c.top),
                    right: so(c.right),
                    bottom: so(c.bottom),
                    left: so(c.left),
                })
            }
            LayerEffect::Transform(m) => LayerEffect::Transform(m.scaled(s)),
            LayerEffect::Opacity(o) => LayerEffect::Opacity(o),
            LayerEffect::BackdropBlur {
                rect,
                radius,
                radii,
            } => LayerEffect::BackdropBlur {
                rect: Rect {
                    x: rect.x * s,
                    y: rect.y * s,
                    width: rect.width * s,
                    height: rect.height * s,
                },
                radius: radius * s,
                radii: radii.map(|r| r * s),
            },
            LayerEffect::FilterBlur(r) => LayerEffect::FilterBlur(r * s),
        };
        Self { effect, ..*self }
    }
}

impl DisplayList {
    /// Return a scaled copy of this display list.
    ///
    /// `scale` is a unitless zoom factor. Values outside `0.25`..=`4.0` are
    /// clamped. A factor within epsilon of `1` returns a clone and does not
    /// rescale. Lengths, radii, blur, spread, border width, and font size are
    /// multiplied. Colors and gradient angles are copied. Text and images are
    /// cloned. [`DisplayItem::PopClip`], [`DisplayItem::PushFixed`], and
    /// [`DisplayItem::PopFixed`] are unchanged. The receiver is borrowed.
    pub fn scaled(&self, scale: f32) -> Self {
        // Wider than the 50%–250% viewport zoom so a caller can probe the edges.
        let s = scale.clamp(0.25, 4.0);
        if (s - 1.0).abs() < f32::EPSILON {
            return self.clone();
        }
        let items = self
            .items
            .iter()
            .map(|item| item.mapped(s, 0.0, 0.0))
            .collect();
        let layers = self.layers.iter().map(|layer| layer.scaled(s)).collect();
        Self { items, layers }
    }
}

impl DisplayItem {
    /// This item scaled by `s` about the origin, then moved by `(tx, ty)`.
    ///
    /// Positions become `p * s + t`. Sizes, radii, blur, spread, border
    /// width, and font size are multiplied by `s`. Colors, gradient angles,
    /// text, and images are copied. Markers are unchanged.
    pub fn mapped(&self, s: f32, tx: f32, ty: f32) -> Self {
        match self {
            DisplayItem::PushClip {
                x,
                y,
                width,
                height,
            } => DisplayItem::PushClip {
                x: x * s + tx,
                y: y * s + ty,
                width: width * s,
                height: height * s,
            },
            DisplayItem::PopClip => DisplayItem::PopClip,
            DisplayItem::PushRoundedClip {
                x,
                y,
                width,
                height,
                radius,
            } => DisplayItem::PushRoundedClip {
                x: x * s + tx,
                y: y * s + ty,
                width: width * s,
                height: height * s,
                radius: radius * s,
            },
            DisplayItem::PopRoundedClip => DisplayItem::PopRoundedClip,
            DisplayItem::PushFixed => DisplayItem::PushFixed,
            DisplayItem::PopFixed => DisplayItem::PopFixed,
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
            } => DisplayItem::BoxShadow {
                x: x * s + tx,
                y: y * s + ty,
                width: width * s,
                height: height * s,
                offset_x: offset_x * s,
                offset_y: offset_y * s,
                blur_radius: blur_radius * s,
                spread_radius: spread_radius * s,
                radius: radius * s,
                color: *color,
            },
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
            } => DisplayItem::LinearGradientRect {
                x: x * s + tx,
                y: y * s + ty,
                width: width * s,
                height: height * s,
                radius: radius * s,
                border_width: border_width * s,
                angle_deg: *angle_deg,
                start_color: *start_color,
                end_color: *end_color,
                border_color: *border_color,
            },
            DisplayItem::GradientRect {
                x,
                y,
                width,
                height,
                radii,
                border_width,
                angle_deg,
                kind,
                stop_count,
                stops,
                border_color,
            } => DisplayItem::GradientRect {
                x: x * s + tx,
                y: y * s + ty,
                width: width * s,
                height: height * s,
                radii: radii.map(|r| r * s),
                border_width: border_width * s,
                angle_deg: *angle_deg,
                kind: *kind,
                stop_count: *stop_count,
                stops: *stops,
                border_color: *border_color,
            },
            DisplayItem::Rect {
                x,
                y,
                width,
                height,
                color,
            } => DisplayItem::Rect {
                x: x * s + tx,
                y: y * s + ty,
                width: width * s,
                height: height * s,
                color: *color,
            },
            DisplayItem::RoundedRect {
                x,
                y,
                width,
                height,
                radius,
                border_width,
                fill_color,
                border_color,
            } => DisplayItem::RoundedRect {
                x: x * s + tx,
                y: y * s + ty,
                width: width * s,
                height: height * s,
                radius: radius * s,
                border_width: border_width * s,
                fill_color: *fill_color,
                border_color: *border_color,
            },
            DisplayItem::Image {
                x,
                y,
                width,
                height,
                border_radius,
                image,
            } => DisplayItem::Image {
                x: x * s + tx,
                y: y * s + ty,
                width: width * s,
                height: height * s,
                border_radius: border_radius * s,
                image: image.clone(),
            },
            DisplayItem::Text {
                x,
                y,
                text,
                color,
                font_size,
            } => DisplayItem::Text {
                x: x * s + tx,
                y: y * s + ty,
                text: text.clone(),
                color: *color,
                font_size: font_size * s,
            },
        }
    }
}

/// Build an owned display list by walking `tree` in paint order.
///
/// `tree` is borrowed. Empty boxes (non-positive width or height) contribute
/// no fill, shadow, or image. Children are still visited.
pub fn paint(tree: &LayoutBox) -> DisplayList {
    let mut items = Vec::new();
    let mut layers = Vec::new();
    walk(tree, &mut items, &mut layers, 0);
    // Layers were pushed as boxes finished, inner first. Sorting by start,
    // longest first, then tree depth and push order puts each outer layer
    // before what it contains, even when two cover the same items.
    layers.sort_by_key(|&(depth, order, layer): &(usize, usize, Layer)| {
        (layer.start, std::cmp::Reverse(layer.end), depth, order)
    });
    let layers = layers.into_iter().map(|(_, _, layer)| layer).collect();
    DisplayList { items, layers }
}

/// Append one box and its descendants. See the crate docs for paint order.
///
/// Child order is [`LayoutBox::stacking_key`], then source index, ascending,
/// so a higher `z-index` paints later and ties keep tree order, with sticky
/// boxes after plain siblings of the same `z-index`. The sort is not stable across equal
/// keys beyond that index.
fn walk(
    tree: &LayoutBox,
    items: &mut Vec<DisplayItem>,
    layers: &mut Vec<(usize, usize, Layer)>,
    depth: usize,
) {
    let is_fixed = tree.position == Position::Fixed;
    if is_fixed {
        items.push(DisplayItem::PushFixed);
    }
    let layer_start = items.len();
    if let Some(shadow) = tree.box_shadow
        && tree.rect.width > 0.0
        && tree.rect.height > 0.0
        && shadow.color.a > 0
    {
        items.push(DisplayItem::BoxShadow {
            x: tree.rect.x,
            y: tree.rect.y,
            width: tree.rect.width,
            height: tree.rect.height,
            offset_x: shadow.offset_x,
            offset_y: shadow.offset_y,
            blur_radius: shadow.blur_radius,
            spread_radius: shadow.spread_radius,
            radius: tree.border_radius,
            color: shadow.color,
        });
    }
    let radii = tree.border_radii.map(|r| r.max(0.0));
    let any_radius = radii.iter().any(|&r| r > 0.0) || tree.border_radius > 0.0;
    let uniform_radii = (radii[0] - radii[1]).abs() < 0.01
        && (radii[0] - radii[2]).abs() < 0.01
        && (radii[0] - radii[3]).abs() < 0.01
        && (!any_radius || (radii[0] - tree.border_radius).abs() < 0.01);
    let all_solid_or_none = tree.border_styles.iter().all(|&st| {
        matches!(
            st,
            BorderStyle::Solid | BorderStyle::None | BorderStyle::Hidden
        )
    });
    let uniform_border = all_solid_or_none
        && (tree.border.top - tree.border.right).abs() < 0.01
        && (tree.border.top - tree.border.bottom).abs() < 0.01
        && (tree.border.top - tree.border.left).abs() < 0.01
        && (tree.border.top <= 0.0 || tree.border_styles[0] == BorderStyle::Solid)
        && tree.border_colors.iter().all(|&c| c == tree.border_color);

    if let Some(grad) = tree.background_gradient {
        if tree.rect.width > 0.0 && tree.rect.height > 0.0 {
            let stop_n = usize::from(grad.stop_count).clamp(2, 4);
            let translucent_grad = grad.start_color.a < 255
                || grad.end_color.a < 255
                || grad.stops[..stop_n].iter().any(|s| s.color.a < 255);
            if translucent_grad && let Some(bg_col) = tree.background.filter(|c| c.a > 0) {
                if any_radius && uniform_radii {
                    items.push(DisplayItem::RoundedRect {
                        x: tree.rect.x,
                        y: tree.rect.y,
                        width: tree.rect.width,
                        height: tree.rect.height,
                        radius: tree.border_radius.max(0.0),
                        border_width: 0.0,
                        fill_color: bg_col,
                        border_color: Color {
                            r: 0,
                            g: 0,
                            b: 0,
                            a: 0,
                        },
                    });
                } else if any_radius {
                    let mut bg_stops = [GradientStop::TRANSPARENT; 4];
                    bg_stops[0] = GradientStop {
                        color: bg_col,
                        position: 0.0,
                    };
                    items.push(DisplayItem::GradientRect {
                        x: tree.rect.x,
                        y: tree.rect.y,
                        width: tree.rect.width,
                        height: tree.rect.height,
                        radii,
                        border_width: 0.0,
                        angle_deg: 0.0,
                        kind: GradientKind::Linear,
                        stop_count: 1,
                        stops: bg_stops,
                        border_color: Color {
                            r: 0,
                            g: 0,
                            b: 0,
                            a: 0,
                        },
                    });
                } else {
                    items.push(DisplayItem::Rect {
                        x: tree.rect.x,
                        y: tree.rect.y,
                        width: tree.rect.width,
                        height: tree.rect.height,
                        color: bg_col,
                    });
                }
            }
            let border_width = if uniform_border {
                tree.border.top.max(0.0)
            } else {
                0.0
            };
            if !grad.is_procedural_2d() && uniform_radii {
                items.push(DisplayItem::LinearGradientRect {
                    x: tree.rect.x,
                    y: tree.rect.y,
                    width: tree.rect.width,
                    height: tree.rect.height,
                    radius: tree.border_radius.max(0.0),
                    border_width,
                    angle_deg: grad.angle_deg.clamp(-36_000.0, 36_000.0),
                    start_color: grad.start_color,
                    end_color: grad.end_color,
                    border_color: tree.border_color,
                });
            } else {
                let mut clean_stops = [GradientStop::TRANSPARENT; 4];
                if grad.stop_count <= 2
                    && grad.stops[0] == GradientStop::TRANSPARENT
                    && grad.stops[1] == GradientStop::TRANSPARENT
                {
                    clean_stops[0] = GradientStop {
                        color: grad.start_color,
                        position: 0.0,
                    };
                    clean_stops[1] = GradientStop {
                        color: grad.end_color,
                        position: 1.0,
                    };
                } else {
                    for (dst, src) in clean_stops.iter_mut().zip(&grad.stops[..stop_n]) {
                        *dst = GradientStop {
                            color: src.color,
                            position: if src.position.is_finite() {
                                src.position.clamp(0.0, 1.0)
                            } else {
                                0.0
                            },
                        };
                    }
                }
                let eff_radii = if any_radius && !uniform_radii {
                    radii
                } else {
                    [tree.border_radius.max(0.0); 4]
                };
                items.push(DisplayItem::GradientRect {
                    x: tree.rect.x,
                    y: tree.rect.y,
                    width: tree.rect.width,
                    height: tree.rect.height,
                    radii: eff_radii,
                    border_width,
                    angle_deg: if grad.angle_deg.is_finite() {
                        grad.angle_deg.clamp(-36_000.0, 36_000.0)
                    } else {
                        0.0
                    },
                    kind: grad.kind,
                    stop_count: stop_n as u8,
                    stops: clean_stops,
                    border_color: tree.border_color,
                });
            }
            if !uniform_border {
                paint_borders_clipped_if_rounded(tree, any_radius, items);
            }
        }
    } else if any_radius && tree.rect.width > 0.0 && tree.rect.height > 0.0 {
        let fill_color = tree.background.unwrap_or(Color {
            r: 0,
            g: 0,
            b: 0,
            a: 0,
        });
        let border_width = if uniform_border {
            tree.border.top.max(0.0)
        } else {
            0.0
        };
        if fill_color.a > 0 || border_width > 0.0 {
            if uniform_radii {
                items.push(DisplayItem::RoundedRect {
                    x: tree.rect.x,
                    y: tree.rect.y,
                    width: tree.rect.width,
                    height: tree.rect.height,
                    radius: tree.border_radius.max(radii[0]).max(0.0),
                    border_width,
                    fill_color,
                    border_color: tree.border_color,
                });
            } else {
                let mut stops = [GradientStop::TRANSPARENT; 4];
                stops[0] = GradientStop {
                    color: fill_color,
                    position: 0.0,
                };
                items.push(DisplayItem::GradientRect {
                    x: tree.rect.x,
                    y: tree.rect.y,
                    width: tree.rect.width,
                    height: tree.rect.height,
                    radii,
                    border_width,
                    angle_deg: 0.0,
                    kind: GradientKind::Linear,
                    stop_count: 1,
                    stops,
                    border_color: tree.border_color,
                });
            }
        }
        if !uniform_border {
            paint_borders_clipped_if_rounded(tree, any_radius, items);
        }
    } else {
        if let Some(color) = tree.background
            && tree.rect.width > 0.0
            && tree.rect.height > 0.0
        {
            items.push(DisplayItem::Rect {
                x: tree.rect.x,
                y: tree.rect.y,
                width: tree.rect.width,
                height: tree.rect.height,
                color,
            });
        }
        paint_borders(tree, items);
    }

    paint_background_image(tree, items);

    // Clip is the padding edge, y-down, when overflow is hidden on either axis.
    let is_clipped = (tree.overflow == Overflow::Hidden
        || tree.overflow_x == Overflow::Hidden
        || tree.overflow_y == Overflow::Hidden)
        && tree.rect.width > 0.0
        && tree.rect.height > 0.0;
    let clip_radius = (tree.border_radius - tree.border.top).max(0.0);
    let rounded_clipped = is_clipped && clip_radius > 0.0;
    if is_clipped {
        let clip_x = tree.rect.x + tree.border.left;
        let clip_y = tree.rect.y + tree.border.top;
        let clip_w = (tree.rect.width - tree.border.left - tree.border.right).max(0.0);
        let clip_h = (tree.rect.height - tree.border.top - tree.border.bottom).max(0.0);
        if rounded_clipped {
            items.push(DisplayItem::PushRoundedClip {
                x: clip_x,
                y: clip_y,
                width: clip_w,
                height: clip_h,
                radius: clip_radius,
            });
        } else {
            items.push(DisplayItem::PushClip {
                x: clip_x,
                y: clip_y,
                width: clip_w,
                height: clip_h,
            });
        }
    }

    if let Some(image) = &tree.image {
        let ix = tree.rect.x + tree.border.left;
        let iy = tree.rect.y + tree.border.top;
        let iw = (tree.rect.width - tree.border.left - tree.border.right).max(0.0);
        let ih = (tree.rect.height - tree.border.top - tree.border.bottom).max(0.0);
        if iw > 0.0 && ih > 0.0 {
            let img_w = (image.width as f32).max(1.0);
            let img_h = (image.height as f32).max(1.0);
            let (dx, dy, dw, dh, clip_fit) = match tree.object_fit {
                ObjectFit::Fill => (ix, iy, iw, ih, false),
                ObjectFit::Contain => {
                    let s = (iw / img_w).min(ih / img_h);
                    let w = img_w * s;
                    let h = img_h * s;
                    (ix + (iw - w) * 0.5, iy + (ih - h) * 0.5, w, h, false)
                }
                ObjectFit::Cover => {
                    let s = (iw / img_w).max(ih / img_h);
                    let w = img_w * s;
                    let h = img_h * s;
                    (
                        ix + (iw - w) * 0.5,
                        iy + (ih - h) * 0.5,
                        w,
                        h,
                        w > iw + 0.5 || h > ih + 0.5,
                    )
                }
                ObjectFit::None => (
                    ix + (iw - img_w) * 0.5,
                    iy + (ih - img_h) * 0.5,
                    img_w,
                    img_h,
                    img_w > iw + 0.5 || img_h > ih + 0.5,
                ),
                ObjectFit::ScaleDown => {
                    let s = (iw / img_w).min(ih / img_h).min(1.0);
                    let w = img_w * s;
                    let h = img_h * s;
                    (ix + (iw - w) * 0.5, iy + (ih - h) * 0.5, w, h, false)
                }
            };
            if clip_fit && !is_clipped {
                items.push(DisplayItem::PushClip {
                    x: ix,
                    y: iy,
                    width: iw,
                    height: ih,
                });
            }
            items.push(DisplayItem::Image {
                x: dx,
                y: dy,
                width: dw,
                height: dh,
                border_radius: (tree.border_radius - tree.border.top).max(0.0),
                image: image.clone(),
            });
            if clip_fit && !is_clipped {
                items.push(DisplayItem::PopClip);
            }
        }
    }
    for run in &tree.text {
        if (run.text == "X" || run.text == " ")
            && tree.rect.width >= 12.0
            && tree.rect.width <= 26.0
            && tree.rect.height >= 12.0
            && tree.rect.height <= 26.0
        {
            let box_sz = tree.rect.width.min(tree.rect.height).clamp(12.0, 20.0);
            let bx = tree.rect.x + (tree.rect.width - box_sz) * 0.5;
            let by = tree.rect.y + (tree.rect.height - box_sz) * 0.5;
            let checked = run.text == "X";
            items.push(DisplayItem::RoundedRect {
                x: bx,
                y: by,
                width: box_sz,
                height: box_sz,
                radius: 4.0,
                border_width: 1.5,
                fill_color: if checked {
                    Color {
                        r: 59,
                        g: 130,
                        b: 246,
                        a: 255,
                    }
                } else {
                    Color {
                        r: 255,
                        g: 255,
                        b: 255,
                        a: 0,
                    }
                },
                border_color: if checked {
                    Color {
                        r: 59,
                        g: 130,
                        b: 246,
                        a: 255,
                    }
                } else {
                    tree.border_color
                },
            });
        } else if (run.text == "(o)" || run.text == "( )")
            && tree.rect.width >= 14.0
            && tree.rect.width <= 38.0
            && tree.rect.height >= 12.0
            && tree.rect.height <= 28.0
        {
            let box_sz = tree.rect.height.min(tree.rect.width).clamp(14.0, 20.0);
            let bx = tree.rect.x + (tree.rect.width - box_sz) * 0.5;
            let by = tree.rect.y + (tree.rect.height - box_sz) * 0.5;
            let checked = run.text == "(o)";
            items.push(DisplayItem::RoundedRect {
                x: bx,
                y: by,
                width: box_sz,
                height: box_sz,
                radius: box_sz * 0.5,
                border_width: 1.5,
                fill_color: Color {
                    r: 0,
                    g: 0,
                    b: 0,
                    a: 0,
                },
                border_color: if checked {
                    Color {
                        r: 59,
                        g: 130,
                        b: 246,
                        a: 255,
                    }
                } else {
                    tree.border_color
                },
            });
            if checked {
                let dot = (box_sz * 0.45).max(6.0);
                items.push(DisplayItem::RoundedRect {
                    x: bx + (box_sz - dot) * 0.5,
                    y: by + (box_sz - dot) * 0.5,
                    width: dot,
                    height: dot,
                    radius: dot * 0.5,
                    border_width: 0.0,
                    fill_color: Color {
                        r: 59,
                        g: 130,
                        b: 246,
                        a: 255,
                    },
                    border_color: Color {
                        r: 0,
                        g: 0,
                        b: 0,
                        a: 0,
                    },
                });
            }
        }
        items.push(DisplayItem::Text {
            x: run.x,
            y: run.y,
            text: run.text.clone(),
            color: run.color,
            font_size: run.font_size,
        });
        // Underline sits below the run top, not on a font baseline.
        if run.underline && run.width > 0.0 {
            items.push(DisplayItem::Rect {
                x: run.x,
                y: run.y + run.font_size * 1.18,
                width: run.width,
                height: 1.0,
                color: run.color,
            });
        }
        if run.line_through && run.width > 0.0 {
            items.push(DisplayItem::Rect {
                x: run.x,
                y: run.y + run.font_size * 0.62,
                width: run.width,
                height: 1.0,
                color: run.color,
            });
        }
    }
    let mut order: Vec<(usize, &LayoutBox)> = tree.children.iter().enumerate().collect();
    order.sort_by_key(|(idx, child)| (child.stacking_key(), *idx));
    for (_, child) in order {
        walk(child, items, layers, depth + 1);
    }
    if is_clipped {
        let client_x = tree.rect.x + tree.border.left;
        let client_y = tree.rect.y + tree.border.top;
        let client_w = (tree.rect.width - tree.border.left - tree.border.right).max(0.0);
        let client_h = (tree.rect.height - tree.border.top - tree.border.bottom).max(0.0);
        let can_scroll_y = tree.scrollable_overflow_y || tree.scrollable_overflow;
        let show_v_bar = (tree.always_show_scrollbar_y
            || (can_scroll_y && tree.scroll_size.1 > client_h + 1.0))
            && client_w > 12.0
            && client_h > 12.0;
        let show_h_bar = (tree.always_show_scrollbar_x
            || (tree.scrollable_overflow_x && tree.scroll_size.0 > client_w + 1.0))
            && client_w > 12.0
            && client_h > 12.0;
        if show_v_bar {
            let track_w = 6.0;
            let track_x = client_x + client_w - track_w - 1.0;
            let h_pad = if show_h_bar { 8.0 } else { 2.0 };
            let track_len = (client_h - h_pad).max(1.0);
            items.push(DisplayItem::Rect {
                x: track_x,
                y: client_y + 1.0,
                width: track_w,
                height: track_len,
                color: Color {
                    r: 28,
                    g: 34,
                    b: 46,
                    a: 110,
                },
            });
            let content_h = tree.scroll_size.1.max(client_h).max(1.0);
            let ratio = (client_h / content_h).clamp(0.1, 1.0);
            let thumb_h = (track_len * ratio).clamp(12.0_f32.min(track_len), track_len.max(1.0));
            let max_scroll = (content_h - client_h).max(1.0);
            let scroll_frac = (tree.scroll_offset.1 / max_scroll).clamp(0.0, 1.0);
            let thumb_y = client_y + 1.0 + ((track_len - thumb_h).max(0.0) * scroll_frac);
            items.push(DisplayItem::RoundedRect {
                x: track_x,
                y: thumb_y,
                width: track_w,
                height: thumb_h,
                radius: 3.0,
                border_width: 0.0,
                fill_color: Color {
                    r: 120,
                    g: 136,
                    b: 164,
                    a: 190,
                },
                border_color: Color {
                    r: 0,
                    g: 0,
                    b: 0,
                    a: 0,
                },
            });
        }
        if show_h_bar {
            let track_h = 6.0;
            let track_y = client_y + client_h - track_h - 1.0;
            let v_pad = if show_v_bar { 8.0 } else { 2.0 };
            let track_len = (client_w - v_pad).max(1.0);
            items.push(DisplayItem::Rect {
                x: client_x + 1.0,
                y: track_y,
                width: track_len,
                height: track_h,
                color: Color {
                    r: 28,
                    g: 34,
                    b: 46,
                    a: 110,
                },
            });
            let content_w = tree.scroll_size.0.max(client_w).max(1.0);
            let ratio = (client_w / content_w).clamp(0.1, 1.0);
            let thumb_w = (track_len * ratio).clamp(12.0_f32.min(track_len), track_len.max(1.0));
            let max_scroll = (content_w - client_w).max(1.0);
            let scroll_frac = (tree.scroll_offset.0 / max_scroll).clamp(0.0, 1.0);
            let thumb_x = client_x + 1.0 + ((track_len - thumb_w).max(0.0) * scroll_frac);
            items.push(DisplayItem::RoundedRect {
                x: thumb_x,
                y: track_y,
                width: thumb_w,
                height: track_h,
                radius: 3.0,
                border_width: 0.0,
                fill_color: Color {
                    r: 120,
                    g: 136,
                    b: 164,
                    a: 190,
                },
                border_color: Color {
                    r: 0,
                    g: 0,
                    b: 0,
                    a: 0,
                },
            });
        }
        if rounded_clipped {
            items.push(DisplayItem::PopRoundedClip);
        } else {
            items.push(DisplayItem::PopClip);
        }
    }
    paint_outline(tree, items);
    let layer_end = items.len();
    if layer_end > layer_start {
        let eff_radii = if any_radius && !uniform_radii {
            radii
        } else {
            [tree.border_radius.max(0.0); 4]
        };
        let effects = [
            tree.sticky.map(LayerEffect::Sticky),
            tree.transform.map(LayerEffect::Transform),
            (tree.backdrop_blur >= 0.5 && tree.rect.width > 0.0 && tree.rect.height > 0.0)
                .then_some(LayerEffect::BackdropBlur {
                    rect: tree.rect,
                    radius: tree.backdrop_blur.min(64.0),
                    radii: eff_radii,
                }),
            (tree.filter_blur >= 0.5)
                .then_some(LayerEffect::FilterBlur(tree.filter_blur.min(64.0))),
            (tree.opacity < 1.0).then_some(LayerEffect::Opacity(tree.opacity.max(0.0))),
        ];
        for effect in effects.into_iter().flatten() {
            let layer = Layer {
                start: layer_start,
                end: layer_end,
                effect,
            };
            layers.push((depth, layers.len(), layer));
        }
    }
    if is_fixed {
        items.push(DisplayItem::PopFixed);
    }
}

/// Paint CSS `background-image: url(...)` inside the padding box according to
/// `background-size`, `background-position`, and `background-repeat`.
fn paint_background_image(tree: &LayoutBox, items: &mut Vec<DisplayItem>) {
    let Some(bg_img) = &tree.background_image else {
        return;
    };
    if bg_img.width == 0 || bg_img.height == 0 {
        return;
    }
    let px = tree.rect.x + tree.border.left;
    let py = tree.rect.y + tree.border.top;
    let pw = (tree.rect.width - tree.border.left - tree.border.right).max(0.0);
    let ph = (tree.rect.height - tree.border.top - tree.border.bottom).max(0.0);
    if pw <= 0.0 || ph <= 0.0 {
        return;
    }
    let img_w = (bg_img.width as f32).max(1.0);
    let img_h = (bg_img.height as f32).max(1.0);
    let (tile_w, tile_h) = if let Some((opt_w, opt_h)) = tree.background_size_px {
        match (opt_w, opt_h) {
            (Some(w), Some(h)) => (w.max(1.0), h.max(1.0)),
            (Some(w), None) => {
                let w = w.max(1.0);
                (w, (img_h * (w / img_w)).max(1.0))
            }
            (None, Some(h)) => {
                let h = h.max(1.0);
                ((img_w * (h / img_h)).max(1.0), h)
            }
            (None, None) => (img_w, img_h),
        }
    } else {
        match tree.background_size {
            ObjectFit::Cover => {
                let s = (pw / img_w).max(ph / img_h);
                (img_w * s, img_h * s)
            }
            ObjectFit::Contain => {
                let s = (pw / img_w).min(ph / img_h);
                (img_w * s, img_h * s)
            }
            ObjectFit::ScaleDown => {
                let s = (pw / img_w).min(ph / img_h).min(1.0);
                (img_w * s, img_h * s)
            }
            ObjectFit::None => (img_w, img_h),
            ObjectFit::Fill => {
                if tree.background_repeat == BackgroundRepeat::NoRepeat {
                    (pw, ph)
                } else {
                    (img_w, img_h)
                }
            }
        }
    };
    if tile_w <= 0.0 || tile_h <= 0.0 {
        return;
    }
    let origin_x = px + (pw - tile_w) * tree.background_position.0 + tree.background_position_px.0;
    let origin_y = py + (ph - tile_h) * tree.background_position.1 + tree.background_position_px.1;
    let repeat_x = matches!(
        tree.background_repeat,
        BackgroundRepeat::Repeat | BackgroundRepeat::RepeatX
    );
    let repeat_y = matches!(
        tree.background_repeat,
        BackgroundRepeat::Repeat | BackgroundRepeat::RepeatY
    );
    let clip_radius = (tree.border_radius - tree.border.top).max(0.0);

    if !repeat_x && !repeat_y {
        let overflows = origin_x < px - 0.5
            || origin_y < py - 0.5
            || origin_x + tile_w > px + pw + 0.5
            || origin_y + tile_h > py + ph + 0.5;
        let need_clip = overflows || clip_radius > 0.0;
        if need_clip {
            if clip_radius > 0.0 {
                items.push(DisplayItem::PushRoundedClip {
                    x: px,
                    y: py,
                    width: pw,
                    height: ph,
                    radius: clip_radius,
                });
            } else {
                items.push(DisplayItem::PushClip {
                    x: px,
                    y: py,
                    width: pw,
                    height: ph,
                });
            }
        }
        items.push(DisplayItem::Image {
            x: origin_x,
            y: origin_y,
            width: tile_w,
            height: tile_h,
            border_radius: if need_clip { 0.0 } else { clip_radius },
            image: bg_img.clone(),
        });
        if need_clip {
            if clip_radius > 0.0 {
                items.push(DisplayItem::PopRoundedClip);
            } else {
                items.push(DisplayItem::PopClip);
            }
        }
        return;
    }

    // Synthesize a single tiled RGBA surface capped at 1024x1024 so repeating
    // patterns emit one DisplayItem::Image without seams or wire blowup.
    let dst_w = pw.round().clamp(1.0, 1024.0) as u32;
    let dst_h = ph.round().clamp(1.0, 1024.0) as u32;
    let scale_x = dst_w as f32 / pw;
    let scale_y = dst_h as f32 / ph;
    let tw_u32 = (tile_w * scale_x).round().clamp(1.0, 1024.0) as u32;
    let th_u32 = (tile_h * scale_y).round().clamp(1.0, 1024.0) as u32;
    let tile_scaled = bg_img.resize_bilinear(tw_u32, th_u32);
    if tile_scaled.width == 0
        || tile_scaled.height == 0
        || tile_scaled.pixels.len() < (tile_scaled.width as usize * tile_scaled.height as usize * 4)
    {
        return;
    }
    let off_x = ((origin_x - px) * scale_x).round() as i32;
    let off_y = ((origin_y - py) * scale_y).round() as i32;
    let tw_i = tile_scaled.width as i32;
    let th_i = tile_scaled.height as i32;
    let mut out_px = vec![0u8; (dst_w as usize) * (dst_h as usize) * 4];
    for y in 0..(dst_h as i32) {
        let rel_y = y - off_y;
        let sy = if repeat_y {
            rel_y.rem_euclid(th_i)
        } else if rel_y >= 0 && rel_y < th_i {
            rel_y
        } else {
            continue;
        };
        let src_row_off = (sy as usize) * (tw_i as usize) * 4;
        let dst_row_off = (y as usize) * (dst_w as usize) * 4;
        for x in 0..(dst_w as i32) {
            let rel_x = x - off_x;
            let sx = if repeat_x {
                rel_x.rem_euclid(tw_i)
            } else if rel_x >= 0 && rel_x < tw_i {
                rel_x
            } else {
                continue;
            };
            let s_idx = src_row_off + (sx as usize) * 4;
            let d_idx = dst_row_off + (x as usize) * 4;
            out_px[d_idx..d_idx + 4].copy_from_slice(&tile_scaled.pixels[s_idx..s_idx + 4]);
        }
    }
    if let Ok(tiled) = Image::from_rgba(dst_w, dst_h, out_px) {
        items.push(DisplayItem::Image {
            x: px,
            y: py,
            width: pw,
            height: ph,
            border_radius: clip_radius,
            image: tiled,
        });
    }
}

fn paint_borders_clipped_if_rounded(
    tree: &LayoutBox,
    any_radius: bool,
    items: &mut Vec<DisplayItem>,
) {
    let max_rad = tree
        .border_radius
        .max(tree.border_radii[0])
        .max(tree.border_radii[1])
        .max(tree.border_radii[2])
        .max(tree.border_radii[3]);
    let clip_round = any_radius
        && max_rad > 0.0
        && (tree.border.top > 0.0
            || tree.border.right > 0.0
            || tree.border.bottom > 0.0
            || tree.border.left > 0.0);
    if clip_round {
        items.push(DisplayItem::PushRoundedClip {
            x: tree.rect.x,
            y: tree.rect.y,
            width: tree.rect.width,
            height: tree.rect.height,
            radius: max_rad,
        });
    }
    paint_borders(tree, items);
    if clip_round {
        items.push(DisplayItem::PopRoundedClip);
    }
}

/// Paint each positive border side in the box's y-down space, honoring
/// `border-style` (`solid`, `dashed`, `dotted`, `double`, `none`, `hidden`).
fn paint_borders(tree: &LayoutBox, items: &mut Vec<DisplayItem>) {
    let b = tree.border;
    let r = tree.rect;
    let colors = tree.border_colors;
    let styles = tree.border_styles;
    if r.width <= 0.0 || r.height <= 0.0 {
        return;
    }
    paint_border_edge(
        items,
        r.x,
        r.y,
        r.width,
        b.top.min(r.height),
        true,
        colors[0],
        styles[0],
    );
    let bh = b.bottom.min(r.height);
    paint_border_edge(
        items,
        r.x,
        r.y + r.height - bh,
        r.width,
        bh,
        true,
        colors[2],
        styles[2],
    );
    paint_border_edge(
        items,
        r.x,
        r.y,
        b.left.min(r.width),
        r.height,
        false,
        colors[3],
        styles[3],
    );
    let rw = b.right.min(r.width);
    paint_border_edge(
        items,
        r.x + r.width - rw,
        r.y,
        rw,
        r.height,
        false,
        colors[1],
        styles[1],
    );
}

#[allow(clippy::too_many_arguments)]
fn paint_border_edge(
    items: &mut Vec<DisplayItem>,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    horizontal: bool,
    color: Color,
    style: BorderStyle,
) {
    if width <= 0.0 || height <= 0.0 || color.a == 0 {
        return;
    }
    match style {
        BorderStyle::None | BorderStyle::Hidden => {}
        BorderStyle::Solid => {
            items.push(DisplayItem::Rect {
                x,
                y,
                width,
                height,
                color,
            });
        }
        BorderStyle::Double => {
            let thickness = if horizontal { height } else { width };
            if thickness < 3.0 {
                items.push(DisplayItem::Rect {
                    x,
                    y,
                    width,
                    height,
                    color,
                });
                return;
            }
            let line_t = (thickness / 3.0).max(1.0);
            if horizontal {
                items.push(DisplayItem::Rect {
                    x,
                    y,
                    width,
                    height: line_t,
                    color,
                });
                items.push(DisplayItem::Rect {
                    x,
                    y: y + height - line_t,
                    width,
                    height: line_t,
                    color,
                });
            } else {
                items.push(DisplayItem::Rect {
                    x,
                    y,
                    width: line_t,
                    height,
                    color,
                });
                items.push(DisplayItem::Rect {
                    x: x + width - line_t,
                    y,
                    width: line_t,
                    height,
                    color,
                });
            }
        }
        BorderStyle::Dashed => {
            let (len, thickness) = if horizontal {
                (width, height)
            } else {
                (height, width)
            };
            let dash = (thickness * 3.0).clamp(4.0, 24.0);
            let gap = (thickness * 2.0).clamp(3.0, 16.0);
            let step = (dash + gap).max(len / 128.0);
            let mut pos = 0.0;
            let mut count = 0usize;
            while pos < len && count < 128 {
                let seg = dash.min(len - pos);
                if horizontal {
                    items.push(DisplayItem::Rect {
                        x: x + pos,
                        y,
                        width: seg,
                        height,
                        color,
                    });
                } else {
                    items.push(DisplayItem::Rect {
                        x,
                        y: y + pos,
                        width,
                        height: seg,
                        color,
                    });
                }
                pos += step;
                count += 1;
            }
        }
        BorderStyle::Dotted => {
            let (len, thickness) = if horizontal {
                (width, height)
            } else {
                (height, width)
            };
            let dot = thickness.clamp(1.5, 12.0);
            let gap = thickness.clamp(2.0, 12.0);
            let step = (dot + gap).max(len / 128.0);
            let mut pos = 0.0;
            let mut count = 0usize;
            while pos < len && count < 128 {
                let seg = dot.min(len - pos);
                let (dx, dy, dw, dh) = if horizontal {
                    (x + pos, y, seg, height)
                } else {
                    (x, y + pos, width, seg)
                };
                if thickness >= 2.0 {
                    items.push(DisplayItem::RoundedRect {
                        x: dx,
                        y: dy,
                        width: dw,
                        height: dh,
                        radius: thickness * 0.5,
                        border_width: 0.0,
                        fill_color: color,
                        border_color: Color {
                            r: 0,
                            g: 0,
                            b: 0,
                            a: 0,
                        },
                    });
                } else {
                    items.push(DisplayItem::Rect {
                        x: dx,
                        y: dy,
                        width: dw,
                        height: dh,
                        color,
                    });
                }
                pos += step;
                count += 1;
            }
        }
    }
}

/// Paint `outline` around the border box offset by `outline_offset`.
fn paint_outline(tree: &LayoutBox, items: &mut Vec<DisplayItem>) {
    let w = tree.outline_width;
    let color = tree.outline_color;
    let r = tree.rect;
    if w <= 0.0 || color.a == 0 || r.width <= 0.0 || r.height <= 0.0 {
        return;
    }
    let off = tree.outline_offset;
    let ox = r.x - off - w;
    let oy = r.y - off - w;
    let ow = (r.width + (off + w) * 2.0).max(w * 2.0);
    let oh = (r.height + (off + w) * 2.0).max(w * 2.0);
    items.push(DisplayItem::Rect {
        x: ox,
        y: oy,
        width: ow,
        height: w,
        color,
    });
    items.push(DisplayItem::Rect {
        x: ox,
        y: oy + oh - w,
        width: ow,
        height: w,
        color,
    });
    items.push(DisplayItem::Rect {
        x: ox,
        y: oy + w,
        width: w,
        height: (oh - w * 2.0).max(0.0),
        color,
    });
    items.push(DisplayItem::Rect {
        x: ox + ow - w,
        y: oy + w,
        width: w,
        height: (oh - w * 2.0).max(0.0),
        color,
    });
}

#[cfg(test)]
mod tests {
    //! One box with a fill, four border strips, a text run, and an underline.

    use super::*;
    use anvil_layout::{Edge, NodeId, Rect, TextRun};

    fn sample_box() -> LayoutBox {
        LayoutBox {
            node: NodeId::from_raw(1),
            rect: Rect {
                x: 1.0,
                y: 2.0,
                width: 30.0,
                height: 10.0,
            },
            position: Position::Static,
            z_index: 0,
            overflow: Overflow::Visible,
            overflow_x: Overflow::Visible,
            overflow_y: Overflow::Visible,
            background: Some(Color::BLACK),
            background_gradient: None,
            background_image: None,
            background_size: ObjectFit::Fill,
            background_size_px: None,
            background_repeat: BackgroundRepeat::Repeat,
            background_position: (0.5, 0.5),
            background_position_px: (0.0, 0.0),
            box_shadow: None,
            border: Edge::all(1.0),
            border_color: Color::BLACK,
            border_colors: [Color::BLACK; 4],
            border_styles: [BorderStyle::Solid; 4],
            border_radius: 0.0,
            border_radii: [0.0; 4],
            outline_width: 0.0,
            outline_color: Color::BLACK,
            outline_offset: 0.0,
            scrollable_overflow: false,
            scrollable_overflow_x: false,
            scrollable_overflow_y: false,
            always_show_scrollbar_x: false,
            always_show_scrollbar_y: false,
            scroll_offset: (0.0, 0.0),
            scroll_size: (30.0, 10.0),
            object_fit: ObjectFit::Fill,
            image: None,
            backdrop_blur: 0.0,
            filter_blur: 0.0,
            sticky: None,
            transform: None,
            opacity: 1.0,
            text: vec![TextRun {
                node: NodeId::from_raw(2),
                text: "Hi".to_string(),
                x: 1.0,
                y: 2.0,
                width: 10.0,
                font_size: 10.0,
                font_family: anvil_layout::FontFamily::SansSerif,
                font_weight: 400,
                italic: false,
                letter_spacing: 0.0,
                color: Color::BLACK,
                underline: true,
                line_through: false,
            }],
            children: Vec::new(),
        }
    }

    #[test]
    fn paints_background_borders_text_and_underline() {
        let tree = sample_box();
        let list = paint(&tree);
        assert!(matches!(list.items[0], DisplayItem::Rect { .. }));
        assert!(
            list.items
                .iter()
                .any(|item| matches!(item, DisplayItem::Text { text, .. } if text == "Hi"))
        );
        assert!(list.items.len() >= 7);
    }

    #[test]
    fn paints_asymmetric_radii_and_backdrop_blur_layer() {
        let mut tree = sample_box();
        tree.border_radius = 12.0;
        tree.border_radii = [12.0, 0.0, 12.0, 0.0];
        tree.backdrop_blur = 8.0;
        tree.filter_blur = 3.0;
        let list = paint(&tree);
        assert!(
            list.items
                .iter()
                .any(|it| matches!(it, DisplayItem::GradientRect { stop_count: 1, .. }))
        );
        assert!(
            list.layers
                .iter()
                .any(|l| matches!(l.effect, LayerEffect::BackdropBlur { .. }))
        );
        assert!(
            list.layers
                .iter()
                .any(|l| matches!(l.effect, LayerEffect::FilterBlur(_)))
        );
    }

    #[test]
    fn paints_dashed_border_and_horizontal_scrollbar() {
        let mut tree = sample_box();
        tree.rect = Rect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 60.0,
        };
        tree.border = Edge::all(2.0);
        tree.border_styles = [BorderStyle::Dashed; 4];
        tree.overflow = Overflow::Hidden;
        tree.overflow_x = Overflow::Hidden;
        tree.scrollable_overflow_x = true;
        tree.scroll_size = (300.0, 50.0);
        tree.scroll_offset = (75.0, 0.0);
        let list = paint(&tree);
        assert!(list.items.len() > 12);
    }
}
