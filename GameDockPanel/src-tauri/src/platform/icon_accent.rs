//! Accent-color sampling from RGBA icon pixels — shared between macOS and Windows.

/// Picks a saturated accent color from icon pixels for the running-app LED.
pub fn accent_color_from_rgba(width: u32, height: u32, rgba: &[u8]) -> Option<String> {
    if width == 0 || height == 0 || rgba.is_empty() {
        return None;
    }

    const HUE_BUCKETS: usize = 16;
    let mut bucket_weight = [0.0f64; HUE_BUCKETS];
    let mut bucket_hue = [0.0f64; HUE_BUCKETS];
    let mut bucket_sat = [0.0f64; HUE_BUCKETS];
    let mut bucket_light = [0.0f64; HUE_BUCKETS];
    let mut bucket_count = [0u32; HUE_BUCKETS];

    let step_x = (width / 32).max(1);
    let step_y = (height / 32).max(1);

    for y in (0..height).step_by(step_y as usize) {
        for x in (0..width).step_by(step_x as usize) {
            let offset = ((y * width + x) * 4) as usize;
            if offset + 3 >= rgba.len() {
                continue;
            }
            let r = rgba[offset];
            let g = rgba[offset + 1];
            let b = rgba[offset + 2];
            let a = rgba[offset + 3];
            if a < 128 {
                continue;
            }

            let (h, s, l) = rgb_to_hsl(r, g, b);
            if l < 0.15 || l > 0.92 || s < 0.25 {
                continue;
            }

            let bucket = ((h * HUE_BUCKETS as f64).floor() as usize).min(HUE_BUCKETS - 1);
            let weight = s * s;
            bucket_weight[bucket] += weight;
            bucket_hue[bucket] += h * weight;
            bucket_sat[bucket] += s * weight;
            bucket_light[bucket] += l * weight;
            bucket_count[bucket] += 1;
        }
    }

    let (best_bucket, _) = bucket_weight
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))?;

    if bucket_weight[best_bucket] <= 0.0 || bucket_count[best_bucket] == 0 {
        return None;
    }

    let total = bucket_weight[best_bucket];
    let h = bucket_hue[best_bucket] / total;
    let s = (bucket_sat[best_bucket] / total * 1.1).clamp(0.35, 1.0);
    let l = (bucket_light[best_bucket] / total).clamp(0.5, 0.72);
    let (r, g, b) = hsl_to_rgb(h, s, l);
    Some(format!("#{:02x}{:02x}{:02x}", r, g, b))
}

fn rgb_to_hsl(r: u8, g: u8, b: u8) -> (f64, f64, f64) {
    let r = r as f64 / 255.0;
    let g = g as f64 / 255.0;
    let b = b as f64 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if (max - min).abs() < f64::EPSILON {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if (max - r).abs() < f64::EPSILON {
        let mut hue = (g - b) / d;
        if g < b {
            hue += 6.0;
        }
        hue / 6.0
    } else if (max - g).abs() < f64::EPSILON {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    (h, s, l)
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (u8, u8, u8) {
    if s <= f64::EPSILON {
        let v = (l * 255.0).round() as u8;
        return (v, v, v);
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let r = hue_to_rgb_channel(p, q, h + 1.0 / 3.0);
    let g = hue_to_rgb_channel(p, q, h);
    let b = hue_to_rgb_channel(p, q, h - 1.0 / 3.0);
    (
        (r * 255.0).round() as u8,
        (g * 255.0).round() as u8,
        (b * 255.0).round() as u8,
    )
}

fn hue_to_rgb_channel(p: f64, q: f64, mut t: f64) -> f64 {
    if t < 0.0 {
        t += 1.0;
    }
    if t > 1.0 {
        t -= 1.0;
    }
    if t < 1.0 / 6.0 {
        return p + (q - p) * 6.0 * t;
    }
    if t < 1.0 / 2.0 {
        return q;
    }
    if t < 2.0 / 3.0 {
        return p + (q - p) * (2.0 / 3.0 - t) * 6.0;
    }
    p
}

pub fn icon_export_px(icon_size_dip: f64, scale_factor: f64) -> u32 {
    const ICON_EXPORT_MAX_PX: f64 = 512.0;
    const ICON_EXPORT_MIN_PX: f64 = 128.0;
    const MAGNIFY_MAX_SCALE: f64 = 1.4;
    (icon_size_dip * MAGNIFY_MAX_SCALE * scale_factor)
        .ceil()
        .clamp(ICON_EXPORT_MIN_PX, ICON_EXPORT_MAX_PX) as u32
}
