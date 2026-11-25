use std::path::Path;

pub struct Rgba {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

pub fn exe_icon(path: &Path) -> Option<Rgba> {
    use windows_sys::Win32::Graphics::Gdi::{BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC};
    use windows_sys::Win32::UI::Shell::ExtractIconExW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO};
    let wide: Vec<u16> = path.as_os_str().to_string_lossy().encode_utf16().chain(Some(0)).collect();
    unsafe {
        let mut icon: HICON = std::ptr::null_mut();
        if ExtractIconExW(wide.as_ptr(), 0, &mut icon, std::ptr::null_mut(), 1) == 0 || icon.is_null() {
            return None;
        }
        let mut info: ICONINFO = std::mem::zeroed();
        let ok = GetIconInfo(icon, &mut info) != 0;
        let result = (|| {
            if !ok || info.hbmColor.is_null() {
                return None;
            }
            let mut bm: BITMAP = std::mem::zeroed();
            if GetObjectW(info.hbmColor as _, std::mem::size_of::<BITMAP>() as i32, &mut bm as *mut _ as *mut _) == 0 {
                return None;
            }
            let (w, h) = (bm.bmWidth.max(0) as usize, bm.bmHeight.unsigned_abs() as usize);
            if w == 0 || h == 0 || w > 256 || h > 256 {
                return None;
            }
            let read = |bmp| -> Option<Vec<u8>> {
                let mut bi: BITMAPINFO = std::mem::zeroed();
                bi.bmiHeader = BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w as i32,
                    biHeight: -(h as i32),
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    ..std::mem::zeroed()
                };
                let mut buf = vec![0u8; w * h * 4];
                let dc = GetDC(std::ptr::null_mut());
                let lines = GetDIBits(dc, bmp, 0, h as u32, buf.as_mut_ptr() as *mut _, &mut bi, DIB_RGB_COLORS);
                ReleaseDC(std::ptr::null_mut(), dc);
                (lines > 0).then_some(buf)
            };
            let mut px = read(info.hbmColor)?;
            let has_alpha = px.chunks_exact(4).any(|p| p[3] != 0);
            let mask = if has_alpha || info.hbmMask.is_null() { None } else { read(info.hbmMask) };
            for (i, p) in px.chunks_exact_mut(4).enumerate() {
                p.swap(0, 2);
                if !has_alpha {
                    p[3] = match &mask {
                        Some(m) if m[i * 4] != 0 => 0,
                        _ => 255,
                    };
                }
            }
            Some(Rgba { width: w, height: h, pixels: px })
        })();
        if !info.hbmColor.is_null() {
            DeleteObject(info.hbmColor as _);
        }
        if !info.hbmMask.is_null() {
            DeleteObject(info.hbmMask as _);
        }
        DestroyIcon(icon);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_icon_of_system_program() {
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
        let notepad = Path::new(&windir).join("System32").join("notepad.exe");
        if !notepad.exists() {
            return;
        }
        let i = exe_icon(&notepad).unwrap();
        assert!(i.width >= 16 && i.height >= 16);
        assert_eq!(i.pixels.len(), i.width * i.height * 4);
        assert!(i.pixels.chunks_exact(4).any(|p| p[3] > 0));
        assert!(exe_icon(Path::new("Z:\\нет\\такого.exe")).is_none());
    }
}
