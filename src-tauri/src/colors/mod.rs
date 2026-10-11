use palette::{Hsl, IntoColor, Oklch, Srgb, Srgba};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorFormat {
    pub name: String,
    pub value: String,
    pub original: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorConversion {
    pub formats: Vec<ColorFormat>,
    pub rgb_preview: String,
}

pub fn convert_color(color_string: &str) -> Result<ColorConversion, String> {
    let trimmed = color_string.trim();
    let mut formats = Vec::new();

    // Parse the color based on format
    let (rgba, original_format) = parse_color(trimmed)?;

    // Add original format first
    formats.push(ColorFormat {
        name: original_format.clone(),
        value: trimmed.to_string(),
        original: true,
    });

    // Generate all other formats
    let rgb = Srgba::new(rgba.0, rgba.1, rgba.2, rgba.3);

    // HEX
    if original_format != "HEX" {
        let hex = if rgba.3 < 1.0 {
            format!(
                "#{:02X}{:02X}{:02X}{:02X}",
                (rgba.0 * 255.0) as u8,
                (rgba.1 * 255.0) as u8,
                (rgba.2 * 255.0) as u8,
                (rgba.3 * 255.0) as u8
            )
        } else {
            format!(
                "#{:02X}{:02X}{:02X}",
                (rgba.0 * 255.0) as u8,
                (rgba.1 * 255.0) as u8,
                (rgba.2 * 255.0) as u8
            )
        };
        formats.push(ColorFormat {
            name: "HEX".to_string(),
            value: hex,
            original: false,
        });
    }

    // RGB
    if original_format != "RGB" {
        let rgb_str = format!(
            "rgb({}, {}, {})",
            (rgba.0 * 255.0) as u8,
            (rgba.1 * 255.0) as u8,
            (rgba.2 * 255.0) as u8
        );
        formats.push(ColorFormat {
            name: "RGB".to_string(),
            value: rgb_str,
            original: false,
        });
    }

    // RGBA (if has alpha)
    if rgba.3 < 1.0 && original_format != "RGBA" {
        let rgba_str = format!(
            "rgba({}, {}, {}, {:.2})",
            (rgba.0 * 255.0) as u8,
            (rgba.1 * 255.0) as u8,
            (rgba.2 * 255.0) as u8,
            rgba.3
        );
        formats.push(ColorFormat {
            name: "RGBA".to_string(),
            value: rgba_str,
            original: false,
        });
    }

    // HSL
    if original_format != "HSL" {
        let hsl: Hsl = rgb.into_color();
        let hsl_str = format!(
            "hsl({:.0}, {:.0}%, {:.0}%)",
            hsl.hue.into_positive_degrees(),
            hsl.saturation * 100.0,
            hsl.lightness * 100.0
        );
        formats.push(ColorFormat {
            name: "HSL".to_string(),
            value: hsl_str,
            original: false,
        });
    }

    // HSLA (if has alpha)
    if rgba.3 < 1.0 && original_format != "HSLA" {
        let hsl: Hsl = rgb.into_color();
        let hsla_str = format!(
            "hsla({:.0}, {:.0}%, {:.0}%, {:.2})",
            hsl.hue.into_positive_degrees(),
            hsl.saturation * 100.0,
            hsl.lightness * 100.0,
            rgba.3
        );
        formats.push(ColorFormat {
            name: "HSLA".to_string(),
            value: hsla_str,
            original: false,
        });
    }

    // OKLCH
    if original_format != "OKLCH" {
        let oklch: Oklch = rgb.into_color();
        let oklch_str = format!(
            "oklch({:.3} {:.3} {:.1})",
            oklch.l,
            oklch.chroma,
            oklch.hue.into_positive_degrees()
        );
        formats.push(ColorFormat {
            name: "OKLCH".to_string(),
            value: oklch_str,
            original: false,
        });
    }

    // RGB preview for displaying the color
    let rgb_preview = format!(
        "rgb({}, {}, {})",
        (rgba.0 * 255.0) as u8,
        (rgba.1 * 255.0) as u8,
        (rgba.2 * 255.0) as u8
    );

    Ok(ColorConversion {
        formats,
        rgb_preview,
    })
}

/// Red, green, blue and alpha, each from 0 to 1.
type Rgba = (f32, f32, f32, f32);

fn parse_color(color_string: &str) -> Result<(Rgba, String), String> {
    let trimmed = color_string.trim();

    // HEX
    if trimmed.starts_with('#') {
        return parse_hex(trimmed);
    }

    // RGB
    if trimmed.starts_with("rgb(") {
        return parse_rgb(trimmed);
    }

    // RGBA
    if trimmed.starts_with("rgba(") {
        return parse_rgba(trimmed);
    }

    // HSL
    if trimmed.starts_with("hsl(") {
        return parse_hsl(trimmed);
    }

    // HSLA
    if trimmed.starts_with("hsla(") {
        return parse_hsla(trimmed);
    }

    // OKLCH
    if trimmed.starts_with("oklch(") {
        return parse_oklch(trimmed);
    }

    Err("Unsupported color format".to_string())
}

fn parse_hex(hex: &str) -> Result<(Rgba, String), String> {
    let hex = hex.trim_start_matches('#');

    let (r, g, b, a) = match hex.len() {
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).map_err(|e| e.to_string())?;
            let g = u8::from_str_radix(&hex[2..4], 16).map_err(|e| e.to_string())?;
            let b = u8::from_str_radix(&hex[4..6], 16).map_err(|e| e.to_string())?;
            (r, g, b, 255)
        }
        8 => {
            let r = u8::from_str_radix(&hex[0..2], 16).map_err(|e| e.to_string())?;
            let g = u8::from_str_radix(&hex[2..4], 16).map_err(|e| e.to_string())?;
            let b = u8::from_str_radix(&hex[4..6], 16).map_err(|e| e.to_string())?;
            let a = u8::from_str_radix(&hex[6..8], 16).map_err(|e| e.to_string())?;
            (r, g, b, a)
        }
        _ => return Err("Invalid hex color format".to_string()),
    };

    Ok((
        (
            r as f32 / 255.0,
            g as f32 / 255.0,
            b as f32 / 255.0,
            a as f32 / 255.0,
        ),
        "HEX".to_string(),
    ))
}

