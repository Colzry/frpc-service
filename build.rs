use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let ico_path = out_dir.join("app.ico");

    // 生成多个尺寸的图标
    let sizes = [16u32, 32, 48, 64, 128, 256];
    let mut images = Vec::new();

    for &size in &sizes {
        let pixels = generate_icon(size);
        let mask = vec![0u8; ((size * size) / 8) as usize];
        let image_data = build_ico_image(size, &pixels, &mask);
        images.push(image_data);
    }

    let ico_data = build_ico_file(&images, &sizes);
    fs::write(&ico_path, &ico_data).expect("无法写入图标文件");

    if cfg!(target_os = "windows") {
        let mut res = winres::WindowsResource::new();
        res.set_icon(ico_path.to_str().unwrap());
        res.compile().expect("无法编译 Windows 资源");
    }
}

// 主题色彩配置 - 与 gpui-component 暗色主题匹配
const COL_BG: [u8; 4] = [0x1A, 0x1A, 0x2E, 0xFF]; // 深蓝黑背景 (#1A1A2E)
const COL_BG_LIGHT: [u8; 4] = [0x22, 0x22, 0x3A, 0xFF]; // 稍浅的内部背景 (#22223A)
const COL_PRIMARY: [u8; 4] = [0x3B, 0x82, 0xF6, 0xFF]; // 主蓝色 (#3B82F6)
const COL_PRIMARY_LIGHT: [u8; 4] = [0x60, 0xA5, 0xFA, 0xFF]; // 浅主蓝 (#60A5FA)
const COL_FG: [u8; 4] = [0xFF, 0xFF, 0xFF, 0xFF]; // 纯白前景
const COL_ACCENT: [u8; 4] = [0x81, 0x8C, 0xF8, 0xFF]; // 紫蓝装饰 (#818CF8)

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

fn fill_circle(pixels: &mut [u8], w: u32, cx: i32, cy: i32, r: i32, c: [u8; 4]) {
    for dy in -r..=r {
        for dx in -r..=r {
            if dx * dx + dy * dy <= r * r {
                let x = cx + dx;
                let y = cy + dy;
                if x >= 0 && x < w as i32 && y >= 0 && y < w as i32 {
                    let i = ((y as u32 * w + x as u32) * 4) as usize;
                    pixels[i..i + 4].copy_from_slice(&c);
                }
            }
        }
    }
}

/// 绘制带倾斜效果的矩形（斜体效果）
/// slant_top: 顶部 x 偏移量，slant_bottom: 底部 x 偏移量
fn fill_slanted_rect(
    pixels: &mut [u8],
    w: u32,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    slant_top: i32,
    slant_bottom: i32,
    c: [u8; 4],
) {
    let height = y2 - y1;
    if height <= 0 {
        return;
    }
    for y in y1.max(0)..=y2.min(w as i32 - 1) {
        // 线性插值计算当前行的 x 偏移
        let t = (y - y1) as f32 / height as f32;
        let offset = slant_top as f32 + t * (slant_bottom - slant_top) as f32;
        let ox = offset.round() as i32;
        let row_x1 = (x1 + ox).max(0);
        let row_x2 = (x2 + ox).min(w as i32 - 1);
        for x in row_x1..=row_x2 {
            let i = ((y as u32 * w + x as u32) * 4) as usize;
            pixels[i..i + 4].copy_from_slice(&c);
        }
    }
}

