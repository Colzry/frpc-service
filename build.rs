use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let ico_path = out_dir.join("app.ico");

    let size = 32u32;
    let pixels = generate_icon(size);
    let mask = vec![0u8; ((size * size) / 8) as usize];
    let image_data = build_ico_image(size, &pixels, &mask);

    let ico_data = build_ico_file(&image_data);
    fs::write(&ico_path, &ico_data).expect("无法写入图标文件");

    if cfg!(target_os = "windows") {
        let mut res = winres::WindowsResource::new();
        res.set_icon(ico_path.to_str().unwrap());
        res.compile().expect("无法编译 Windows 资源");
    }
}

const COL_BG: [u8; 4] = [0x1B, 0x1B, 0x30, 0xFF]; // 深蓝黑底
const COL_BORDER: [u8; 4] = [0x4A, 0x90, 0xD9, 0xFF]; // 中蓝边框
const COL_F: [u8; 4] = [0xFF, 0xFF, 0xFF, 0xFF]; // 纯白字
const COL_ACCENT: [u8; 4] = [0x3A, 0x78, 0xBF, 0xFF]; // 蓝色装饰线
const COL_DOT: [u8; 4] = [0x80, 0xC0, 0xFF, 0xFF]; // 浅蓝圆点

fn fill_rect(pixels: &mut [u8], w: u32, x1: i32, y1: i32, x2: i32, y2: i32, c: [u8; 4]) {
    for y in y1.max(0)..=y2.min(w as i32 - 1) {
        for x in x1.max(0)..=x2.min(w as i32 - 1) {
            let i = ((y as u32 * w + x as u32) * 4) as usize;
            pixels[i..i + 4].copy_from_slice(&c);
        }
    }
}

fn fill_rounded_rect(
    pixels: &mut [u8],
    w: u32,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    r: i32,
    c: [u8; 4],
) {
    for y in y1.max(0)..=y2.min(w as i32 - 1) {
        for x in x1.max(0)..=x2.min(w as i32 - 1) {
            let in_corner = |cx: i32, cy: i32| {
                let dx = x - cx;
                let dy = y - cy;
                dx * dx + dy * dy <= r * r
            };
            let inside = if x < x1 + r && y < y1 + r {
                in_corner(x1 + r, y1 + r)
            } else if x > x2 - r && y < y1 + r {
                in_corner(x2 - r, y1 + r)
            } else if x < x1 + r && y > y2 - r {
                in_corner(x1 + r, y2 - r)
            } else if x > x2 - r && y > y2 - r {
                in_corner(x2 - r, y2 - r)
            } else {
                true
            };
            if inside {
                let i = ((y as u32 * w + x as u32) * 4) as usize;
                pixels[i..i + 4].copy_from_slice(&c);
            }
        }
    }
}

fn draw_h_line(pixels: &mut [u8], w: u32, x1: i32, x2: i32, y: i32, c: [u8; 4]) {
    for x in x1.max(0)..=x2.min(w as i32 - 1) {
        let i = ((y as u32 * w + x as u32) * 4) as usize;
        pixels[i..i + 4].copy_from_slice(&c);
    }
}

fn draw_dot(pixels: &mut [u8], w: u32, cx: i32, cy: i32, c: [u8; 4]) {
    for dy in -1..=1 {
        for dx in -1..=1 {
            let x = cx + dx;
            let y = cy + dy;
            if x >= 0 && x < w as i32 && y >= 0 && y < w as i32 {
                if dx.abs() + dy.abs() <= 1 {
                    let i = ((y as u32 * w + x as u32) * 4) as usize;
                    pixels[i..i + 4].copy_from_slice(&c);
                }
            }
        }
    }
}

fn generate_icon(size: u32) -> Vec<u8> {
    let s = size as i32;
    let mut px = vec![0u8; (size * size * 4) as usize];

    // 1. 深色背景
    fill_rect(&mut px, size, 0, 0, s - 1, s - 1, COL_BG);

    // 2. 外边框（圆角矩形）
    fill_rounded_rect(&mut px, size, 0, 0, s - 1, s - 1, 5, COL_BORDER);

    // 3. 内部填充背景（覆盖中间区域，留出边框）
    fill_rounded_rect(&mut px, size, 2, 2, s - 3, s - 3, 4, COL_BG);

    // 4. 底部装饰线
    draw_h_line(&mut px, size, 5, s - 6, s - 5, COL_ACCENT);

    // 5. 字母 "F" — 粗体风格，4px 笔画
    // 垂直主笔
    fill_rect(&mut px, size, 10, 5, 13, 25, COL_F);
    // 顶部横笔
    fill_rect(&mut px, size, 10, 5, 23, 8, COL_F);
    // 中间横笔
    fill_rect(&mut px, size, 10, 14, 20, 17, COL_F);

    // 6. 底部网络连接圆点
    draw_dot(&mut px, size, 10, 27, COL_DOT);
    draw_dot(&mut px, size, 16, 27, COL_DOT);
    draw_dot(&mut px, size, 22, 27, COL_DOT);

    px
}

fn build_ico_image(size: u32, pixels: &[u8], mask: &[u8]) -> Vec<u8> {
    let mut data = Vec::new();
    let row_bytes = size as usize * 4;

    data.extend_from_slice(&40u32.to_le_bytes());
    data.extend_from_slice(&size.to_le_bytes());
    data.extend_from_slice(&(size * 2).to_le_bytes());
    data.extend_from_slice(&1u16.to_le_bytes());
    data.extend_from_slice(&32u16.to_le_bytes());
    data.extend_from_slice(&0u32.to_le_bytes());
    data.extend_from_slice(&(pixels.len() as u32 + mask.len() as u32).to_le_bytes());
    data.extend_from_slice(&[0u8; 16]);

    for y in (0..size as usize).rev() {
        let start = y * row_bytes;
        data.extend_from_slice(&pixels[start..start + row_bytes]);
    }
    data.extend_from_slice(mask);
    data
}

fn build_ico_file(image_data: &[u8]) -> Vec<u8> {
    let mut ico = Vec::new();
    ico.extend_from_slice(&0u16.to_le_bytes());
    ico.extend_from_slice(&1u16.to_le_bytes());
    ico.extend_from_slice(&1u16.to_le_bytes());
    ico.push(0);
    ico.push(0);
    ico.push(0);
    ico.push(0);
    ico.extend_from_slice(&1u16.to_le_bytes());
    ico.extend_from_slice(&32u16.to_le_bytes());
    ico.extend_from_slice(&(image_data.len() as u32).to_le_bytes());
    ico.extend_from_slice(&22u32.to_le_bytes());
    ico.extend_from_slice(image_data);
    ico
}
