//! 图片工具：从文件检测图片尺寸和类型。

use std::io::Read;

use crate::error::AppError;

/// 检测图片文件的宽度、高度和类型。
///
/// 支持 JPEG、PNG、GIF、BMP、WebP。
/// 返回 `(width, height, image_type)`，未知格式返回 `(0, 0, "")`。
pub fn detect_image_dims(path: &std::path::Path) -> Result<(i64, i64, String), AppError> {
    let mut file = std::fs::File::open(path)
        .map_err(|e| AppError::Internal(format!("open file for image detect: {}", e)))?;

    // 读取足够所有常见图片头部的数据（JPEG SOF 可能在 EXIF 之后更深的位置）
    let mut buf = vec![0u8; 4096];
    let n = file
        .read(&mut buf)
        .map_err(|e| AppError::Internal(format!("read file for image detect: {}", e)))?;
    let header = &buf[..n];

    // JPEG：以 FF D8 FF 开头
    if n >= 3 && header[0] == 0xFF && header[1] == 0xD8 && header[2] == 0xFF {
        let mut pos = 2;
        while pos + 9 < n {
            if header[pos] == 0xFF && matches!(header[pos + 1], 0xC0..=0xC2) {
                let height = u16::from_be_bytes([header[pos + 5], header[pos + 6]]);
                let width = u16::from_be_bytes([header[pos + 7], header[pos + 8]]);
                return Ok((width as i64, height as i64, "jpeg".into()));
            }
            pos += 1;
        }
        return Ok((0, 0, "jpeg".into())); // 已知为 jpeg 但未在头部窗口中找到尺寸
    }

    // PNG：魔数 89 50 4E 47 0D 0A 1A 0A，IHDR 在偏移 16 处
    if n >= 24 && header[0] == 0x89 && header[1] == b'P' && header[2] == b'N' && header[3] == b'G' {
        let width = u32::from_be_bytes([header[16], header[17], header[18], header[19]]);
        let height = u32::from_be_bytes([header[20], header[21], header[22], header[23]]);
        return Ok((width as i64, height as i64, "png".into()));
    }

    // GIF：魔数 "GIF87a" 或 "GIF89a"，尺寸在偏移 6 处（小端）
    if n >= 10
        && (header[0] == b'G'
            && header[1] == b'I'
            && header[2] == b'F'
            && (header[3] == b'8' && (header[4] == b'7' || header[4] == b'9') && header[5] == b'a'))
    {
        let width = u16::from_le_bytes([header[6], header[7]]);
        let height = u16::from_le_bytes([header[8], header[9]]);
        return Ok((width as i64, height as i64, "gif".into()));
    }

    // BMP：魔数 "BM"，尺寸在偏移 18 处（小端）
    if n >= 26 && header[0] == b'B' && header[1] == b'M' {
        let width = u32::from_le_bytes([header[18], header[19], header[20], header[21]]);
        let height = u32::from_le_bytes([header[22], header[23], header[24], header[25]]);
        return Ok((width as i64, height as i64, "bmp".into()));
    }

    // WebP：RIFF + 大小 + WEBP
    if n >= 30
        && header[0] == b'R'
        && header[1] == b'I'
        && header[2] == b'F'
        && header[3] == b'F'
        && header[8] == b'W'
        && header[9] == b'E'
        && header[10] == b'B'
        && header[11] == b'P'
    {
        let fourcc = &header[12..16];
        if fourcc == b"VP8 " && n >= 30 {
            // VP8 关键帧：宽/高在偏移 26 处（小端，16 像素对齐）
            let raw = u16::from_le_bytes([header[26], header[27]]);
            let width = (raw & 0x3FFF) as u32;
            let raw = u16::from_le_bytes([header[28], header[29]]);
            let height = (raw & 0x3FFF) as u32;
            if width > 0 && height > 0 {
                return Ok((width as i64, height as i64, "webp".into()));
            }
        } else if fourcc == b"VP8L" && n >= 25 {
            // VP8L 无损：宽/高打包在偏移 21 处的 4 个字节中
            let bits = u32::from_le_bytes([header[21], header[22], header[23], header[24]]);
            let width = (bits & 0x3FFF) + 1;
            let height = ((bits >> 14) & 0x3FFF) + 1;
            if width > 0 && height > 0 {
                return Ok((width as i64, height as i64, "webp".into()));
            }
        } else if fourcc == b"VP8X" && n >= 30 {
            // VP8X 扩展：3 字节宽（小端），3 字节高，在偏移 24 处
            let width = u24_le(&header[24..27]);
            let height = u24_le(&header[27..30]);
            if width > 0 && height > 0 {
                return Ok((width as i64, height as i64, "webp".into()));
            }
        }
        return Ok((0, 0, "webp".into()));
    }

    // 未知格式——返回默认值
    Ok((0, 0, String::new()))
}

