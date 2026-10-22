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
    for run in &tree.text {
        items.push(DisplayItem::Text {
            x: run.x,
            y: run.y,
            text: run.text.clone(),
            color: run.color,
            font_size: run.font_size,
        });
    }
    for child in &tree.children {
        walk(child, items);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flux_layout::{NodeId, Rect, TextRun};

    #[test]
    fn paints_background_before_text() {
        let tree = LayoutBox {
            node: NodeId::from_raw(1),
            rect: Rect {
                x: 1.0,
                y: 2.0,
                width: 30.0,
                height: 10.0,
            },
            background: Some(Color::BLACK),
            text: vec![TextRun {
                node: NodeId::from_raw(2),
                text: "Hi".to_string(),
                x: 1.0,
                y: 2.0,
                font_size: 10.0,
                color: Color::BLACK,
            }],
            children: Vec::new(),
        };
        let list = paint(&tree);
        assert!(matches!(list.items[0], DisplayItem::Rect { .. }));
        assert!(matches!(&list.items[1], DisplayItem::Text { text, .. } if text == "Hi"));
    }
}
