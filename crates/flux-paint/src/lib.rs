//! Display list built from a laid-out box tree.

use flux_layout::{Color, Image, LayoutBox, Overflow, Position};

#[derive(Clone, Debug, PartialEq)]
pub enum DisplayItem {
    PushClip {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    },
    PopClip,
    PushFixed,
    PopFixed,
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
    Rect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        color: Color,
    },
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
    Image {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        border_radius: f32,
        image: Image,
    },
    Text {
        x: f32,
        y: f32,
        text: String,
        color: Color,
        font_size: f32,
    },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DisplayList {
    pub items: Vec<DisplayItem>,
}

pub fn paint(tree: &LayoutBox) -> DisplayList {
    let mut items = Vec::new();
    walk(tree, &mut items);
    DisplayList { items }
}

fn walk(tree: &LayoutBox, items: &mut Vec<DisplayItem>) {
    let is_fixed = tree.position == Position::Fixed;
    if is_fixed {
        items.push(DisplayItem::PushFixed);
    }
    if let Some(shadow) = tree.box_shadow {
        if tree.rect.width > 0.0 && tree.rect.height > 0.0 && shadow.color.a > 0 {
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
    }
    if let Some(grad) = tree.background_gradient {
        if tree.rect.width > 0.0 && tree.rect.height > 0.0 {
            let border_width = tree
                .border
                .top
                .max(tree.border.right)
                .max(tree.border.bottom)
                .max(tree.border.left);
            items.push(DisplayItem::LinearGradientRect {
                x: tree.rect.x,
                y: tree.rect.y,
                width: tree.rect.width,
                height: tree.rect.height,
                radius: tree.border_radius,
                border_width,
                angle_deg: grad.angle_deg,
                start_color: grad.start_color,
                end_color: grad.end_color,
                border_color: tree.border_color,
            });
        }
    } else if tree.border_radius > 0.0 && tree.rect.width > 0.0 && tree.rect.height > 0.0 {
        let fill_color = tree.background.unwrap_or(Color {
            r: 0,
            g: 0,
            b: 0,
            a: 0,
        });
        let border_width = tree
            .border
            .top
            .max(tree.border.right)
            .max(tree.border.bottom)
            .max(tree.border.left);
        if fill_color.a > 0 || border_width > 0.0 {
            items.push(DisplayItem::RoundedRect {
                x: tree.rect.x,
                y: tree.rect.y,
                width: tree.rect.width,
                height: tree.rect.height,
                radius: tree.border_radius,
                border_width,
                fill_color,
                border_color: tree.border_color,
            });
        }
    } else {
        if let Some(color) = tree.background {
            if tree.rect.width > 0.0 && tree.rect.height > 0.0 {
                items.push(DisplayItem::Rect {
                    x: tree.rect.x,
                    y: tree.rect.y,
                    width: tree.rect.width,
                    height: tree.rect.height,
                    color,
                });
            }
        }
        paint_borders(tree, items);
    }

    let is_clipped =
        tree.overflow == Overflow::Hidden && tree.rect.width > 0.0 && tree.rect.height > 0.0;
    if is_clipped {
        items.push(DisplayItem::PushClip {
            x: tree.rect.x + tree.border.left,
            y: tree.rect.y + tree.border.top,
            width: (tree.rect.width - tree.border.left - tree.border.right).max(0.0),
            height: (tree.rect.height - tree.border.top - tree.border.bottom).max(0.0),
        });
    }

    if let Some(image) = &tree.image {
        let ix = tree.rect.x + tree.border.left;
        let iy = tree.rect.y + tree.border.top;
        let iw = (tree.rect.width - tree.border.left - tree.border.right).max(0.0);
        let ih = (tree.rect.height - tree.border.top - tree.border.bottom).max(0.0);
        if iw > 0.0 && ih > 0.0 {
            items.push(DisplayItem::Image {
                x: ix,
                y: iy,
                width: iw,
                height: ih,
                border_radius: (tree.border_radius - tree.border.top).max(0.0),
                image: image.clone(),
            });
        }
    }
    for run in &tree.text {
        items.push(DisplayItem::Text {
            x: run.x,
            y: run.y,
            text: run.text.clone(),
            color: run.color,
            font_size: run.font_size,
        });
        if run.underline && run.width > 0.0 {
            items.push(DisplayItem::Rect {
                x: run.x,
                y: run.y + run.font_size * 1.05,
                width: run.width,
                height: 1.0,
                color: run.color,
            });
        }
    }
    let mut order: Vec<(usize, &LayoutBox)> = tree.children.iter().enumerate().collect();
    order.sort_by_key(|(idx, child)| (child.z_index, *idx));
    for (_, child) in order {
        walk(child, items);
    }
    if is_clipped {
        items.push(DisplayItem::PopClip);
    }
    if is_fixed {
        items.push(DisplayItem::PopFixed);
    }
}

fn paint_borders(tree: &LayoutBox, items: &mut Vec<DisplayItem>) {
    let b = tree.border;
    let r = tree.rect;
    let color = tree.border_color;
    if r.width <= 0.0 || r.height <= 0.0 {
        return;
    }
    if b.top > 0.0 {
        items.push(DisplayItem::Rect {
            x: r.x,
            y: r.y,
            width: r.width,
            height: b.top.min(r.height),
            color,
        });
    }
    if b.bottom > 0.0 {
        let h = b.bottom.min(r.height);
        items.push(DisplayItem::Rect {
            x: r.x,
            y: r.y + r.height - h,
            width: r.width,
            height: h,
            color,
        });
    }
    if b.left > 0.0 {
        items.push(DisplayItem::Rect {
            x: r.x,
            y: r.y,
            width: b.left.min(r.width),
            height: r.height,
            color,
        });
    }
    if b.right > 0.0 {
        let w = b.right.min(r.width);
        items.push(DisplayItem::Rect {
            x: r.x + r.width - w,
            y: r.y,
            width: w,
            height: r.height,
            color,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flux_layout::{Edge, NodeId, Rect, TextRun};

    #[test]
    fn paints_background_borders_text_and_underline() {
        let tree = LayoutBox {
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
            background: Some(Color::BLACK),
            background_gradient: None,
            box_shadow: None,
            border: Edge::all(1.0),
            border_color: Color::BLACK,
            border_radius: 0.0,
            image: None,
            text: vec![TextRun {
                node: NodeId::from_raw(2),
                text: "Hi".to_string(),
                x: 1.0,
                y: 2.0,
                width: 10.0,
                font_size: 10.0,
                color: Color::BLACK,
                underline: true,
            }],
            children: Vec::new(),
        };
        let list = paint(&tree);
        assert!(matches!(list.items[0], DisplayItem::Rect { .. }));
        assert!(
            list.items
                .iter()
                .any(|item| matches!(item, DisplayItem::Text { text, .. } if text == "Hi"))
        );
        assert!(list.items.len() >= 7);
    }
}
