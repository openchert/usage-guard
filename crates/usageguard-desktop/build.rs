use std::fs;
use std::path::Path;

fn main() {
    generate_placeholder_icons();
    tauri_build::build();
}

fn generate_placeholder_icons() {
    let icons_dir = Path::new("icons");
    fs::create_dir_all(icons_dir).unwrap();

    let p32 = icons_dir.join("32x32.png");
    if !p32.exists() {
        write_icon_png(&p32, 32);
    }
    let p128 = icons_dir.join("128x128.png");
    if !p128.exists() {
        write_icon_png(&p128, 128);
    }

    // Windows resource file requires an .ico
    let ico_path = icons_dir.join("icon.ico");
    if !ico_path.exists() {
        write_icon_ico(&ico_path, 32);
    }
}

fn write_icon_ico(path: &Path, size: u32) {
    use ico::{IconDir, IconDirEntry, IconImage, ResourceType};
    let pixels = icon_rgba_pixels(size);
    let image = IconImage::from_rgba_data(size, size, pixels);
    let mut icon_dir = IconDir::new(ResourceType::Icon);
    icon_dir.add_entry(IconDirEntry::encode(&image).unwrap());
    let file = fs::File::create(path).unwrap();
    icon_dir.write(file).unwrap();
}

fn icon_rgba_pixels(size: u32) -> Vec<u8> {
    let half = (size / 2) as i32;
    let margin = (size as i32 / 7).max(2);
    let r2 = (half - margin).pow(2);
    let mut pixels = vec![0u8; (size * size * 4) as usize];
    for y in 0..size as i32 {
        for x in 0..size as i32 {
            let idx = ((y * size as i32 + x) * 4) as usize;
            let dx = x - half;
            let dy = y - half;
            if dx * dx + dy * dy <= r2 {
                pixels[idx] = 100;
                pixels[idx + 1] = 160;
                pixels[idx + 2] = 255;
                pixels[idx + 3] = 220;
            }
        }
    }
    pixels
}

fn write_icon_png(path: &Path, size: u32) {
    let file = fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(file, size, size);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(&icon_rgba_pixels(size)).unwrap();
}
