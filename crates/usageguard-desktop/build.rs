#[path = "src/icon_art.rs"]
mod icon_art;

use std::fs;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/icon_art.rs");
    configure_windows_sdk_env();
    generate_icons();
    tauri_build::build();
}

fn generate_icons() {
    let icons_dir = Path::new("icons");
    fs::create_dir_all(icons_dir).unwrap();

    for size in icon_art::BUNDLE_ICON_SIZES {
        let path = icons_dir.join(format!("{size}x{size}.png"));
        write_icon_png(&path, *size);
    }

    let ico_path = icons_dir.join("icon.ico");
    write_icon_ico(&ico_path);
}

fn write_icon_ico(path: &Path) {
    use ico::{IconDir, IconDirEntry, IconImage, ResourceType};
    let mut icon_dir = IconDir::new(ResourceType::Icon);

    for size in icon_art::ICO_ICON_SIZES {
        let pixels = icon_art::icon_rgba_pixels(*size);
        let image = IconImage::from_rgba_data(*size, *size, pixels);
        icon_dir.add_entry(IconDirEntry::encode(&image).unwrap());
    }

    let file = fs::File::create(path).unwrap();
    icon_dir.write(file).unwrap();
}

fn write_icon_png(path: &Path, size: u32) {
    let file = fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(file, size, size);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer
        .write_image_data(&icon_art::icon_rgba_pixels(size))
        .unwrap();
}

fn configure_windows_sdk_env() {
    #[cfg(target_os = "windows")]
    {
        use std::env;

        if env::var("CARGO_CFG_TARGET_OS").ok().as_deref() != Some("windows") {
            return;
        }

        if command_exists("rc.exe") {
            return;
        }

        let Some((bin_dir, include_root)) = find_windows_sdk() else {
            return;
        };

        prepend_path(&bin_dir);
        extend_include_path(&include_root);
    }
}

#[cfg(target_os = "windows")]
fn command_exists(command: &str) -> bool {
    std::process::Command::new("where")
        .arg(command)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[cfg(target_os = "windows")]
fn find_windows_sdk() -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    use std::env;
    use std::path::PathBuf;

    let programs = [
        env::var_os("ProgramFiles(x86)").map(PathBuf::from),
        env::var_os("ProgramFiles").map(PathBuf::from),
    ];

    for base in programs.into_iter().flatten() {
        let root = base.join("Windows Kits").join("10");
        let Some((bin_dir, version)) = find_sdk_bin_dir(&root) else {
            continue;
        };

        let include_root = root.join("Include").join(version);
        if include_root.is_dir() {
            return Some((bin_dir, include_root));
        }
    }

    None
}

#[cfg(target_os = "windows")]
fn find_sdk_bin_dir(root: &std::path::Path) -> Option<(std::path::PathBuf, String)> {
    use std::env;
    use std::path::PathBuf;

    let host = env::var("HOST").unwrap_or_default();
    let arch_dir = if host.starts_with("aarch64") {
        "arm64"
    } else if host.starts_with("x86_64") {
        "x64"
    } else {
        "x86"
    };

    let bin_root = root.join("bin");
    let mut versions = fs::read_dir(&bin_root)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
        .filter_map(|entry| {
            let version = entry.file_name().to_string_lossy().into_owned();
            let candidate = entry.path().join(arch_dir).join("rc.exe");
            candidate
                .exists()
                .then_some((version, entry.path().join(arch_dir)))
        })
        .collect::<Vec<(String, PathBuf)>>();

    versions.sort_by(|left, right| natural_cmp(&left.0, &right.0));
    versions.pop().map(|(version, bin_dir)| (bin_dir, version))
}

#[cfg(target_os = "windows")]
fn prepend_path(dir: &std::path::Path) {
    use std::env;

    let mut paths = env::split_paths(&env::var_os("PATH").unwrap_or_default()).collect::<Vec<_>>();
    if paths.iter().any(|path| path == dir) {
        return;
    }

    paths.insert(0, dir.to_path_buf());
    let joined = env::join_paths(paths).unwrap();
    env::set_var("PATH", joined);
}

#[cfg(target_os = "windows")]
fn extend_include_path(include_root: &std::path::Path) {
    use std::env;

    let mut includes =
        env::split_paths(&env::var_os("INCLUDE").unwrap_or_default()).collect::<Vec<_>>();
    let extra = fs::read_dir(include_root)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
        .map(|entry| entry.path())
        .collect::<Vec<_>>();

    let mut changed = false;
    for dir in extra {
        if includes.iter().any(|path| path == &dir) {
            continue;
        }
        includes.push(dir);
        changed = true;
    }

    if changed {
        let joined = env::join_paths(includes).unwrap();
        env::set_var("INCLUDE", joined);
    }
}

#[cfg(target_os = "windows")]
fn natural_cmp(left: &str, right: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    let parse = |value: &str| {
        value
            .split('.')
            .map(|part| part.parse::<u32>().unwrap_or(0))
            .collect::<Vec<_>>()
    };

    let left = parse(left);
    let right = parse(right);
    for (lhs, rhs) in left.iter().zip(right.iter()) {
        match lhs.cmp(rhs) {
            Ordering::Equal => continue,
            order => return order,
        }
    }
    left.len().cmp(&right.len())
}