/// 从字节切片解析 24 位小端值（必须至少 3 字节）。
fn u24_le(bytes: &[u8]) -> u32 {
    bytes[0] as u32 | (bytes[1] as u32) << 8 | (bytes[2] as u32) << 16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_png() {
        // minimal 1x1 red PNG
        let png = hex::decode("89504E470D0A1A0A0000000D49484452000000010000000108060000001F15C4890000000467414D410000B18F0BFC6105000000097048597300000EC300000EC301C76FA8640000000B4944415408D76360A80F0000030001B6B3D50D0000000049454E44AE426082").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.png");
        std::fs::write(&path, &png).unwrap();
        let (w, h, t) = detect_image_dims(&path).unwrap();
        assert_eq!(w, 1);
        assert_eq!(h, 1);
        assert_eq!(t, "png");
    }

    #[test]
    fn test_detect_jpeg() {
        // minimal 1x1 white JPEG
        let jpeg = hex::decode("FFD8FFE000104A46494600010101004800480000FFDB004300080606070605080707070909080A0C140D0C0B0B0C1912130F141D1A1F1E1D1A1C1C20242E2720222C231C1C2837292C30313434341F27393D38323C2E333432FFC0000B080001000101011100FFC4001F0000010501010101010100000000000000000102030405060708090A0BFFC400B5100002010303020403050504040000017D01020300041105122131410613516107227114328191A1082342B1C11552D1F02433627282090A161718191A25262728292A3435363738393A434445464748494A535455565758595A636465666768696A737475767778797A838485868788898A92939495969798999AA2A3A4A5A6A7A8A9AAB2B3B4B5B6B7B8B9BAC2C3C4C5C6C7C8C9CAD2D3D4D5D6D7D8D9DAE1E2E3E4E5E6E7E8E9EAF1F2F3F4F5F6F7F8F9FAFFC4001F0100030101010101010101010000000000000102030405060708090A0BFFC400B51100020102040403040705040400010277000102031104052131061241510761711322328108144291A1B1C109233352F0156272D10A162434E125F11718191A262728292A35363738393A434445464748494A535455565758595A636465666768696A737475767778797A82838485868788898A92939495969798999AA2A3A4A5A6A7A8A9AAB2B3B4B5B6B7B8B9BAC2C3C4C5C6C7C8C9CAD2D3D4D5D6D7D8D9DAE2E3E4E5E6E7E8E9EAF2F3F4F5F6F7F8F9FAFFDA000C03010002110311003F00F8A2800A28A0028A2800A28A0028A2800A28A0028A2800A28A0028A2800A28A0028A2800A28A0028A2800A28A0028A2800A28A0028A2800A28A0028A28A00FFFD9").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.jpg");
        std::fs::write(&path, &jpeg).unwrap();
        let (w, h, t) = detect_image_dims(&path).unwrap();
        assert_eq!(w, 1);
        assert_eq!(h, 1);
        assert_eq!(t, "jpeg");
    }

    #[test]
    fn test_non_image() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, b"hello world").unwrap();
        let (w, h, t) = detect_image_dims(&path).unwrap();
        assert_eq!(w, 0);
        assert_eq!(h, 0);
        assert!(t.is_empty());
    }
}
