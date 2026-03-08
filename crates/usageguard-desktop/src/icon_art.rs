#[derive(Clone, Copy)]
struct Point {
    x: f32,
    y: f32,
}

#[derive(Clone, Copy)]
struct Color {
    r: f32,
    g: f32,
    b: f32,
    a: f32,
}

impl Point {
    const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    fn offset(self, other: Point) -> Self {
        Self::new(self.x - other.x, self.y - other.y)
    }
}

impl Color {
    const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self {
            r: r as f32 / 255.0,
            g: g as f32 / 255.0,
            b: b as f32 / 255.0,
            a: 1.0,
        }
    }

    const fn transparent() -> Self {
        Self {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        }
    }
}

#[allow(dead_code)]
pub const BUNDLE_ICON_SIZES: &[u32] = &[256, 1024];
#[allow(dead_code)]
pub const ICO_ICON_SIZES: &[u32] = &[16, 24, 32, 48, 64, 128, 256];
#[allow(dead_code)]
pub const TRAY_ICON_SIZE: u32 = 512;

pub fn icon_rgba_pixels(size: u32) -> Vec<u8> {
    let mut pixels = vec![0u8; (size * size * 4) as usize];
    let samples = [
        (-0.375f32, -0.375f32),
        (-0.125, -0.375),
        (0.125, -0.375),
        (0.375, -0.375),
        (-0.375, -0.125),
        (-0.125, -0.125),
        (0.125, -0.125),
        (0.375, -0.125),
        (-0.375, 0.125),
        (-0.125, 0.125),
        (0.125, 0.125),
        (0.375, 0.125),
        (-0.375, 0.375),
        (-0.125, 0.375),
        (0.125, 0.375),
        (0.375, 0.375),
    ];

    for y in 0..size {
        for x in 0..size {
            let mut premul_r = 0.0f32;
            let mut premul_g = 0.0f32;
            let mut premul_b = 0.0f32;
            let mut alpha_sum = 0.0f32;
            for (ox, oy) in samples {
                let px = ((x as f32 + 0.5 + ox) / size as f32) * 2.0 - 1.0;
                let py = ((y as f32 + 0.5 + oy) / size as f32) * 2.0 - 1.0;
                let sample = sample_icon(Point::new(px, py));
                premul_r += sample.r * sample.a;
                premul_g += sample.g * sample.a;
                premul_b += sample.b * sample.a;
                alpha_sum += sample.a;
            }

            let avg_alpha = alpha_sum / samples.len() as f32;
            let idx = ((y * size + x) * 4) as usize;
            if alpha_sum > 0.0 {
                pixels[idx] = to_byte(premul_r / alpha_sum);
                pixels[idx + 1] = to_byte(premul_g / alpha_sum);
                pixels[idx + 2] = to_byte(premul_b / alpha_sum);
                pixels[idx + 3] = to_byte(avg_alpha);
            }
        }
    }

    pixels
}

fn sample_icon(p: Point) -> Color {
    use std::f32::consts::{FRAC_PI_2, TAU};

    let ring_center = Point::new(0.0, 0.0);
    let ring_radius = 0.78;
    let ring_thickness = 0.42;
    let ring_start = -FRAC_PI_2;
    let ring_sweep = TAU * (240.0 / 360.0);

    if inside_ring_arc(
        p,
        ring_center,
        ring_radius,
        ring_thickness,
        ring_start,
        ring_sweep,
    ) {
        Color::rgb(82, 213, 119)
    } else {
        Color::transparent()
    }
}

fn inside_ring(point: Point, center: Point, radius: f32, thickness: f32) -> bool {
    let local = point.offset(center);
    let distance = (local.x * local.x + local.y * local.y).sqrt();
    (distance - radius).abs() <= thickness * 0.5
}

fn inside_ring_arc(
    point: Point,
    center: Point,
    radius: f32,
    thickness: f32,
    start_angle: f32,
    sweep_angle: f32,
) -> bool {
    let local = point.offset(center);
    let angle = local.y.atan2(local.x);
    let end_angle = start_angle + sweep_angle;

    inside_ring(point, center, radius, thickness) && angle_in_arc(angle, start_angle, end_angle)
}

fn angle_in_arc(angle: f32, start: f32, end: f32) -> bool {
    use std::f32::consts::TAU;

    let normalize = |value: f32| value.rem_euclid(TAU);
    let angle = normalize(angle);
    let start = normalize(start);
    let end = normalize(end);

    if start <= end {
        angle >= start && angle <= end
    } else {
        angle >= start || angle <= end
    }
}

fn to_byte(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}
