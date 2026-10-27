// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Display list frames from a renderer.
//!
//! A frame is what a renderer will send instead of HTML: the page's display
//! list, the page size, and where its links are. The browser draws it
//! without parsing anything a page wrote, so this format and its checks are
//! what stands between a hostile renderer and the browser's rasterizer.
//!
//! The layout is a fixed list of fields. Numbers are little-endian, `f32`
//! values are their raw bits, and a bool is one byte, 0 or 1:
//!
//! | Field | Encoding |
//! | --- | --- |
//! | page width, page height | `f32`, `f32` |
//! | cut | bool, set when the renderer dropped part of the page to fit |
//! | items | `u32` count, then each item: a tag byte and its fields |
//! | layers | `u32` count, then each: `u32` start, `u32` end, a tag byte and its fields |
//! | links | `u32` count, then each: four `f32`, a `u32` length with the URL, and its moves |
//!
//! Text is a `u32` length and UTF-8. An image is its `u32` width and height
//! followed by exactly `width * height * 4` bytes of RGBA. An optional `f32`
//! is a bool, then the value only when the bool is 1. A link's moves are a
//! count byte, then each move as a tag byte and its fields: sticky and
//! transform as in a layer, and fixed with no fields.
//!
//! [`decode`] is strict. Every count and length is checked against its cap
//! and against the bytes that are left before anything is set aside for it.
//! Unknown tags, a bool that isn't 0 or 1, a number that isn't finite and
//! spare bytes are all refused, so a frame that decodes encodes back to the
//! same bytes. After that, [`check`] applies the rules on the values: every
//! number in its range, clips and fixed layers balanced and not too deep,
//! layers sorted and nested, text and image totals under their caps, and
//! links that stay on the page with an `http` or `https` URL.
//!
//! Text runs are strings with a position, colour and size. The browser draws
//! them with system fonts, so no font file from a page is ever read here.
//!
//! [`Frame::build`] is the renderer's side. It drops anything that would
//! fail [`check`] and cuts a page that doesn't fit the caps, so an honest
//! renderer never sends a frame the browser refuses.

#![forbid(unsafe_code)]

use std::fmt;

use anvil_image::Image;
use anvil_layout::{Color, Edge};
use anvil_paint::{
    DisplayItem, DisplayList, Layer, LayerEffect, Rect, StickyConstraint, Transform2D,
};
use anvil_sandbox::wire::MAX_URL_BYTES;

/// Largest encoded frame (32 MiB), the most a wire `Frame` message carries.
/// The caps below add up to less than this.
pub const MAX_FRAME_BYTES: usize = anvil_sandbox::wire::MAX_DISPLAY_LIST_BYTES;

/// Most display items in one frame.
pub const MAX_ITEMS: usize = 131_072;

/// Most layers in one frame.
pub const MAX_LAYERS: usize = 16_384;

/// Most moves on one link: the fixed, sticky and transformed boxes it sits in.
pub const MAX_LINK_MOVES: usize = 8;

/// Most link regions in one frame.
pub const MAX_LINKS: usize = 8_192;

/// Most URL bytes over all link regions (1 MiB).
pub const MAX_LINK_URL_BYTES: usize = 1024 * 1024;

/// Deepest nesting of clips and fixed ranges together, and separately of
/// layers.
pub const MAX_NESTING: usize = 64;

/// Longest single text run, in bytes.
pub const MAX_TEXT_RUN_BYTES: usize = 64 * 1024;

/// Most text bytes over all runs (4 MiB).
pub const MAX_TEXT_BYTES: usize = 4 * 1024 * 1024;

/// Largest image side, in pixels.
pub const MAX_IMAGE_SIDE: u32 = 4_096;

/// Most RGBA bytes over all images (16 MiB). One image may use all of it.
pub const MAX_IMAGE_BYTES: usize = 16 * 1024 * 1024;

/// Widest page, in CSS pixels.
pub const MAX_PAGE_WIDTH: f32 = 65_536.0;

/// Tallest page, in CSS pixels. A longer page is cut here.
pub const MAX_PAGE_HEIGHT: f32 = 1_048_576.0;

/// Largest distance from the page origin for any position or length.
pub const MAX_COORD: f32 = 2_097_152.0;

/// Largest shadow offset or spread, either way.
pub const MAX_SHADOW_SHIFT: f32 = 4_096.0;

/// Largest shadow blur radius. Blur costs time in the rasterizer.
pub const MAX_BLUR: f32 = 1_024.0;

/// Largest gradient angle, either way, in degrees.
pub const MAX_ANGLE: f32 = 36_000.0;

/// Smallest and largest font size, in CSS pixels.
pub const MIN_FONT_SIZE: f32 = 1.0;
pub const MAX_FONT_SIZE: f32 = 512.0;

/// Largest scale or skew term in a layer transform, either way.
pub const MAX_TRANSFORM_SCALE: f32 = 64.0;

const TAG_PUSH_CLIP: u8 = 1;
const TAG_POP_CLIP: u8 = 2;
const TAG_PUSH_FIXED: u8 = 3;
const TAG_POP_FIXED: u8 = 4;
const TAG_BOX_SHADOW: u8 = 5;
const TAG_GRADIENT: u8 = 6;
const TAG_RECT: u8 = 7;
const TAG_ROUNDED_RECT: u8 = 8;
const TAG_IMAGE: u8 = 9;
const TAG_TEXT: u8 = 10;

const TAG_STICKY: u8 = 1;
const TAG_TRANSFORM: u8 = 2;
const TAG_OPACITY: u8 = 3;
const TAG_FIXED: u8 = 4;

/// A rectangle on the page that opens a URL when clicked.
#[derive(Clone, Debug, PartialEq)]
pub struct LinkRegion {
    /// Where the link's box is laid out, before any move.
    pub rect: Rect,
    /// Always `http://` or `https://`, visible ASCII only.
    pub url: String,
    /// How the boxes around the link move it when drawn, outermost first.
    /// Empty for a link in the normal flow of the page.
    pub moves: Vec<LinkMove>,
}

/// One way a box moves what is inside it when the page is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LinkMove {
    /// A `position: fixed` box, laid out against the viewport, so it stays
    /// where it is while the page scrolls.
    Fixed,
    /// A sticky box, shifted for the scroll position.
    Sticky(StickyConstraint),
    /// A transformed box, drawn through this map.
    Transform(Transform2D),
}

impl LinkRegion {
    /// Whether the page point `(x, y)` is on this link as drawn. `view` is
    /// the visible part of the page in page space, as for sticky layers.
    ///
    /// The point is moved back through each move, the same way the layout
    /// hit test does it, and then tested against the laid-out rectangle.
    pub fn contains(&self, x: f32, y: f32, view: Rect) -> bool {
        let (mut x, mut y, mut view) = (x, y, view);
        for step in &self.moves {
            match step {
                LinkMove::Fixed => {
                    x -= view.x;
                    y -= view.y;
                    view = Rect {
                        x: 0.0,
                        y: 0.0,
                        ..view
                    };
                }
                LinkMove::Sticky(sticky) => {
                    let (dx, dy) = sticky.offset(view);
                    x -= dx;
                    y -= dy;
                    view = Rect {
                        x: view.x - dx,
                        y: view.y - dy,
                        ..view
                    };
                }
                LinkMove::Transform(m) => match m.inverse() {
                    Some(inverse) => (x, y) = inverse.apply(x, y),
                    None => return false,
                },
            }
        }
        let r = &self.rect;
        x >= r.x && y >= r.y && x < r.x + r.width && y < r.y + r.height
    }
}

/// One painted page.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub page_width: f32,
    pub page_height: f32,
    /// The renderer left part of the page out to stay under the caps.
    pub cut: bool,
    pub list: DisplayList,
    pub links: Vec<LinkRegion>,
}

