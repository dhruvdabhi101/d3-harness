use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

pub fn wrap_line(line: Line<'static>, width: u16) -> Vec<Line<'static>> {
    let width = usize::from(width);
    if width == 0 || line.width() <= width {
        return vec![line];
    }
    let plain: String = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect();
    let indent = continuation_indent(&plain).min(width.saturating_sub(2));
    let mut glyphs = Vec::new();
    for span in &line.spans {
        glyphs.extend(
            span.content
                .graphemes(true)
                .map(|glyph| (glyph.to_string(), span.style)),
        );
    }
    let mut output = Vec::new();
    let mut start = 0;
    let mut first = true;
    while start < glyphs.len() {
        let padding = if first { 0 } else { indent };
        let mut used = padding;
        let mut end = start;
        let mut word_break = None;
        while end < glyphs.len() {
            let glyph = &glyphs[end].0;
            if used + glyph.width() > width {
                break;
            }
            used += glyph.width();
            if glyph.chars().all(char::is_whitespace) {
                word_break = Some(end);
            }
            end += 1;
        }
        if end < glyphs.len()
            && let Some(boundary) = word_break.filter(|boundary| *boundary > start)
        {
            end = boundary;
        }
        if end == start {
            end += 1;
        }
        let mut spans = Vec::new();
        if padding > 0 {
            spans.push(Span::raw(" ".repeat(padding)));
        }
        spans.extend(
            glyphs[start..end]
                .iter()
                .map(|(glyph, style)| Span::styled(glyph.clone(), *style)),
        );
        output.push(Line::from(spans).style(line.style));
        start = end;
        while start < glyphs.len() && glyphs[start].0.chars().all(char::is_whitespace) {
            start += 1;
        }
        first = false;
    }
    output
}

fn continuation_indent(text: &str) -> usize {
    let leading = text.len() - text.trim_start().len();
    let mut offset = leading;
    let mut rest = &text[offset..];
    if rest.starts_with("◆ ") || rest.starts_with("› ") {
        offset += rest.chars().next().unwrap().len_utf8() + 1;
        rest = &text[offset..];
    }
    let spaces = rest.len() - rest.trim_start().len();
    offset += spaces;
    rest = &text[offset..];
    let marker = rest.split_whitespace().next().unwrap_or("");
    if matches!(marker, "•" | "-" | "*" | "+" | "│" | ">")
        || marker.strip_suffix(['.', ')']).is_some_and(|number| {
            !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
        })
    {
        offset += marker.len();
        offset += text[offset..].len() - text[offset..].trim_start().len();
    }
    text[..offset].width()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuation_preserves_indents_markers_styles_and_unicode() {
        for (prefix, indent) in [
            ("   ◆ • ", 7),
            ("   12. ", 7),
            ("     ", 5),
            (" › ", 3),
            ("   │ ", 5),
        ] {
            let style = ratatui::style::Style::default().fg(ratatui::style::Color::Green);
            let line = Line::from(vec![
                Span::raw(prefix.to_string()),
                Span::styled("界e\u{301} alpha beta gamma delta epsilon zeta", style),
            ]);
            let wrapped = wrap_line(line, 18);
            assert!(wrapped.len() > 1);
            for line in &wrapped[1..] {
                let plain: String = line
                    .spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect();
                assert_eq!(
                    plain.chars().take_while(|ch| ch.is_whitespace()).count(),
                    indent,
                    "{plain:?}"
                );
                assert!(line.width() <= 18);
                assert!(line.spans.iter().any(|span| span.style == style));
            }
        }
    }
}
