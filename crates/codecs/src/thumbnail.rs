//! Embedded EXIF thumbnails for fast previews.
//!
//! Phone JPEGs (and many TIFFs) carry a small JPEG preview in IFD1
//! (Compression = 6, tags 513/514), usually 160x120 or 320x240: decoding it
//! instead of the 12 MP main image is what keeps the file browser's grid
//! smooth. Anything missing, malformed or oversized gives `None` and the
//! caller falls back to the full decode, never an error.
//!
//! Offsets are relative to the TIFF header and every slice is bounds-checked
//! (`get`, `checked_add`); absurd thumbnail lengths are capped before the
//! decoder's own budgets see them. Only classic TIFF layouts are read: camera
//! EXIF is always classic, and anything else takes the fallback.

use crate::codecs::jpeg::scan_metadata;
use crate::image::Image;
use crate::options::{DecodeOptions, Limits};
use crate::orientation::{Order, exif_orientation, ifd0_entries, tiff_body};
use crate::{Format, decode_as_with};

/// EXIF tags of the embedded preview.
const TAG_COMPRESSION: u16 = 259;
const TAG_JPEG_OFFSET: u16 = 513;
const TAG_JPEG_LENGTH: u16 = 514;
/// The only preview layout this reads: a JPEG blob (what phones write).
/// Anything else (uncompressed strips, odd types) takes the full decode.
const COMPRESSION_JPEG: u32 = 6;
/// Biggest embedded preview this touches: real ones are tens of KB, so this
/// only stops hostile lengths before the decoder budgets kick in.
const MAX_THUMB_BYTES: usize = 8 * 1024 * 1024;

/// Decode budget for the preview itself: no real thumbnail is anywhere near it.
fn thumb_limits() -> Limits {
    Limits { max_width: 4096, max_height: 4096, max_pixels: 16 * 1024 * 1024, max_alloc: 64 * 1024 * 1024 }
}

/// The embedded JPEG preview of `bytes` as a decoded image: a JPEG's APP1
/// EXIF segment, or a whole TIFF file. `None` for anything else (PNGs, raws,
/// files without a preview): the caller decodes the main image instead.
///
/// The stored thumbnail pixels share the main image's orientation, so they
/// are turned upright by the main EXIF flag, like every photo viewer does.
pub fn exif_thumbnail(bytes: &[u8]) -> Option<Image> {
    if bytes.starts_with(&[0xFF, 0xD8]) {
        let exif = scan_metadata(bytes).exif?;
        let body = tiff_body(&exif);
        thumb_from_body(body, exif_orientation(&exif))
    } else if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
        thumb_from_body(bytes, exif_orientation(bytes))
    } else {
        None
    }
}

fn thumb_from_body(body: &[u8], main_orientation: u16) -> Option<Image> {
    let preview = jpeg_preview(body)?;
    // As-stored pixels: a thumbnail rarely carries its own EXIF, and when it
    // does it mirrors the main flag, so the main flag alone orients it.
    let opts = DecodeOptions { limits: thumb_limits(), keep_orientation: true };
    let img = decode_as_with(Format::Jpeg, preview, &opts).ok()?;
    img.oriented(main_orientation).ok()
}

/// IFD1's entry table: byte order and (first entry, count), validated exactly
/// like IFD0 is. Classic TIFF only, and the table must start past the 8-byte
/// header (offset 0 means "no IFD1").
fn ifd1_entries(body: &[u8]) -> Option<(Order, usize, usize)> {
    let (order, first, count, entry_bytes, big_tiff) = ifd0_entries(body)?;
    if big_tiff {
        return None;
    }
    let next_at = first.checked_add(count.checked_mul(entry_bytes)?)?;
    let at = usize::try_from(order.u32(body, next_at)?).ok()?;
    if at < 8 {
        return None;
    }
    let count = usize::from(order.u16(body, at)?);
    let first = at.checked_add(2)?;
    let end = first.checked_add(count.checked_mul(12)?)?.checked_add(4)?;
    if end > body.len() {
        return None;
    }
    Some((order, first, count))
}

