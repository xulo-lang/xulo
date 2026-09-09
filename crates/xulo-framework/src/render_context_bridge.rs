use xulo_render_core::RenderContext;
use xulo_ui::{Color, FontWeight, Widget};

/// Render a widget tree into a [`RenderContext`].
pub fn render_with_context(ctx: &mut dyn RenderContext, root: &Widget, width: f64, height: f64) {
    ctx.set_fill_color("#202020");
    ctx.begin_path();
    ctx.rect(0.0, 0.0, width, height);
    ctx.fill();
    render_widget(ctx, root, 0.0, 0.0, width, height);
}

fn render_widget(
    ctx: &mut dyn RenderContext,
    widget: &Widget,
    x: f64,
    y: f64,
    available_w: f64,
    available_h: f64,
) {
    match widget {
        Widget::Screen { background, children, .. } => {
            if let Some(bg) = background {
                ctx.set_fill_color(&color_to_hex(*bg));
                ctx.begin_path();
                ctx.rect(x, y, available_w, available_h);
                ctx.fill();
            }
            render_children(ctx, children, x, y, available_w, available_h);
        }
        Widget::VStack { spacing, children, style } => {
            let pad = style.padding.unwrap_or(0) as f64;
            let inner_x = x + pad;
            let inner_y = y + pad;
            let inner_w = available_w - pad * 2.0;
            let mut child_y = inner_y;
            for child in children {
                let child_h = estimate_height(child, inner_w);
                render_widget(ctx, child, inner_x, child_y, inner_w, child_h);
                child_y += child_h + *spacing as f64;
            }
        }
        Widget::HStack { spacing, children, style } => {
            let pad = style.padding.unwrap_or(0) as f64;
            let inner_x = x + pad;
            let inner_y = y + pad;
            let inner_h = available_h - pad * 2.0;
            let mut child_x = inner_x;
            for child in children {
                let child_w = estimate_width(child, inner_h);
                render_widget(ctx, child, child_x, inner_y, child_w, inner_h);
                child_x += child_w + *spacing as f64;
            }
        }
        Widget::Fill { children, style } => {
            let pad = style.padding.unwrap_or(0) as f64;
            let inner_x = x + pad;
            let inner_y = y + pad;
            let inner_w = available_w - pad * 2.0;
            let inner_h = available_h - pad * 2.0;
            render_children(ctx, children, inner_x, inner_y, inner_w, inner_h);
        }
        Widget::Text { text, color, style } => {
            let font_size = style.font_size.unwrap_or(12);
            let bold = style.font_weight == Some(FontWeight::Bold);
            let font = if bold { format!("bold {font_size}px monospace") } else { format!("{font_size}px monospace") };
            ctx.set_font(&font);
            let text_color = color.unwrap_or(Color::WHITE);
            ctx.set_fill_color(&color_to_hex(text_color));
            ctx.fill_text(text, x, y);
        }
        Widget::Button { label, style } => {
            let bg = style.background_color.unwrap_or(Color::new(74, 144, 217));
            ctx.set_fill_color(&color_to_hex(bg));
            ctx.begin_path();
            ctx.round_rect(x, y, available_w, available_h, 4.0);
            ctx.fill();
            if let Some(border) = style.border_color {
                ctx.set_stroke_color(&color_to_hex(border));
                ctx.set_stroke_width(1.0);
                ctx.begin_path();
                ctx.round_rect(x, y, available_w, available_h, 4.0);
                ctx.stroke();
            }
            let fg = style.color.unwrap_or(Color::WHITE);
            ctx.set_fill_color(&color_to_hex(fg));
            ctx.set_font("12px monospace");
            ctx.fill_text(label, x + 8.0, y + 4.0);
        }
        Widget::Input { value, placeholder, style, .. } => {
            let bg = style.background_color.unwrap_or(Color::new(255, 255, 255));
            ctx.set_fill_color(&color_to_hex(bg));
            ctx.begin_path();
            ctx.rect(x, y, available_w, available_h);
            ctx.fill();
            let border = style.border_color.unwrap_or(Color::GRAY);
            ctx.set_stroke_color(&color_to_hex(border));
            ctx.set_stroke_width(1.0);
            ctx.begin_path();
            ctx.rect(x, y, available_w, available_h);
            ctx.stroke();
            let display = if value.is_empty() { placeholder } else { value };
            let text_color = if value.is_empty() { Color::GRAY } else { style.color.unwrap_or(Color::BLACK) };
            ctx.set_fill_color(&color_to_hex(text_color));
            ctx.set_font("12px monospace");
            ctx.fill_text(display, x + 4.0, y + 4.0);
        }
        Widget::ScrollView { children, style, .. } => {
            let pad = style.padding.unwrap_or(0) as f64;
            render_children(ctx, children, x + pad, y + pad, available_w - pad * 2.0, available_h - pad * 2.0);
        }
        Widget::Unknown { kind, .. } => {
            ctx.set_fill_color("#404040");
            ctx.begin_path();
            ctx.rect(x, y, available_w, available_h);
            ctx.fill();
            ctx.set_fill_color("#808080");
            ctx.set_font("10px monospace");
            ctx.fill_text(kind, x + 4.0, y + 4.0);
        }
    }
}

fn render_children(ctx: &mut dyn RenderContext, children: &[Widget], x: f64, y: f64, w: f64, h: f64) {
    for child in children {
        render_widget(ctx, child, x, y, w, h);
    }
}

pub fn estimate_height(widget: &Widget, _available_w: f64) -> f64 {
    match widget {
        Widget::Text { style, .. } => style.font_size.unwrap_or(12) as f64 + 4.0,
        Widget::Button { .. } => 28.0,
        Widget::Input { .. } => 24.0,
        Widget::VStack { children, spacing, .. } => {
            children.iter().map(|c| estimate_height(c, _available_w)).sum::<f64>()
                + (*spacing as f64 * children.len().saturating_sub(1) as f64)
        }
        Widget::HStack { children, .. } => {
            children.iter().map(|c| estimate_height(c, _available_w)).fold(0.0, f64::max)
        }
        Widget::Fill { children, .. } => {
            children.iter().map(|c| estimate_height(c, _available_w)).sum::<f64>()
        }
        _ => 20.0,
    }
}

pub fn estimate_width(widget: &Widget, _available_h: f64) -> f64 {
    match widget {
        Widget::Text { text, style, .. } => {
            let font_size = style.font_size.unwrap_or(12);
            text.chars().count() as f64 * font_size as f64 * 0.6
        }
        Widget::Button { label, .. } => label.len() as f64 * 8.0 + 16.0,
        Widget::Input { width, .. } => width.map(|w| w as f64).unwrap_or(200.0),
        _ => 100.0,
    }
}

fn color_to_hex(c: Color) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
}
