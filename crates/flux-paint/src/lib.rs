//! Display list built from a laid-out box tree.

use flux_layout::{Color, LayoutBox};

#[derive(Clone, Debug, PartialEq)]
pub enum DisplayItem {
    Rect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        color: Color,
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
    for child in &tree.children {
        walk(child, items);
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
            background: Some(Color::BLACK),
            border: Edge::all(1.0),
            border_color: Color::BLACK,
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