/// The value of a SHORT/LONG tag with count 1 in IFD1 (it sits inline).
fn tag_u32(order: Order, body: &[u8], first: usize, count: usize, tag: u16) -> Option<u32> {
    let e = (0..count).filter_map(|i| first.checked_add(i.checked_mul(12)?)).find(|&e| order.u16(body, e) == Some(tag))?;
    if u64::from(order.u32(body, e.checked_add(4)?)?) != 1 {
        return None;
    }
    match order.u16(body, e.checked_add(2)?)? {
        3 => order.u16(body, e.checked_add(8)?).map(u32::from),
        4 => order.u32(body, e.checked_add(8)?),
        _ => None,
    }
}

/// The embedded JPEG preview bytes, sliced out of the TIFF body.
fn jpeg_preview(body: &[u8]) -> Option<&[u8]> {
    let (order, first, count) = ifd1_entries(body)?;
    if tag_u32(order, body, first, count, TAG_COMPRESSION)? != COMPRESSION_JPEG {
        return None;
    }
    let at = usize::try_from(tag_u32(order, body, first, count, TAG_JPEG_OFFSET)?).ok()?;
    let len = usize::try_from(tag_u32(order, body, first, count, TAG_JPEG_LENGTH)?).ok()?;
    if len == 0 || len > MAX_THUMB_BYTES {
        return None;
    }
    let thumb = body.get(at..at.checked_add(len)?)?;
    // Offsets pointing at IFD data are not a preview: don't feed them to the
    // decoder when they don't even look like a JPEG.
    thumb.starts_with(&[0xFF, 0xD8]).then_some(thumb)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn le16(v: u16) -> [u8; 2] {
        v.to_le_bytes()
    }

    fn le32(v: u32) -> [u8; 4] {
        v.to_le_bytes()
    }

    /// One 12-byte IFD entry: tag, type, count, inline value/offset.
    fn entry(tag: u16, ty: u16, count: u32, value: u32) -> Vec<u8> {
        [le16(tag).as_slice(), le16(ty).as_slice(), le32(count).as_slice(), le32(value).as_slice()].concat()
    }

    /// A little-endian TIFF body whose IFD1 holds `thumb_jpeg` as a
    /// Compression = 6 preview; IFD0 carries `orientation` (0 = no entry).
    fn tiff_with_thumb(thumb_jpeg: &[u8], orientation: u16) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(b"II*\0");
        b.extend_from_slice(&le32(8));
        // IFD0.
        let ifd0: Vec<u8> = if orientation == 0 { Vec::new() } else { entry(274, 3, 1, u32::from(orientation)).to_vec() };
        let ifd1_at = 8 + 2 + ifd0.len() + 4;
        b.extend_from_slice(&le16(u16::try_from(ifd0.len() / 12).unwrap()));
        b.extend_from_slice(&ifd0);
        b.extend_from_slice(&le32(u32::try_from(ifd1_at).unwrap()));
        // IFD1.
        let thumb_at = ifd1_at + 2 + 3 * 12 + 4;
        b.extend_from_slice(&le16(3));
        b.extend_from_slice(&entry(TAG_COMPRESSION, 3, 1, COMPRESSION_JPEG));
        b.extend_from_slice(&entry(TAG_JPEG_OFFSET, 4, 1, u32::try_from(thumb_at).unwrap()));
        b.extend_from_slice(&entry(TAG_JPEG_LENGTH, 4, 1, u32::try_from(thumb_jpeg.len()).unwrap()));
        b.extend_from_slice(&le32(0));
        b.extend_from_slice(thumb_jpeg);
        b
    }

    /// A minimal JPEG: SOI, one APP1 EXIF segment, EOI. There are no main
    /// image pixels at all, so a test passing on this proves the main image
    /// is never decoded.
    fn jpeg_with_exif(tiff: &[u8]) -> Vec<u8> {
        let mut b = vec![0xFF, 0xD8, 0xFF, 0xE1];
        let len = u16::try_from(2 + 6 + tiff.len()).unwrap();
        b.extend_from_slice(&len.to_be_bytes());
        b.extend_from_slice(b"Exif\0\0");
        b.extend_from_slice(tiff);
        b.extend_from_slice(&[0xFF, 0xD9]);
        b
    }

    /// A small real JPEG payload (lossy: assert colours loosely).
    fn red_jpeg(w: u32, h: u32) -> Vec<u8> {
        let px = [255u8, 0, 0, 255].repeat(usize::try_from(w * h).unwrap());
        let img = Image::from_u8(w, h, crate::ChannelLayout::Rgba, px).unwrap();
        crate::encode(&img, Format::Jpeg, &crate::EncodeOptions::default()).unwrap()
    }

    fn centre_px(img: &Image) -> [u8; 4] {
        let px = img.to_rgba8();
        let (w, h) = (img.width() as usize, img.height() as usize);
        let o = (h / 2 * w + w / 2) * 4;
        px[o..o + 4].try_into().unwrap()
    }

    #[test]
    fn jpeg_preview_decodes_without_main_pixels() {
        let thumb = red_jpeg(64, 32);
        let file = jpeg_with_exif(&tiff_with_thumb(&thumb, 0));
        let img = exif_thumbnail(&file).expect("IFD1 preview decodes");
        assert_eq!((img.width(), img.height()), (64, 32));
        let [r, g, bl, _] = centre_px(&img);
        assert!(r > 200 && g < 100 && bl < 100, "still red: {r},{g},{bl}");
    }

    #[test]
    fn main_orientation_turns_the_preview_upright() {
        let thumb = red_jpeg(64, 32);
        let file = jpeg_with_exif(&tiff_with_thumb(&thumb, 6));
        let img = exif_thumbnail(&file).expect("oriented preview decodes");
        assert_eq!((img.width(), img.height()), (32, 64), "orientation 6 swaps the sides");
    }

    #[test]
    fn tiff_file_preview_decodes() {
        let thumb = red_jpeg(32, 32);
        let img = exif_thumbnail(&tiff_with_thumb(&thumb, 0)).expect("TIFF IFD1 preview decodes");
        assert_eq!((img.width(), img.height()), (32, 32));
    }

    #[test]
    fn files_without_a_preview_give_none() {
        assert!(exif_thumbnail(&[0xFF, 0xD8, 0xFF, 0xD9]).is_none(), "no APP1");
        assert!(exif_thumbnail(b"not an image at all").is_none(), "not an image");
        let thumb = red_jpeg(16, 16);
        // EXIF with an IFD0 but no IFD1.
        let mut tiff = vec![];
        tiff.extend_from_slice(b"II*\0");
        tiff.extend_from_slice(&le32(8));
        tiff.extend_from_slice(&le16(0));
        tiff.extend_from_slice(&le32(0));
        assert!(exif_thumbnail(&jpeg_with_exif(&tiff)).is_none(), "no IFD1");
        // Uncompressed thumbnails are not previews this reads.
        let mut t = tiff_with_thumb(&thumb, 0);
        let compression_at = 8 + 2 + 4 + 2;
        t[compression_at..compression_at + 12].copy_from_slice(&entry(TAG_COMPRESSION, 3, 1, 1));
        assert!(exif_thumbnail(&jpeg_with_exif(&t)).is_none(), "Compression 1 takes the fallback");
    }

    #[test]
    fn hostile_offsets_and_lengths_give_none() {
        let thumb = red_jpeg(16, 16);
        let good = tiff_with_thumb(&thumb, 0);
        // Entry layout: header 8, IFD0 (count 0) 6 bytes, IFD1 at 14:
        // count 2, entries at 16/28/40, next at 52, data at 56.
        let mut oob = good.clone();
        oob[36..40].copy_from_slice(&le32(60000));
        assert!(exif_thumbnail(&jpeg_with_exif(&oob)).is_none(), "offset past the end");
        let mut huge = good.clone();
        huge[48..52].copy_from_slice(&le32(u32::MAX));
        assert!(exif_thumbnail(&jpeg_with_exif(&huge)).is_none(), "absurd length");
        let mut zero = good.clone();
        zero[48..52].copy_from_slice(&le32(0));
        assert!(exif_thumbnail(&jpeg_with_exif(&zero)).is_none(), "empty preview");
        assert!(exif_thumbnail(&jpeg_with_exif(&good[..good.len() / 2])).is_none(), "truncated EXIF");
    }
}