fn parse_rgb(rgb: &str) -> Result<(Rgba, String), String> {
    let inner = rgb.trim_start_matches("rgb(").trim_end_matches(')');
    let parts: Vec<&str> = inner.split(',').collect();

    if parts.len() != 3 {
        return Err("Invalid RGB format".to_string());
    }

    let r = parts[0].trim().parse::<u8>().map_err(|e| e.to_string())?;
    let g = parts[1].trim().parse::<u8>().map_err(|e| e.to_string())?;
    let b = parts[2].trim().parse::<u8>().map_err(|e| e.to_string())?;

    Ok((
        (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0),
        "RGB".to_string(),
    ))
}

fn parse_rgba(rgba: &str) -> Result<(Rgba, String), String> {
    let inner = rgba.trim_start_matches("rgba(").trim_end_matches(')');
    let parts: Vec<&str> = inner.split(',').collect();

    if parts.len() != 4 {
        return Err("Invalid RGBA format".to_string());
    }

    let r = parts[0].trim().parse::<u8>().map_err(|e| e.to_string())?;
    let g = parts[1].trim().parse::<u8>().map_err(|e| e.to_string())?;
    let b = parts[2].trim().parse::<u8>().map_err(|e| e.to_string())?;
    let a = parts[3].trim().parse::<f32>().map_err(|e| e.to_string())?;

    Ok((
        (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a),
        "RGBA".to_string(),
    ))
}

fn parse_hsl(hsl: &str) -> Result<(Rgba, String), String> {
    let inner = hsl.trim_start_matches("hsl(").trim_end_matches(')');
    let parts: Vec<&str> = inner.split(',').collect();

    if parts.len() != 3 {
        return Err("Invalid HSL format".to_string());
    }

    let h = parts[0].trim().parse::<f32>().map_err(|e| e.to_string())?;
    let s = parts[1]
        .trim()
        .trim_end_matches('%')
        .parse::<f32>()
        .map_err(|e| e.to_string())?
        / 100.0;
    let l = parts[2]
        .trim()
        .trim_end_matches('%')
        .parse::<f32>()
        .map_err(|e| e.to_string())?
        / 100.0;

    let hsl_color = Hsl::new(h, s, l);
    let rgb: Srgb = hsl_color.into_color();

    Ok(((rgb.red, rgb.green, rgb.blue, 1.0), "HSL".to_string()))
}

fn parse_hsla(hsla: &str) -> Result<(Rgba, String), String> {
    let inner = hsla.trim_start_matches("hsla(").trim_end_matches(')');
    let parts: Vec<&str> = inner.split(',').collect();

    if parts.len() != 4 {
        return Err("Invalid HSLA format".to_string());
    }

    let h = parts[0].trim().parse::<f32>().map_err(|e| e.to_string())?;
    let s = parts[1]
        .trim()
        .trim_end_matches('%')
        .parse::<f32>()
        .map_err(|e| e.to_string())?
        / 100.0;
    let l = parts[2]
        .trim()
        .trim_end_matches('%')
        .parse::<f32>()
        .map_err(|e| e.to_string())?
        / 100.0;
    let a = parts[3].trim().parse::<f32>().map_err(|e| e.to_string())?;

    let hsl_color = Hsl::new(h, s, l);
    let rgb: Srgb = hsl_color.into_color();

    Ok(((rgb.red, rgb.green, rgb.blue, a), "HSLA".to_string()))
}

fn parse_oklch(oklch: &str) -> Result<(Rgba, String), String> {
    let inner = oklch.trim_start_matches("oklch(").trim_end_matches(')');
    let parts: Vec<&str> = inner.split_whitespace().collect();

    if parts.len() != 3 {
        return Err("Invalid OKLCH format".to_string());
    }

    let l = parts[0].parse::<f32>().map_err(|e| e.to_string())?;
    let c = parts[1].parse::<f32>().map_err(|e| e.to_string())?;
    let h = parts[2].parse::<f32>().map_err(|e| e.to_string())?;

    let oklch_color = Oklch::new(l, c, h);
    let rgb: Srgb = oklch_color.into_color();

    Ok(((rgb.red, rgb.green, rgb.blue, 1.0), "OKLCH".to_string()))
}
