#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

mod drawing_tests {
    use super::*;

    #[test]
    fn the_mark_has_its_parts_where_the_favicon_has_them() {
        let size = 320; // ten pixels per unit of the 32-unit grid
        let img = render_rgba(size, 0.0);
        let at = |ux: f64, uy: f64| {
            let i = ((uy * 10.0) as usize * size as usize + (ux * 10.0) as usize) * 4;
            [img[i], img[i + 1], img[i + 2], img[i + 3]]
        };
        assert_eq!(at(16.0, 8.0), [0xff, 0x5a, 0x36, 255], "the beam");
        assert_eq!(at(16.0, 18.0), [0xff, 0xd1, 0x66, 255], "the spark");
        assert_eq!(at(16.0, 25.0), [0x73, 0xc6, 0xff, 255], "the cut line");
        assert_eq!(at(4.0, 4.0), [0x1c, 0x1f, 0x24, 255], "the tile");
        assert_eq!(at(0.2, 0.2)[3], 0, "the corner is cut away");
        assert_eq!(at(31.8, 31.8)[3], 0);
    }

    #[test]
    fn edges_are_smoothed_not_jagged() {
        let img = render_rgba(64, 0.0);
        let partial = img.as_chunks::<4>().0.iter().filter(|p| p[3] != 0 && p[3] != 255).count();
        assert!(partial > 20, "{partial} partly covered pixels");
    }

    #[test]
    fn the_inset_leaves_a_transparent_margin() {
        let size = 128usize;
        let img = render_rgba(size as u32, MAC_INSET);
        let alpha = |x: usize, y: usize| img[(y * size + x) * 4 + 3];
        for i in 0..size {
            assert_eq!(alpha(i, 2), 0, "top margin at {i}");
            assert_eq!(alpha(2, i), 0, "left margin at {i}");
        }
        assert_eq!(alpha(size / 2, size / 2 - 20 + 8), 255);
    }

    #[test]
    fn drawing_is_deterministic_and_every_size_works() {
        assert_eq!(render_rgba(48, 0.0), render_rgba(48, 0.0));
        for size in [1, 2, 3, 16, 17, 255, 256, 257] {
            assert_eq!(render_rgba(size, 0.0).len(), (size * size * 4) as usize);
        }
    }

    #[test]
    fn hostile_sizes_and_insets_are_clamped() {
        assert_eq!(render_rgba(0, 0.0).len(), 4);
        assert_eq!(render_rgba(5, f64::NAN).len(), 100);
        assert_eq!(render_rgba(5, 9.0).len(), 100);
        assert_eq!(render_rgba(5, -1.0).len(), 100);
        assert_eq!(png(u32::MAX, 0.0).len(), png(MAX_SIZE, 0.0).len());
    }
}

mod file_tests {
    use super::*;

    #[test]
    fn the_png_decodes_to_the_drawing() {
        let decoded = image::load_from_memory(&png(64, 0.0)).unwrap().to_rgba8();
        assert_eq!((decoded.width(), decoded.height()), (64, 64));
        assert_eq!(decoded.into_raw(), render_rgba(64, 0.0));
    }

    #[test]
    fn the_ico_is_well_formed_and_holds_every_size() {
        let ico = ico();
        assert_eq!(&ico[..4], &[0, 0, 1, 0], "reserved, type = icon");
        let count = u16::from_le_bytes([ico[4], ico[5]]) as usize;
        assert_eq!(count, ICO_SIZES.len());
        for (i, size) in ICO_SIZES.iter().enumerate() {
            let e = &ico[6 + i * 16..6 + (i + 1) * 16];
            let dim = if e[0] == 0 { 256 } else { u32::from(e[0]) };
            assert_eq!(dim, *size);
            assert_eq!(e[0], e[1], "square");
            let len = u32::from_le_bytes([e[8], e[9], e[10], e[11]]) as usize;
            let off = u32::from_le_bytes([e[12], e[13], e[14], e[15]]) as usize;
            let img = image::load_from_memory(ico.get(off..off + len).expect("entry inside the file")).unwrap();
            assert_eq!(img.width(), *size);
        }
        let last = &ico[6 + (count - 1) * 16..6 + count * 16];
        let end = u32::from_le_bytes([last[12], last[13], last[14], last[15]]) as usize
            + u32::from_le_bytes([last[8], last[9], last[10], last[11]]) as usize;
        assert_eq!(end, ico.len(), "no gap or trailing bytes");
    }

    #[test]
    fn the_icns_chunks_add_up_and_each_holds_a_png_of_its_size() {
        let icns = icns();
        assert_eq!(&icns[..4], b"icns");
        assert_eq!(u32::from_be_bytes([icns[4], icns[5], icns[6], icns[7]]) as usize, icns.len());
        let mut at = 8;
        let mut seen = Vec::new();
        while at < icns.len() {
            let tag: [u8; 4] = icns[at..at + 4].try_into().unwrap();
            let len = u32::from_be_bytes([icns[at + 4], icns[at + 5], icns[at + 6], icns[at + 7]]) as usize;
            let img = image::load_from_memory(&icns[at + 8..at + len]).unwrap();
            let want = ICNS_SIZES.iter().find(|(t, _)| **t == tag).map(|(_, s)| *s);
            assert_eq!(Some(img.width()), want, "{}", String::from_utf8_lossy(&tag));
            seen.push(tag);
            at += len;
        }
        assert_eq!(at, icns.len());
        assert_eq!(seen.len(), ICNS_SIZES.len());
    }
}