fn generate_icon(size: u32) -> Vec<u8> {
    let s = size as i32;
    let scale = s as f32 / 32.0;
    let mut px = vec![0u8; (size * size * 4) as usize];

    // 1. 深色背景（圆角矩形）
    fill_rounded_rect(
        &mut px,
        size,
        0,
        0,
        s - 1,
        s - 1,
        (5.0 * scale) as i32,
        COL_BG,
    );

    // 2. 内部浅色背景（圆角矩形，留出 1px 边框）
    fill_rounded_rect(
        &mut px,
        size,
        1,
        1,
        s - 2,
        s - 2,
        (4.0 * scale) as i32,
        COL_BG_LIGHT,
    );

    // 3. 字母 "F" - 粗体无衬线斜体风格
    let f_left = (7.0 * scale) as i32;
    let f_top = (5.0 * scale) as i32;
    let f_right = (25.0 * scale) as i32;
    let f_bottom = (27.0 * scale) as i32;
    let stroke = (4.0 * scale) as i32;
    let mid_y = (15.0 * scale) as i32;
    let mid_right = (21.0 * scale) as i32;
    // 斜体倾斜量：顶部向右偏移，底部向左偏移
    let slant_top = (2.0 * scale) as i32;
    let slant_bottom = (-2.0 * scale) as i32;

    // 垂直主笔（斜体）
    fill_slanted_rect(
        &mut px,
        size,
        f_left,
        f_top,
        f_left + stroke - 1,
        f_bottom,
        slant_top,
        slant_bottom,
        COL_FG,
    );
    // 顶部横笔（斜体）
    fill_slanted_rect(
        &mut px,
        size,
        f_left,
        f_top,
        f_right,
        f_top + stroke - 1,
        slant_top,
        slant_top,
        COL_FG,
    );
    // 中间横笔（稍短，斜体）
    fill_slanted_rect(
        &mut px,
        size,
        f_left,
        mid_y,
        mid_right,
        mid_y + stroke - 1,
        slant_bottom,
        slant_bottom,
        COL_FG,
    );

    // 4. 底部装饰圆点（表示多个配置实例）
    let dot_y = (28.0 * scale) as i32;
    let dot_r = (1.5 * scale) as i32;
    fill_circle(
        &mut px,
        size,
        (10.0 * scale) as i32,
        dot_y,
        dot_r,
        COL_PRIMARY,
    );
    fill_circle(
        &mut px,
        size,
        (16.0 * scale) as i32,
        dot_y,
        dot_r,
        COL_PRIMARY_LIGHT,
    );
    fill_circle(
        &mut px,
        size,
        (22.0 * scale) as i32,
        dot_y,
        dot_r,
        COL_ACCENT,
    );

    // 5. 顶部装饰线
    let line_y = (3.0 * scale) as i32;
    fill_rect(
        &mut px,
        size,
        (8.0 * scale) as i32,
        line_y,
        (24.0 * scale) as i32,
        line_y,
        COL_PRIMARY,
    );

    px
}

fn build_ico_image(size: u32, pixels: &[u8], mask: &[u8]) -> Vec<u8> {
    let mut data = Vec::new();
    let row_bytes = size as usize * 4;

    // BITMAPINFOHEADER
    data.extend_from_slice(&40u32.to_le_bytes()); // biSize
    data.extend_from_slice(&size.to_le_bytes()); // biWidth
    data.extend_from_slice(&(size * 2).to_le_bytes()); // biHeight (doubled for ICO)
    data.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
    data.extend_from_slice(&32u16.to_le_bytes()); // biBitCount
    data.extend_from_slice(&0u32.to_le_bytes()); // biCompression
    data.extend_from_slice(&(pixels.len() as u32 + mask.len() as u32).to_le_bytes()); // biSizeImage
    data.extend_from_slice(&[0u8; 16]); // rest of header

    // 像素数据（自底向上）
    for y in (0..size as usize).rev() {
        let start = y * row_bytes;
        data.extend_from_slice(&pixels[start..start + row_bytes]);
    }
    // 掩码
    data.extend_from_slice(mask);
    data
}

fn build_ico_file(images: &[Vec<u8>], sizes: &[u32]) -> Vec<u8> {
    let mut ico = Vec::new();
    let count = images.len() as u16;

    // ICONDIR header
    ico.extend_from_slice(&0u16.to_le_bytes()); // reserved
    ico.extend_from_slice(&1u16.to_le_bytes()); // type: icon
    ico.extend_from_slice(&count.to_le_bytes()); // image count

    // 计算数据偏移量
    let header_size = 6u32;
    let entry_size = 16u32;
    let data_offset = header_size + count as u32 * entry_size;

    let mut current_offset = data_offset;
    for (i, (image, &size)) in images.iter().zip(sizes.iter()).enumerate() {
        // ICONDIRENTRY
        ico.push(if size >= 256 { 0 } else { size as u8 }); // width
        ico.push(if size >= 256 { 0 } else { size as u8 }); // height
        ico.push(0); // color palette
        ico.push(0); // reserved
        ico.extend_from_slice(&1u16.to_le_bytes()); // color planes
        ico.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        ico.extend_from_slice(&(image.len() as u32).to_le_bytes()); // data size
        ico.extend_from_slice(&current_offset.to_le_bytes()); // data offset
        current_offset += image.len() as u32;

        // 验证偏移量计算
        let _ = i;
    }

    // 写入图像数据
    for image in images {
        ico.extend_from_slice(image);
    }

    ico
}