/// Why a frame was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrameError {
    /// The bytes ended in the middle of a field.
    Truncated,
    /// Bytes are left over after the last field.
    TrailingBytes,
    /// The encoded frame is over [`MAX_FRAME_BYTES`].
    TooLarge,
    /// A list has more entries than its cap.
    TooMany(&'static str),
    /// A text field or a total is over its cap.
    TooLong(&'static str),
    /// A text field is not UTF-8.
    BadUtf8(&'static str),
    /// A tag byte with no meaning.
    BadTag(&'static str),
    /// A bool byte that is not 0 or 1.
    BadBool(&'static str),
    /// A number that is NaN or infinite.
    NotFinite(&'static str),
    /// A number outside its range.
    OutOfRange(&'static str),
    /// A pop without its push, a push without its pop, or nesting too deep.
    Unbalanced,
    /// Layers out of order, overlapping without nesting, or past the items.
    BadLayer,
    /// A link with a bad URL or a rectangle off the page.
    BadLink,
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => write!(f, "frame ended early"),
            Self::TrailingBytes => write!(f, "frame has spare bytes"),
            Self::TooLarge => write!(f, "frame is too large"),
            Self::TooMany(field) => write!(f, "{field} has too many entries"),
            Self::TooLong(field) => write!(f, "{field} is too long"),
            Self::BadUtf8(field) => write!(f, "{field} is not UTF-8"),
            Self::BadTag(field) => write!(f, "{field} has an unknown tag"),
            Self::BadBool(field) => write!(f, "{field} is not 0 or 1"),
            Self::NotFinite(field) => write!(f, "{field} is not a finite number"),
            Self::OutOfRange(field) => write!(f, "{field} is out of range"),
            Self::Unbalanced => write!(f, "clips or fixed ranges do not balance"),
            Self::BadLayer => write!(f, "layers are out of order or overlap"),
            Self::BadLink => write!(f, "a link is off the page or has a bad URL"),
        }
    }
}

impl std::error::Error for FrameError {}

// ---------------------------------------------------------------------------
// Rules

fn finite(v: f32, field: &'static str) -> Result<f32, FrameError> {
    if v.is_finite() {
        Ok(v)
    } else {
        Err(FrameError::NotFinite(field))
    }
}

fn within(v: f32, low: f32, high: f32, field: &'static str) -> Result<(), FrameError> {
    if (low..=high).contains(&finite(v, field)?) {
        Ok(())
    } else {
        Err(FrameError::OutOfRange(field))
    }
}

fn position(v: f32, field: &'static str) -> Result<(), FrameError> {
    within(v, -MAX_COORD, MAX_COORD, field)
}

fn length(v: f32, field: &'static str) -> Result<(), FrameError> {
    within(v, 0.0, MAX_COORD, field)
}

fn check_box(x: f32, y: f32, width: f32, height: f32) -> Result<(), FrameError> {
    position(x, "x")?;
    position(y, "y")?;
    length(width, "width")?;
    length(height, "height")
}

fn check_rect(r: &Rect) -> Result<(), FrameError> {
    check_box(r.x, r.y, r.width, r.height)
}

/// What one item adds to the frame's text and image totals.
#[derive(Clone, Copy, Default)]
struct Cost {
    text: usize,
    image: usize,
}

/// The value rules for one item, without the balance and total rules.
fn check_item(item: &DisplayItem) -> Result<Cost, FrameError> {
    let mut cost = Cost::default();
    match item {
        DisplayItem::PushClip {
            x,
            y,
            width,
            height,
        } => check_box(*x, *y, *width, *height)?,
        DisplayItem::PopClip | DisplayItem::PushFixed | DisplayItem::PopFixed => {}
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
            color: _,
        } => {
            check_box(*x, *y, *width, *height)?;
            within(*offset_x, -MAX_SHADOW_SHIFT, MAX_SHADOW_SHIFT, "offset_x")?;
            within(*offset_y, -MAX_SHADOW_SHIFT, MAX_SHADOW_SHIFT, "offset_y")?;
            within(*blur_radius, 0.0, MAX_BLUR, "blur_radius")?;
            within(
                *spread_radius,
                -MAX_SHADOW_SHIFT,
                MAX_SHADOW_SHIFT,
                "spread_radius",
            )?;
            length(*radius, "radius")?;
        }
        DisplayItem::LinearGradientRect {
            x,
            y,
            width,
            height,
            radius,
            border_width,
            angle_deg,
            ..
        } => {
            check_box(*x, *y, *width, *height)?;
            length(*radius, "radius")?;
            length(*border_width, "border_width")?;
            within(*angle_deg, -MAX_ANGLE, MAX_ANGLE, "angle_deg")?;
        }
        DisplayItem::Rect {
            x,
            y,
            width,
            height,
            color: _,
        } => check_box(*x, *y, *width, *height)?,
        DisplayItem::RoundedRect {
            x,
            y,
            width,
            height,
            radius,
            border_width,
            ..
        } => {
            check_box(*x, *y, *width, *height)?;
            length(*radius, "radius")?;
            length(*border_width, "border_width")?;
        }
        DisplayItem::Image {
            x,
            y,
            width,
            height,
            border_radius,
            image,
        } => {
            check_box(*x, *y, *width, *height)?;
            length(*border_radius, "border_radius")?;
            cost.image = image_bytes(image.width, image.height)?;
            if image.pixels.len() != cost.image {
                return Err(FrameError::OutOfRange("image pixels"));
            }
        }
        DisplayItem::Text {
            x,
            y,
            text,
            color: _,
            font_size,
        } => {
            position(*x, "x")?;
            position(*y, "y")?;
            within(*font_size, MIN_FONT_SIZE, MAX_FONT_SIZE, "font_size")?;
            if text.len() > MAX_TEXT_RUN_BYTES {
                return Err(FrameError::TooLong("text"));
            }
            cost.text = text.len();
        }
    }
    Ok(cost)
}

/// RGBA bytes for an image of this size, when the size is allowed.
fn image_bytes(width: u32, height: u32) -> Result<usize, FrameError> {
    if !(1..=MAX_IMAGE_SIDE).contains(&width) || !(1..=MAX_IMAGE_SIDE).contains(&height) {
        return Err(FrameError::OutOfRange("image size"));
    }
    let bytes = width as usize * height as usize * 4;
    if bytes > MAX_IMAGE_BYTES {
        return Err(FrameError::TooLong("image"));
    }
    Ok(bytes)
}

fn check_effect(effect: &LayerEffect) -> Result<(), FrameError> {
    match effect {
        LayerEffect::Sticky(c) => {
            check_rect(&c.rect)?;
            check_rect(&c.containing)?;
            for v in [c.margin.top, c.margin.right, c.margin.bottom, c.margin.left] {
                position(v, "sticky margin")?;
            }
            for v in [c.top, c.right, c.bottom, c.left].into_iter().flatten() {
                position(v, "sticky inset")?;
            }
            Ok(())
        }
        LayerEffect::Transform(m) => {
            for v in [m.a, m.b, m.c, m.d] {
                within(v, -MAX_TRANSFORM_SCALE, MAX_TRANSFORM_SCALE, "transform")?;
            }
            position(m.e, "transform")?;
            position(m.f, "transform")
        }
        LayerEffect::Opacity(o) => within(*o, 0.0, 1.0, "opacity"),
    }
}

/// A renderer may only link to `http://` or `https://` URLs made of visible
/// ASCII, the same rule as its fetches.
fn link_url_ok(url: &str) -> bool {
    (url.starts_with("http://") || url.starts_with("https://"))
        && url.len() <= MAX_URL_BYTES
        && url.bytes().all(|b| (0x21..=0x7e).contains(&b))
}

fn check_link(link: &LinkRegion, page_width: f32, page_height: f32) -> Result<(), FrameError> {
    let r = &link.rect;
    let on_page = [r.x, r.y, r.width, r.height].iter().all(|v| v.is_finite())
        && r.x >= 0.0
        && r.y >= 0.0
        && r.width >= 0.0
        && r.height >= 0.0
        && r.x + r.width <= page_width
        && r.y + r.height <= page_height;
    if !(on_page && link_url_ok(&link.url)) {
        return Err(FrameError::BadLink);
    }
    if link.moves.len() > MAX_LINK_MOVES {
        return Err(FrameError::TooMany("link moves"));
    }
    for step in &link.moves {
        match step {
            LinkMove::Fixed => {}
            LinkMove::Sticky(c) => check_effect(&LayerEffect::Sticky(*c))?,
            LinkMove::Transform(m) => check_effect(&LayerEffect::Transform(*m))?,
        }
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Open {
    Clip,
    Fixed,
}

/// Every rule on a whole frame. [`decode`] runs this, and [`encode`] runs it
/// too so neither side can make a frame the other refuses.
pub fn check(frame: &Frame) -> Result<(), FrameError> {
    within(frame.page_width, 1.0, MAX_PAGE_WIDTH, "page_width")?;
    within(frame.page_height, 1.0, MAX_PAGE_HEIGHT, "page_height")?;
    let items = &frame.list.items;
    if items.len() > MAX_ITEMS {
        return Err(FrameError::TooMany("items"));
    }
    let mut stack: Vec<Open> = Vec::new();
    let mut text = 0usize;
    let mut image = 0usize;
    for item in items {
        let cost = check_item(item)?;
        text += cost.text;
        image += cost.image;
        if text > MAX_TEXT_BYTES {
            return Err(FrameError::TooLong("text total"));
        }
        if image > MAX_IMAGE_BYTES {
            return Err(FrameError::TooLong("image total"));
        }
        let closes = match item {
            DisplayItem::PushClip { .. } => {
                stack.push(Open::Clip);
                None
            }
            DisplayItem::PushFixed => {
                stack.push(Open::Fixed);
                None
            }
            DisplayItem::PopClip => Some(Open::Clip),
            DisplayItem::PopFixed => Some(Open::Fixed),
            _ => None,
        };
        if closes.is_some() && stack.pop() != closes {
            return Err(FrameError::Unbalanced);
        }
        if stack.len() > MAX_NESTING {
            return Err(FrameError::Unbalanced);
        }
    }
    if !stack.is_empty() {
        return Err(FrameError::Unbalanced);
    }
    check_layers(&frame.list.layers, items.len())?;
    if frame.links.len() > MAX_LINKS {
        return Err(FrameError::TooMany("links"));
    }
    let mut url_bytes = 0usize;
    for link in &frame.links {
        check_link(link, frame.page_width, frame.page_height)?;
        url_bytes += link.url.len();
        if url_bytes > MAX_LINK_URL_BYTES {
            return Err(FrameError::TooLong("link URLs"));
        }
    }
    Ok(())
}

/// Layers must be sorted by start, outer before inner, inside the items, and
/// two layers either nest or don't overlap.
fn check_layers(layers: &[Layer], item_count: usize) -> Result<(), FrameError> {
    if layers.len() > MAX_LAYERS {
        return Err(FrameError::TooMany("layers"));
    }
    let mut open: Vec<usize> = Vec::new();
    let mut last_start = 0usize;
    for layer in layers {
        if layer.start > layer.end || layer.end > item_count || layer.start < last_start {
            return Err(FrameError::BadLayer);
        }
        last_start = layer.start;
        while open.last().is_some_and(|&end| end <= layer.start) {
            open.pop();
        }
        if open.last().is_some_and(|&end| layer.end > end) {
            return Err(FrameError::BadLayer);
        }
        open.push(layer.end);
        if open.len() > MAX_NESTING {
            return Err(FrameError::BadLayer);
        }
        check_effect(&layer.effect)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Encoding

/// Encode a frame. Refuses one that breaks a rule or is over
/// [`MAX_FRAME_BYTES`].
pub fn encode(frame: &Frame) -> Result<Vec<u8>, FrameError> {
    check(frame)?;
    let mut out = Vec::new();
    put_f32(&mut out, frame.page_width);
    put_f32(&mut out, frame.page_height);
    out.push(u8::from(frame.cut));
    put_u32(&mut out, frame.list.items.len() as u32);
    for item in &frame.list.items {
        put_item(&mut out, item);
        if out.len() > MAX_FRAME_BYTES {
            return Err(FrameError::TooLarge);
        }
    }
    put_u32(&mut out, frame.list.layers.len() as u32);
    for layer in &frame.list.layers {
        put_u32(&mut out, layer.start as u32);
        put_u32(&mut out, layer.end as u32);
        put_effect(&mut out, &layer.effect);
    }
    put_u32(&mut out, frame.links.len() as u32);
    for link in &frame.links {
        put_rect(&mut out, &link.rect);
        put_text(&mut out, &link.url);
        out.push(link.moves.len() as u8);
        for step in &link.moves {
            match step {
                LinkMove::Fixed => out.push(TAG_FIXED),
                LinkMove::Sticky(c) => put_effect(&mut out, &LayerEffect::Sticky(*c)),
                LinkMove::Transform(m) => put_effect(&mut out, &LayerEffect::Transform(*m)),
            }
        }
    }
    if out.len() > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge);
    }
    Ok(out)
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_f32(out: &mut Vec<u8>, v: f32) {
    out.extend_from_slice(&v.to_bits().to_le_bytes());
}

fn put_f32s(out: &mut Vec<u8>, values: &[f32]) {
    for v in values {
        put_f32(out, *v);
    }
}

fn put_color(out: &mut Vec<u8>, c: Color) {
    out.extend_from_slice(&[c.r, c.g, c.b, c.a]);
}

fn put_text(out: &mut Vec<u8>, text: &str) {
    put_u32(out, text.len() as u32);
    out.extend_from_slice(text.as_bytes());
}

fn put_rect(out: &mut Vec<u8>, r: &Rect) {
    put_f32s(out, &[r.x, r.y, r.width, r.height]);
}

fn put_option(out: &mut Vec<u8>, v: Option<f32>) {
    match v {
        Some(v) => {
            out.push(1);
            put_f32(out, v);
        }
        None => out.push(0),
    }
}

fn put_item(out: &mut Vec<u8>, item: &DisplayItem) {
    match item {
        DisplayItem::PushClip {
            x,
            y,
            width,
            height,
        } => {
            out.push(TAG_PUSH_CLIP);
            put_f32s(out, &[*x, *y, *width, *height]);
        }
        DisplayItem::PopClip => out.push(TAG_POP_CLIP),
        DisplayItem::PushFixed => out.push(TAG_PUSH_FIXED),
        DisplayItem::PopFixed => out.push(TAG_POP_FIXED),
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
            out.push(TAG_BOX_SHADOW);
            put_f32s(
                out,
                &[
                    *x,
                    *y,
                    *width,
                    *height,
                    *offset_x,
                    *offset_y,
                    *blur_radius,
                    *spread_radius,
                    *radius,
                ],
            );
            put_color(out, *color);
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
            out.push(TAG_GRADIENT);
            put_f32s(
                out,
                &[*x, *y, *width, *height, *radius, *border_width, *angle_deg],
            );
            put_color(out, *start_color);
            put_color(out, *end_color);
            put_color(out, *border_color);
        }
        DisplayItem::Rect {
            x,
            y,
            width,
            height,
            color,
        } => {
            out.push(TAG_RECT);
            put_f32s(out, &[*x, *y, *width, *height]);
            put_color(out, *color);
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
            out.push(TAG_ROUNDED_RECT);
            put_f32s(out, &[*x, *y, *width, *height, *radius, *border_width]);
            put_color(out, *fill_color);
            put_color(out, *border_color);
        }
        DisplayItem::Image {
            x,
            y,
            width,
            height,
            border_radius,
            image,
        } => {
            out.push(TAG_IMAGE);
            put_f32s(out, &[*x, *y, *width, *height, *border_radius]);
            put_u32(out, image.width);
            put_u32(out, image.height);
            out.extend_from_slice(&image.pixels);
        }
        DisplayItem::Text {
            x,
            y,
            text,
            color,
            font_size,
        } => {
            out.push(TAG_TEXT);
            put_f32s(out, &[*x, *y]);
            put_color(out, *color);
            put_f32(out, *font_size);
            put_text(out, text);
        }
    }
}

fn put_effect(out: &mut Vec<u8>, effect: &LayerEffect) {
    match effect {
        LayerEffect::Sticky(c) => {
            out.push(TAG_STICKY);
            put_rect(out, &c.rect);
            put_f32s(
                out,
                &[c.margin.top, c.margin.right, c.margin.bottom, c.margin.left],
            );
            put_rect(out, &c.containing);
            for v in [c.top, c.right, c.bottom, c.left] {
                put_option(out, v);
            }
        }
        LayerEffect::Transform(m) => {
            out.push(TAG_TRANSFORM);
            put_f32s(out, &[m.a, m.b, m.c, m.d, m.e, m.f]);
        }
        LayerEffect::Opacity(o) => {
            out.push(TAG_OPACITY);
            put_f32(out, *o);
        }
    }
}

// ---------------------------------------------------------------------------
// Decoding

/// Decode and check a frame from a renderer.
///
/// Nothing is drawn from a frame that hasn't passed through here.
pub fn decode(bytes: &[u8]) -> Result<Frame, FrameError> {
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge);
    }
    let mut r = Reader { bytes, at: 0 };
    let page_width = r.f32("page_width")?;
    let page_height = r.f32("page_height")?;
    let cut = r.bool("cut")?;
    // Each item is at least one byte, each layer nine and each link 21,
    // so a count larger than what is left can be refused before the list is
    // made.
    let count = r.count(MAX_ITEMS, 1, "items")?;
    let mut items = Vec::with_capacity(count);
    let mut text = 0usize;
    let mut image = 0usize;
    for _ in 0..count {
        let item = r.item(&mut text, &mut image)?;
        items.push(item);
    }
    let count = r.count(MAX_LAYERS, 9, "layers")?;
    let mut layers = Vec::with_capacity(count);
    for _ in 0..count {
        let start = r.u32()? as usize;
        let end = r.u32()? as usize;
        let effect = r.effect()?;
        layers.push(Layer { start, end, effect });
    }
    let count = r.count(MAX_LINKS, 21, "links")?;
    let mut links = Vec::with_capacity(count);
    for _ in 0..count {
        let rect = r.rect()?;
        let url = r.text(MAX_URL_BYTES, "link URL")?;
        let moves = r.moves()?;
        links.push(LinkRegion { rect, url, moves });
    }
    if r.at != bytes.len() {
        return Err(FrameError::TrailingBytes);
    }
    let frame = Frame {
        page_width,
        page_height,
        cut,
        list: DisplayList { items, layers },
        links,
    };
    check(&frame)?;
    Ok(frame)
}

/// Reads fields from a frame. Every read checks what is left first.
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn left(&self) -> usize {
        self.bytes.len() - self.at
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], FrameError> {
        if n > self.left() {
            return Err(FrameError::Truncated);
        }
        let slice = &self.bytes[self.at..self.at + n];
        self.at += n;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, FrameError> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, FrameError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn f32(&mut self, field: &'static str) -> Result<f32, FrameError> {
        finite(f32::from_bits(self.u32()?), field)
    }

    fn bool(&mut self, field: &'static str) -> Result<bool, FrameError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(FrameError::BadBool(field)),
        }
    }

    fn option(&mut self, field: &'static str) -> Result<Option<f32>, FrameError> {
        if self.bool(field)? {
            Ok(Some(self.f32(field)?))
        } else {
            Ok(None)
        }
    }

    fn color(&mut self) -> Result<Color, FrameError> {
        let b = self.take(4)?;
        Ok(Color {
            r: b[0],
            g: b[1],
            b: b[2],
            a: b[3],
        })
    }

    /// A list count, refused when over `cap` or when even the smallest
    /// entries could not fit in what is left.
    fn count(
        &mut self,
        cap: usize,
        min_entry: usize,
        field: &'static str,
    ) -> Result<usize, FrameError> {
        let n = self.u32()? as usize;
        if n > cap {
            return Err(FrameError::TooMany(field));
        }
        if n.saturating_mul(min_entry) > self.left() {
            return Err(FrameError::Truncated);
        }
        Ok(n)
    }

    fn text(&mut self, cap: usize, field: &'static str) -> Result<String, FrameError> {
        let len = self.u32()? as usize;
        if len > cap {
            return Err(FrameError::TooLong(field));
        }
        let raw = self.take(len)?;
        std::str::from_utf8(raw)
            .map(str::to_string)
            .map_err(|_| FrameError::BadUtf8(field))
    }

    fn f32s<const N: usize>(&mut self, field: &'static str) -> Result<[f32; N], FrameError> {
        let mut out = [0.0; N];
        for v in &mut out {
            *v = self.f32(field)?;
        }
        Ok(out)
    }

    fn rect(&mut self) -> Result<Rect, FrameError> {
        let [x, y, width, height] = self.f32s("rect")?;
        Ok(Rect {
            x,
            y,
            width,
            height,
        })
    }

    fn item(&mut self, text: &mut usize, image: &mut usize) -> Result<DisplayItem, FrameError> {
        Ok(match self.u8()? {
            TAG_PUSH_CLIP => {
                let [x, y, width, height] = self.f32s("clip")?;
                DisplayItem::PushClip {
                    x,
                    y,
                    width,
                    height,
                }
            }
            TAG_POP_CLIP => DisplayItem::PopClip,
            TAG_PUSH_FIXED => DisplayItem::PushFixed,
            TAG_POP_FIXED => DisplayItem::PopFixed,
            TAG_BOX_SHADOW => {
                let [
                    x,
                    y,
                    width,
                    height,
                    offset_x,
                    offset_y,
                    blur_radius,
                    spread_radius,
                    radius,
                ] = self.f32s("box shadow")?;
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
                    color: self.color()?,
                }
            }
            TAG_GRADIENT => {
                let [x, y, width, height, radius, border_width, angle_deg] =
                    self.f32s("gradient")?;
                DisplayItem::LinearGradientRect {
                    x,
                    y,
                    width,
                    height,
                    radius,
                    border_width,
                    angle_deg,
                    start_color: self.color()?,
                    end_color: self.color()?,
                    border_color: self.color()?,
                }
            }
            TAG_RECT => {
                let [x, y, width, height] = self.f32s("rect")?;
                DisplayItem::Rect {
                    x,
                    y,
                    width,
                    height,
                    color: self.color()?,
                }
            }
            TAG_ROUNDED_RECT => {
                let [x, y, width, height, radius, border_width] = self.f32s("rounded rect")?;
                DisplayItem::RoundedRect {
                    x,
                    y,
                    width,
                    height,
                    radius,
                    border_width,
                    fill_color: self.color()?,
                    border_color: self.color()?,
                }
            }
            TAG_IMAGE => {
                let [x, y, width, height, border_radius] = self.f32s("image")?;
                let w = self.u32()?;
                let h = self.u32()?;
                // The size and the running total are checked before the
                // pixels are copied.
                let bytes = image_bytes(w, h)?;
                *image += bytes;
                if *image > MAX_IMAGE_BYTES {
                    return Err(FrameError::TooLong("image total"));
                }
                let pixels = self.take(bytes)?.to_vec();
                DisplayItem::Image {
                    x,
                    y,
                    width,
                    height,
                    border_radius,
                    image: Image {
                        width: w,
                        height: h,
                        pixels,
                    },
                }
            }
            TAG_TEXT => {
                let [x, y] = self.f32s("text")?;
                let color = self.color()?;
                let font_size = self.f32("font_size")?;
                let run = self.text(MAX_TEXT_RUN_BYTES, "text")?;
                *text += run.len();
                if *text > MAX_TEXT_BYTES {
                    return Err(FrameError::TooLong("text total"));
                }
                DisplayItem::Text {
                    x,
                    y,
                    text: run,
                    color,
                    font_size,
                }
            }
            _ => return Err(FrameError::BadTag("item")),
        })
    }

    fn effect(&mut self) -> Result<LayerEffect, FrameError> {
        Ok(match self.u8()? {
            TAG_STICKY => {
                let rect = self.rect()?;
                let [top, right, bottom, left] = self.f32s("sticky margin")?;
                let containing = self.rect()?;
                LayerEffect::Sticky(StickyConstraint {
                    rect,
                    margin: Edge {
                        top,
                        right,
                        bottom,
                        left,
                    },
                    containing,
                    top: self.option("sticky inset")?,
                    right: self.option("sticky inset")?,
                    bottom: self.option("sticky inset")?,
                    left: self.option("sticky inset")?,
                })
            }
            TAG_TRANSFORM => {
                let [a, b, c, d, e, f] = self.f32s("transform")?;
                LayerEffect::Transform(Transform2D { a, b, c, d, e, f })
            }
            TAG_OPACITY => LayerEffect::Opacity(self.f32("opacity")?),
            _ => return Err(FrameError::BadTag("layer")),
        })
    }

    fn moves(&mut self) -> Result<Vec<LinkMove>, FrameError> {
        let count = usize::from(self.u8()?);
        if count > MAX_LINK_MOVES {
            return Err(FrameError::TooMany("link moves"));
        }
        let mut moves = Vec::with_capacity(count);
        for _ in 0..count {
            // Fixed has no fields; the others read like a layer's effect.
            if self.bytes.get(self.at) == Some(&TAG_FIXED) {
                self.at += 1;
                moves.push(LinkMove::Fixed);
                continue;
            }
            moves.push(match self.effect() {
                Ok(LayerEffect::Sticky(c)) => LinkMove::Sticky(c),
                Ok(LayerEffect::Transform(m)) => LinkMove::Transform(m),
                Ok(LayerEffect::Opacity(_)) | Err(FrameError::BadTag(_)) => {
                    return Err(FrameError::BadTag("link move"));
                }
                Err(e) => return Err(e),
            });
        }
        Ok(moves)
    }
}

// ---------------------------------------------------------------------------
// Building a frame in the renderer

/// Bytes an item takes in a frame, for the size budget.
fn encoded_len(item: &DisplayItem) -> usize {
    match item {
        DisplayItem::PushClip { .. } => 17,
        DisplayItem::PopClip | DisplayItem::PushFixed | DisplayItem::PopFixed => 1,
        DisplayItem::BoxShadow { .. } => 41,
        DisplayItem::LinearGradientRect { .. } => 41,
        DisplayItem::Rect { .. } => 21,
        DisplayItem::RoundedRect { .. } => 33,
        DisplayItem::Image { image, .. } => 29 + image.pixels.len(),
        DisplayItem::Text { text, .. } => 21 + text.len(),
    }
}

/// The top edge of what an item draws, for cutting a long page.
fn item_top(item: &DisplayItem) -> Option<f32> {
    match item {
        DisplayItem::BoxShadow { y, .. }
        | DisplayItem::LinearGradientRect { y, .. }
        | DisplayItem::Rect { y, .. }
        | DisplayItem::RoundedRect { y, .. }
        | DisplayItem::Image { y, .. }
        | DisplayItem::Text { y, .. } => Some(*y),
        _ => None,
    }
}

/// Room kept for layers and links when the items are budgeted.
const ITEM_BUDGET: usize = MAX_FRAME_BYTES - 4 * 1024 * 1024;

/// One past the lowest top edge of any drawing item in `list`, or 0 for a
/// list with none. A frame at least this tall keeps every item.
pub fn lowest_item_top(list: &DisplayList) -> f32 {
    list.items
        .iter()
        .filter_map(item_top)
        .filter(|top| top.is_finite())
        .fold(0.0f32, |low, top| low.max(top + 1.0))
}

impl Frame {
    /// The URL of the link drawn at the page point `(x, y)`, for the visible
    /// part of the page `view`. Links later in the list are drawn later, so
    /// they are tested first.
    pub fn link_at(&self, x: f32, y: f32, view: Rect) -> Option<&str> {
        self.links
            .iter()
            .rev()
            .find(|link| link.contains(x, y, view))
            .map(|link| link.url.as_str())
    }

    /// Make a frame that passes [`check`] from a painted page.
    ///
    /// Items that break a value rule are left out. When the page is taller
    /// than [`MAX_PAGE_HEIGHT`], or the items, text, images or bytes go over
    /// their caps, drawing items past that point are left out and `cut` is
    /// set. Clip and fixed pairs are kept whole, so the result balances, and
    /// layers are moved to the items that are left.
    pub fn build(
        list: &DisplayList,
        page_width: f32,
        page_height: f32,
        links: &[LinkRegion],
    ) -> Frame {
        Self::build_owned(list.clone(), page_width, page_height, links)
    }

    /// [`Frame::build`] from a list the caller gives up. Its items move
    /// into the frame instead of being copied, so a page's images are not
    /// held twice.
    pub fn build_owned(
        list: DisplayList,
        page_width: f32,
        page_height: f32,
        links: &[LinkRegion],
    ) -> Frame {
        let page_width = clamp_side(page_width, MAX_PAGE_WIDTH);
        let mut cut = !(page_height.is_finite() && page_height <= MAX_PAGE_HEIGHT);
        let page_height = clamp_side(page_height, MAX_PAGE_HEIGHT);

        // new_index[i] is how many items were kept before old item i.
        let mut new_index = Vec::with_capacity(list.items.len() + 1);
        let mut items = Vec::new();
        // For each open push: its kind and whether it was kept.
        let mut open: Vec<(Open, bool)> = Vec::new();
        let mut kept_depth = 0usize;
        let (mut text, mut image, mut bytes) = (0usize, 0usize, 0usize);
        for item in list.items {
            new_index.push(items.len());
            // Clip and fixed pushes and pops: (is a push, which kind).
            let structural = match item {
                DisplayItem::PushClip { .. } => Some((true, Open::Clip)),
                DisplayItem::PushFixed => Some((true, Open::Fixed)),
                DisplayItem::PopClip => Some((false, Open::Clip)),
                DisplayItem::PopFixed => Some((false, Open::Fixed)),
                _ => None,
            };
            if let Some((is_push, kind)) = structural {
                if is_push {
                    // Room for this push and every pop still owed.
                    let fits = kept_depth < MAX_NESTING
                        && items.len() + kept_depth + 2 <= MAX_ITEMS
                        && check_item(&item).is_ok();
                    open.push((kind, fits));
                    if fits {
                        kept_depth += 1;
                        bytes += encoded_len(&item);
                        items.push(item);
                    }
                } else if open.last().is_some_and(|(k, _)| *k == kind)
                    && let Some((_, true)) = open.pop()
                {
                    // A pop that doesn't match the open push is left out.
                    kept_depth -= 1;
                    bytes += 1;
                    items.push(item);
                }
                continue;
            }
            let Ok(cost) = check_item(&item) else {
                continue;
            };
            let below = item_top(&item).is_some_and(|top| top >= page_height);
            let len = encoded_len(&item);
            let over = items.len() + kept_depth + 1 > MAX_ITEMS
                || text + cost.text > MAX_TEXT_BYTES
                || image + cost.image > MAX_IMAGE_BYTES
                || bytes + len > ITEM_BUDGET;
            if below || over {
                cut = true;
                continue;
            }
            text += cost.text;
            image += cost.image;
            bytes += len;
            items.push(item);
        }
        new_index.push(items.len());
        // Close what the list left open.
        while let Some((kind, kept)) = open.pop() {
            if kept {
                items.push(match kind {
                    Open::Clip => DisplayItem::PopClip,
                    Open::Fixed => DisplayItem::PopFixed,
                });
            }
        }

        let mut layers = Vec::new();
        for layer in &list.layers {
            let (Some(&start), Some(&end)) = (new_index.get(layer.start), new_index.get(layer.end))
            else {
                continue;
            };
            if start < end && check_effect(&layer.effect).is_ok() && layers.len() < MAX_LAYERS {
                layers.push(Layer {
                    start,
                    end,
                    effect: layer.effect,
                });
            }
        }
        if check_layers(&layers, items.len()).is_err() {
            layers.clear();
        }

        let mut kept_links = Vec::new();
        let mut url_bytes = 0usize;
        for link in links {
            let Some(rect) = clip_to_page(&link.rect, page_width, page_height) else {
                continue;
            };
            let link = LinkRegion {
                rect,
                url: link.url.clone(),
                moves: link.moves.clone(),
            };
            // Rounding while clipping can leave an edge a hair off the page.
            if check_link(&link, page_width, page_height).is_err() {
                continue;
            }
            if kept_links.len() == MAX_LINKS || url_bytes + link.url.len() > MAX_LINK_URL_BYTES {
                cut = true;
                break;
            }
            url_bytes += link.url.len();
            kept_links.push(link);
        }

        let frame = Frame {
            page_width,
            page_height,
            cut,
            list: DisplayList { items, layers },
            links: kept_links,
        };
        if check(&frame).is_ok() {
            frame
        } else {
            // Not expected, but a frame that can't be sent is worse than an
            // empty one.
            Frame {
                page_width,
                page_height,
                cut: true,
                list: DisplayList::default(),
                links: Vec::new(),
            }
        }
    }
}

fn clamp_side(v: f32, max: f32) -> f32 {
    if v.is_finite() {
        v.clamp(1.0, max)
    } else {
        max
    }
}

/// The part of `r` on the page, or `None` when nothing is left.
fn clip_to_page(r: &Rect, page_width: f32, page_height: f32) -> Option<Rect> {
    if ![r.x, r.y, r.width, r.height].iter().all(|v| v.is_finite()) {
        return None;
    }
    let x0 = r.x.max(0.0);
    let y0 = r.y.max(0.0);
    let x1 = (r.x + r.width).min(page_width);
    let y1 = (r.y + r.height).min(page_height);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some(Rect {
        x: x0,
        y: y0,
        width: x1 - x0,
        height: y1 - y0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Color = Color {
        r: 200,
        g: 10,
        b: 20,
        a: 255,
    };

    fn rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    fn text(y: f32, s: &str) -> DisplayItem {
        DisplayItem::Text {
            x: 4.0,
            y,
            text: s.to_string(),
            color: RED,
            font_size: 16.0,
        }
    }

    fn fill(y: f32) -> DisplayItem {
        DisplayItem::Rect {
            x: 0.0,
            y,
            width: 100.0,
            height: 20.0,
            color: RED,
        }
    }

    fn clip() -> DisplayItem {
        DisplayItem::PushClip {
            x: 0.0,
            y: 0.0,
            width: 300.0,
            height: 300.0,
        }
    }

    /// A frame with every item kind, every layer kind and a link.
    fn sample() -> Frame {
        let items = vec![
            DisplayItem::BoxShadow {
                x: 1.0,
                y: 2.0,
                width: 30.0,
                height: 40.0,
                offset_x: -3.0,
                offset_y: 4.0,
                blur_radius: 8.0,
                spread_radius: -2.0,
                radius: 6.0,
                color: RED,
            },
            DisplayItem::LinearGradientRect {
                x: 0.0,
                y: 0.0,
                width: 300.0,
                height: 50.0,
                radius: 4.0,
                border_width: 1.0,
                angle_deg: 90.0,
                start_color: RED,
                end_color: Color::BLACK,
                border_color: RED,
            },
            clip(),
            DisplayItem::RoundedRect {
                x: 10.0,
                y: 60.0,
                width: 80.0,
                height: 30.0,
                radius: 9999.0,
                border_width: 2.0,
                fill_color: RED,
                border_color: Color::BLACK,
            },
            DisplayItem::Image {
                x: 10.0,
                y: 100.0,
                width: 4.0,
                height: 2.0,
                border_radius: 0.0,
                image: Image {
                    width: 2,
                    height: 1,
                    pixels: vec![1, 2, 3, 4, 5, 6, 7, 8],
                },
            },
            DisplayItem::PopClip,
            DisplayItem::PushFixed,
            fill(0.0),
            text(2.0, "Héllo, 漢字"),
            DisplayItem::PopFixed,
            text(-0.0, ""),
        ];
        let layers = vec![
            Layer {
                start: 2,
                end: 6,
                effect: LayerEffect::Sticky(StickyConstraint {
                    rect: rect(0.0, 0.0, 300.0, 300.0),
                    margin: Edge {
                        top: 1.0,
                        right: 0.0,
                        bottom: -1.0,
                        left: 0.0,
                    },
                    containing: rect(0.0, 0.0, 800.0, 2000.0),
                    top: Some(0.0),
                    right: None,
                    bottom: Some(10.0),
                    left: None,
                }),
            },
            Layer {
                start: 3,
                end: 5,
                effect: LayerEffect::Transform(Transform2D {
                    a: 0.5,
                    b: 0.1,
                    c: -0.1,
                    d: 2.0,
                    e: 30.0,
                    f: -40.0,
                }),
            },
            Layer {
                start: 4,
                end: 5,
                effect: LayerEffect::Opacity(0.25),
            },
            Layer {
                start: 7,
                end: 9,
                effect: LayerEffect::Opacity(1.0),
            },
        ];
        Frame {
            page_width: 800.0,
            page_height: 2000.0,
            cut: false,
            list: DisplayList { items, layers },
            links: vec![
                LinkRegion {
                    rect: rect(10.0, 60.0, 80.0, 30.0),
                    url: "https://example.test/a?b=c".to_string(),
                    moves: Vec::new(),
                },
                LinkRegion {
                    rect: rect(10.0, 0.0, 80.0, 30.0),
                    url: "https://example.test/top".to_string(),
                    moves: vec![
                        LinkMove::Fixed,
                        LinkMove::Sticky(StickyConstraint {
                            rect: rect(0.0, 0.0, 800.0, 40.0),
                            margin: Edge {
                                top: 0.0,
                                right: 0.0,
                                bottom: 0.0,
                                left: 0.0,
                            },
                            containing: rect(0.0, 0.0, 800.0, 2000.0),
                            top: Some(0.0),
                            right: None,
                            bottom: None,
                            left: None,
                        }),
                        LinkMove::Transform(Transform2D {
                            a: 2.0,
                            b: 0.0,
                            c: 0.0,
                            d: 2.0,
                            e: -10.0,
                            f: 0.0,
                        }),
                    ],
                },
            ],
        }
    }

    fn with_items(items: Vec<DisplayItem>) -> Frame {
        Frame {
            list: DisplayList {
                items,
                layers: Vec::new(),
            },
            links: Vec::new(),
            ..sample()
        }
    }

    #[test]
    fn a_frame_travels_inside_a_wire_message() {
        use anvil_sandbox::wire::{self, Direction, Message, RenderedFrame};
        let bytes = encode(&sample()).unwrap();
        let message = Message::Frame(RenderedFrame {
            tab_id: 1,
            nav_id: 2,
            title: "t".to_string(),
            dom_nodes: 3,
            blocked_resources: 0,
            display_list: bytes,
        });
        let on_wire = wire::encode(5, &message).unwrap();
        let Ok((5, Message::Frame(got))) = wire::decode(&on_wire, Direction::ToBrowser) else {
            panic!("frame message did not decode");
        };
        assert_eq!(decode(&got.display_list).unwrap(), sample());
    }

    #[test]
    fn a_frame_round_trips_exactly() {
        let frame = sample();
        let bytes = encode(&frame).unwrap();
        let back = decode(&bytes).unwrap();
        assert_eq!(back, frame);
        assert_eq!(encode(&back).unwrap(), bytes);
        // -0.0 keeps its sign bit.
        let DisplayItem::Text { y, .. } = back.list.items[10] else {
            panic!("text expected");
        };
        assert!(y.is_sign_negative());
    }

    #[test]
    fn strict_decoding_refuses_any_change_of_form() {
        let bytes = encode(&sample()).unwrap();
        for len in 0..bytes.len() {
            assert!(decode(&bytes[..len]).is_err(), "prefix {len} accepted");
        }
        let mut spare = bytes.clone();
        spare.push(0);
        assert_eq!(decode(&spare), Err(FrameError::TrailingBytes));
        let mut cut = bytes.clone();
        cut[8] = 2;
        assert_eq!(decode(&cut), Err(FrameError::BadBool("cut")));
        let mut tag = bytes.clone();
        tag[13] = 0;
        assert_eq!(decode(&tag), Err(FrameError::BadTag("item")));
        tag[13] = 11;
        assert_eq!(decode(&tag), Err(FrameError::BadTag("item")));
        let mut nan = bytes.clone();
        nan[0..4].copy_from_slice(&f32::NAN.to_bits().to_le_bytes());
        assert_eq!(decode(&nan), Err(FrameError::NotFinite("page_width")));
        nan[0..4].copy_from_slice(&f32::INFINITY.to_bits().to_le_bytes());
        assert_eq!(decode(&nan), Err(FrameError::NotFinite("page_width")));
    }

    #[test]
    fn caps_are_checked_before_anything_is_allocated() {
        let header = |count: u32| {
            let mut b = Vec::new();
            put_f32(&mut b, 800.0);
            put_f32(&mut b, 600.0);
            b.push(0);
            put_u32(&mut b, count);
            b
        };
        assert_eq!(decode(&header(u32::MAX)), Err(FrameError::TooMany("items")));
        // Under the cap, but more items than bytes left.
        assert_eq!(decode(&header(1000)), Err(FrameError::Truncated));

        // A text run that claims 4 GiB.
        let mut b = header(1);
        b.push(TAG_TEXT);
        put_f32s(&mut b, &[0.0, 0.0]);
        put_color(&mut b, RED);
        put_f32(&mut b, 16.0);
        put_u32(&mut b, u32::MAX);
        assert_eq!(decode(&b), Err(FrameError::TooLong("text")));

        // Images: too big a side, too many bytes, and a fair size that claims
        // more pixels than were sent.
        let image = |w: u32, h: u32| {
            let mut b = header(1);
            b.push(TAG_IMAGE);
            put_f32s(&mut b, &[0.0; 5]);
            put_u32(&mut b, w);
            put_u32(&mut b, h);
            b
        };
        assert_eq!(
            decode(&image(MAX_IMAGE_SIDE + 1, 1)),
            Err(FrameError::OutOfRange("image size"))
        );
        assert_eq!(
            decode(&image(0, 1)),
            Err(FrameError::OutOfRange("image size"))
        );
        assert_eq!(
            decode(&image(MAX_IMAGE_SIDE, MAX_IMAGE_SIDE)),
            Err(FrameError::TooLong("image"))
        );
        assert_eq!(decode(&image(1000, 1000)), Err(FrameError::Truncated));

        let mut huge = vec![0u8; MAX_FRAME_BYTES + 1];
        huge[0..4].copy_from_slice(&800f32.to_bits().to_le_bytes());
        assert_eq!(decode(&huge), Err(FrameError::TooLarge));
    }

    #[test]
    fn value_rules_refuse_out_of_range_numbers() {
        let bad_items = [
            DisplayItem::Rect {
                x: MAX_COORD * 2.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
                color: RED,
            },
            DisplayItem::Rect {
                x: 0.0,
                y: 0.0,
                width: -1.0,
                height: 1.0,
                color: RED,
            },
            DisplayItem::Text {
                x: 0.0,
                y: 0.0,
                text: "a".to_string(),
                color: RED,
                font_size: 0.0,
            },
            DisplayItem::Text {
                x: 0.0,
                y: 0.0,
                text: "a".to_string(),
                color: RED,
                font_size: MAX_FONT_SIZE + 1.0,
            },
            DisplayItem::Text {
                x: 0.0,
                y: 0.0,
                text: "a".repeat(MAX_TEXT_RUN_BYTES + 1),
                color: RED,
                font_size: 12.0,
            },
            DisplayItem::BoxShadow {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
                offset_x: 0.0,
                offset_y: 0.0,
                blur_radius: MAX_BLUR + 1.0,
                spread_radius: 0.0,
                radius: 0.0,
                color: RED,
            },
            DisplayItem::LinearGradientRect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
                radius: 0.0,
                border_width: 0.0,
                angle_deg: f32::NAN,
                start_color: RED,
                end_color: RED,
                border_color: RED,
            },
            DisplayItem::Image {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
                border_radius: 0.0,
                image: Image {
                    width: 2,
                    height: 2,
                    pixels: vec![0; 15],
                },
            },
        ];
        for item in bad_items {
            let frame = with_items(vec![item.clone()]);
            assert!(check(&frame).is_err(), "{item:?} passed");
            assert!(encode(&frame).is_err(), "{item:?} encoded");
        }
        for (page_width, page_height) in [
            (0.0, 600.0),
            (MAX_PAGE_WIDTH + 1.0, 600.0),
            (800.0, MAX_PAGE_HEIGHT + 1.0),
        ] {
            let frame = Frame {
                page_width,
                page_height,
                ..with_items(Vec::new())
            };
            assert!(check(&frame).is_err());
        }
        for effect in [
            LayerEffect::Opacity(1.5),
            LayerEffect::Opacity(-0.1),
            LayerEffect::Transform(Transform2D {
                a: MAX_TRANSFORM_SCALE * 2.0,
                ..Transform2D::IDENTITY
            }),
            LayerEffect::Transform(Transform2D {
                e: f32::INFINITY,
                ..Transform2D::IDENTITY
            }),
        ] {
            let mut frame = with_items(vec![fill(0.0)]);
            frame.list.layers.push(Layer {
                start: 0,
                end: 1,
                effect,
            });
            assert!(check(&frame).is_err(), "{effect:?} passed");
        }
    }

    #[test]
    fn totals_are_capped() {
        let run = "a".repeat(MAX_TEXT_RUN_BYTES);
        let runs = MAX_TEXT_BYTES / MAX_TEXT_RUN_BYTES;
        let mut items: Vec<_> = (0..runs).map(|_| text(0.0, &run)).collect();
        assert!(check(&with_items(items.clone())).is_ok());
        items.push(text(0.0, "a"));
        assert_eq!(
            check(&with_items(items)),
            Err(FrameError::TooLong("text total"))
        );

        let image = |side: u32| DisplayItem::Image {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
            border_radius: 0.0,
            image: Image::new(side, side),
        };
        assert!(check(&with_items(vec![image(2048)])).is_ok());
        assert_eq!(
            check(&with_items(vec![image(2048), image(1)])),
            Err(FrameError::TooLong("image total"))
        );
    }

    #[test]
    fn clips_and_fixed_ranges_must_balance() {
        let bad = [
            vec![DisplayItem::PopClip],
            vec![clip()],
            vec![DisplayItem::PushFixed, DisplayItem::PopClip],
            vec![
                clip(),
                DisplayItem::PushFixed,
                DisplayItem::PopClip,
                DisplayItem::PopFixed,
            ],
        ];
        for items in bad {
            assert_eq!(check(&with_items(items)), Err(FrameError::Unbalanced));
        }
        let deep = |n: usize| {
            let mut items = vec![clip(); n];
            items.extend(vec![DisplayItem::PopClip; n]);
            with_items(items)
        };
        assert!(check(&deep(MAX_NESTING)).is_ok());
        assert_eq!(check(&deep(MAX_NESTING + 1)), Err(FrameError::Unbalanced));
    }

    #[test]
    fn layers_must_be_sorted_nested_and_inside_the_items() {
        let layer = |start, end| Layer {
            start,
            end,
            effect: LayerEffect::Opacity(0.5),
        };
        let frame = |layers: Vec<Layer>| Frame {
            list: DisplayList {
                items: vec![fill(0.0); 6],
                layers,
            },
            ..with_items(Vec::new())
        };
        for good in [
            vec![layer(0, 6), layer(1, 3), layer(3, 6)],
            vec![layer(0, 2), layer(2, 4)],
            vec![layer(1, 4), layer(1, 2)],
        ] {
            assert!(check(&frame(good.clone())).is_ok(), "{good:?}");
        }
        for bad in [
            vec![layer(2, 4), layer(0, 1)],
            vec![layer(0, 3), layer(2, 5)],
            vec![layer(1, 2), layer(1, 4)],
            vec![layer(4, 2)],
            vec![layer(0, 7)],
        ] {
            assert_eq!(
                check(&frame(bad.clone())),
                Err(FrameError::BadLayer),
                "{bad:?}"
            );
        }
        let nested = (0..=MAX_NESTING).map(|i| layer(0, 6 - i.min(5))).collect();
        assert_eq!(check(&frame(nested)), Err(FrameError::BadLayer));
    }

    #[test]
    fn link_moves_are_checked_and_decoded_strictly() {
        let with_moves = |moves: Vec<LinkMove>| Frame {
            links: vec![LinkRegion {
                rect: rect(0.0, 0.0, 10.0, 10.0),
                url: "https://a.test/".to_string(),
                moves,
            }],
            ..with_items(Vec::new())
        };
        let fixed = with_moves(vec![LinkMove::Fixed]);
        let bytes = encode(&fixed).unwrap();
        assert_eq!(decode(&bytes).unwrap(), fixed);
        assert_eq!(bytes[bytes.len() - 2..], [1, TAG_FIXED]);
        let mut bad = bytes.clone();
        *bad.last_mut().unwrap() = 9;
        assert_eq!(decode(&bad), Err(FrameError::BadTag("link move")));
        // Opacity is a layer effect, not a way to move a link.
        let mut bad = bytes.clone();
        *bad.last_mut().unwrap() = TAG_OPACITY;
        bad.extend_from_slice(&0.5f32.to_le_bytes());
        assert_eq!(decode(&bad), Err(FrameError::BadTag("link move")));
        let mut bad = bytes.clone();
        let at = bad.len() - 2;
        bad[at] = (MAX_LINK_MOVES + 1) as u8;
        assert_eq!(decode(&bad), Err(FrameError::TooMany("link moves")));
        assert_eq!(
            check(&with_moves(vec![LinkMove::Fixed; MAX_LINK_MOVES + 1])),
            Err(FrameError::TooMany("link moves"))
        );
        let stretched = Transform2D {
            a: MAX_TRANSFORM_SCALE * 2.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        };
        assert_eq!(
            check(&with_moves(vec![LinkMove::Transform(stretched)])),
            Err(FrameError::OutOfRange("transform"))
        );
    }

    #[test]
    fn a_link_is_hit_where_its_boxes_draw_it() {
        let view = |scroll: f32| rect(0.0, scroll, 800.0, 600.0);
        let link = |moves: Vec<LinkMove>| LinkRegion {
            rect: rect(10.0, 10.0, 100.0, 20.0),
            url: "https://a.test/".to_string(),
            moves,
        };
        let plain = link(Vec::new());
        assert!(plain.contains(20.0, 15.0, view(0.0)));
        assert!(!plain.contains(20.0, 515.0, view(500.0)));
        // A fixed link stays at the same place in the window.
        let fixed = link(vec![LinkMove::Fixed]);
        assert!(fixed.contains(20.0, 515.0, view(500.0)));
        assert!(!fixed.contains(20.0, 15.0, view(500.0)));
        // A sticky header with top: 0 follows the scroll.
        let sticky = link(vec![LinkMove::Sticky(StickyConstraint {
            rect: rect(0.0, 10.0, 800.0, 20.0),
            margin: Edge {
                top: 0.0,
                right: 0.0,
                bottom: 0.0,
                left: 0.0,
            },
            containing: rect(0.0, 0.0, 800.0, 5000.0),
            top: Some(0.0),
            right: None,
            bottom: None,
            left: None,
        })]);
        assert!(sticky.contains(20.0, 15.0, view(0.0)));
        assert!(sticky.contains(20.0, 505.0, view(500.0)));
        assert!(!sticky.contains(20.0, 15.0, view(500.0)));
        // Moved 200 to the right by a transform.
        let moved = Transform2D {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 200.0,
            f: 0.0,
        };
        let shifted = link(vec![LinkMove::Transform(moved)]);
        assert!(shifted.contains(220.0, 15.0, view(0.0)));
        assert!(!shifted.contains(20.0, 15.0, view(0.0)));
        // A flattening transform draws nothing, so nothing is hit.
        let flat = link(vec![LinkMove::Transform(Transform2D {
            a: 0.0,
            d: 0.0,
            ..moved
        })]);
        assert!(!flat.contains(210.0, 10.0, view(0.0)));
    }

    #[test]
    fn links_stay_on_the_page_with_a_web_url() {
        let frame = |r: Rect, url: &str| Frame {
            links: vec![LinkRegion {
                rect: r,
                url: url.to_string(),
                moves: Vec::new(),
            }],
            ..with_items(Vec::new())
        };
        assert!(check(&frame(rect(0.0, 0.0, 800.0, 2000.0), "http://a.test/")).is_ok());
        for url in [
            "file:///etc/passwd",
            "anvil://settings",
            "javascript:alert(1)",
            "https://a.test/ b",
            "https://a.test/\u{e9}",
            "",
        ] {
            assert_eq!(
                check(&frame(rect(0.0, 0.0, 1.0, 1.0), url)),
                Err(FrameError::BadLink)
            );
        }
        let long = format!("https://a.test/{}", "a".repeat(MAX_URL_BYTES));
        assert_eq!(
            check(&frame(rect(0.0, 0.0, 1.0, 1.0), &long)),
            Err(FrameError::BadLink)
        );
        for r in [
            rect(-1.0, 0.0, 1.0, 1.0),
            rect(0.0, 0.0, 801.0, 1.0),
            rect(0.0, 1999.5, 1.0, 1.0),
            rect(0.0, 0.0, f32::NAN, 1.0),
        ] {
            assert_eq!(
                check(&frame(r, "https://a.test/")),
                Err(FrameError::BadLink)
            );
        }
    }

    #[test]
    fn build_cuts_a_long_page_and_keeps_layers_on_their_items() {
        let mut items = vec![fill(0.0), clip()];
        items.push(fill(MAX_PAGE_HEIGHT + 10.0));
        items.push(text(20.0, "kept"));
        items.push(DisplayItem::PopClip);
        items.push(fill(40.0));
        let list = DisplayList {
            items,
            layers: vec![
                Layer {
                    start: 1,
                    end: 5,
                    effect: LayerEffect::Opacity(0.5),
                },
                Layer {
                    start: 2,
                    end: 3,
                    effect: LayerEffect::Opacity(0.5),
                },
                Layer {
                    start: 5,
                    end: 6,
                    effect: LayerEffect::Opacity(0.5),
                },
            ],
        };
        let frame = Frame::build(&list, 800.0, MAX_PAGE_HEIGHT * 3.0, &[]);
        assert!(frame.cut);
        assert_eq!(frame.page_height, MAX_PAGE_HEIGHT);
        assert_eq!(
            frame.list.items,
            vec![
                fill(0.0),
                clip(),
                text(20.0, "kept"),
                DisplayItem::PopClip,
                fill(40.0)
            ]
        );
        // The second layer only held the dropped item, so it is gone.
        let ranges: Vec<_> = frame.list.layers.iter().map(|l| (l.start, l.end)).collect();
        assert_eq!(ranges, vec![(1, 4), (4, 5)]);
        assert_eq!(decode(&encode(&frame).unwrap()).unwrap(), frame);
    }

    #[test]
    fn build_drops_bad_items_and_balances_what_is_left() {
        let list = DisplayList {
            items: vec![
                DisplayItem::PopClip,
                text(0.0, "a"),
                DisplayItem::Text {
                    x: f32::NAN,
                    y: 0.0,
                    text: "b".to_string(),
                    color: RED,
                    font_size: 12.0,
                },
                DisplayItem::PushFixed,
                clip(),
                fill(5.0),
            ],
            layers: vec![Layer {
                start: 1,
                end: 3,
                effect: LayerEffect::Opacity(7.0),
            }],
        };
        let frame = Frame::build(&list, 800.0, 600.0, &[]);
        assert!(!frame.cut);
        assert_eq!(
            frame.list.items,
            vec![
                text(0.0, "a"),
                DisplayItem::PushFixed,
                clip(),
                fill(5.0),
                DisplayItem::PopClip,
                DisplayItem::PopFixed
            ]
        );
        assert!(frame.list.layers.is_empty());

        let mut deep = vec![clip(); MAX_NESTING + 3];
        deep.push(fill(1.0));
        deep.extend(vec![DisplayItem::PopClip; MAX_NESTING + 3]);
        let frame = Frame::build(
            &DisplayList {
                items: deep,
                layers: Vec::new(),
            },
            800.0,
            600.0,
            &[],
        );
        assert_eq!(frame.list.items.len(), 2 * MAX_NESTING + 1);
        assert!(check(&frame).is_ok());
    }

    #[test]
    fn build_cuts_at_the_image_and_link_caps() {
        let big = DisplayItem::Image {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
            border_radius: 0.0,
            image: Image::new(2048, 2048),
        };
        let list = DisplayList {
            items: vec![big.clone(), big, fill(1.0)],
            layers: Vec::new(),
        };
        let links: Vec<_> = (0..MAX_LINKS + 5)
            .map(|i| LinkRegion {
                rect: rect(-5.0, i as f32, 1000.0, 1.0),
                url: format!("https://a.test/{i}"),
                moves: Vec::new(),
            })
            .chain([LinkRegion {
                rect: rect(0.0, 0.0, 1.0, 1.0),
                url: "anvil://settings".to_string(),
                moves: Vec::new(),
            }])
            .collect();
        let frame = Frame::build(&list, 800.0, 20_000.0, &links);
        assert!(frame.cut);
        assert_eq!(frame.list.items.len(), 2);
        assert_eq!(frame.links.len(), MAX_LINKS);
        assert_eq!(frame.links[0].rect, rect(0.0, 0.0, 800.0, 1.0));
        assert!(check(&frame).is_ok());
    }
}
