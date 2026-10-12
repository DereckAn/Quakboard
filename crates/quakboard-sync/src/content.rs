//! What kind of text a clip is: a link, a color, SVG, code or plain text.
//! Shared so every app labels clips the same way.

/// `link`, `color`, `svg`, `code` or `text`.
pub fn detect_content_type(text: &str) -> String {
    // URL detection
    if text.starts_with("http://") || text.starts_with("https://") {
        return "link".to_string();
    }

    // Color detection (hex, rgb, rgba, hsl)
    if is_color(text) {
        return "color".to_string();
    }

    // SVG detection (before code detection to avoid false positives)
    if is_svg(text) {
        return "svg".to_string();
    }

    // Code detection (simple heuristic)
    if is_code(text) {
        return "code".to_string();
    }

    // Default to text
    "text".to_string()
}

fn is_svg(text: &str) -> bool {
    let trimmed = text.trim();

    // Check if starts with <svg or <?xml and contains <svg
    if trimmed.starts_with("<svg") {
        return true;
    }

    // Check for XML declaration with SVG
    if trimmed.starts_with("<?xml") && trimmed.contains("<svg") {
        return true;
    }

    false
}

fn is_color(text: &str) -> bool {
    let trimmed = text.trim();

    // Hex color
    if trimmed.starts_with('#') && (trimmed.len() == 7 || trimmed.len() == 9) {
        return trimmed[1..].chars().all(|c| c.is_ascii_hexdigit());
    }

    // RGB/RGBA
    if trimmed.starts_with("rgb(") || trimmed.starts_with("rgba(") {
        return true;
    }

    // HSL/HSLA
    if trimmed.starts_with("hsl(") || trimmed.starts_with("hsla(") {
        return true;
    }

    // OKLCH
    if trimmed.starts_with("oklch(") {
        return true;
    }

    false
}

fn is_code(text: &str) -> bool {
    // Simple heuristics for code detection
    let code_indicators = [
        "function ",
        "const ",
        "let ",
        "var ",
        "class ",
        "import ",
        "export ",
        "def ",
        "print(",
        "console.log",
        "public ",
        "private ",
        "protected ",
        "<?php",
        "#!/",
        "<script",
        "</",
        "/>",
        "=>",
        "fn ",
        "impl ",
        "struct ",
        "{",
        "}",
        ";",
        "package ",
        "interface ",
        "func ",
        "using ",
        "namespace ",
    ];

    let indicator_count = code_indicators
        .iter()
        .filter(|indicator| text.contains(*indicator))
        .count();

    // If has 2+ code indicators, likely code
    indicator_count >= 2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_address_is_a_link() {
        assert_eq!(
            detect_content_type("https://github.com/DereckAn/Quakboard"),
            "link"
        );
    }

    #[test]
    fn hex_color_is_a_color() {
        assert_eq!(detect_content_type("#DDD2BD"), "color");
    }

    #[test]
    fn svg_markup_is_svg() {
        assert_eq!(
            detect_content_type("<svg viewBox=\"0 0 1 1\"></svg>"),
            "svg"
        );
    }

    #[test]
    fn rust_function_is_code() {
        assert_eq!(
            detect_content_type("fn main() {\n    println!(\"hi\");\n}"),
            "code"
        );
    }

    #[test]
    fn sentence_is_text() {
        assert_eq!(
            detect_content_type("Standup at 10:00 with the team"),
            "text"
        );
    }
}
