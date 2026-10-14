//! Contact sheets, burned-in labels and PNG encoding.

use crate::common::{BACKGROUND, fitted, lavapipe, luma, upload};
use crate::scene;
use gs_render::matcap::MatCapImage;
use gs_render::{Image, Preset, RenderSize, Shading, View, Wire};

/// Label layout: 5x7 glyphs drawn 2 pixels per dot, 1 dot of padding, 6 dots per character.
const SCALE: u32 = 2;
const PAD: u32 = 1;
const ADVANCE: u32 = 6;
const BOX_HEIGHT: u32 = (7 + 2 * PAD) * SCALE;

fn box_width(label: &str) -> u32 {
    (2 * PAD + label.len() as u32 * ADVANCE) * SCALE
}

#[test]
fn contact_sheet_is_the_snapshots_tiled_and_labelled() {
    let r = lavapipe();
    let mut sphere = scene::quad_sphere(8);
    scene::dent(&mut sphere, 0.12);
    let mesh = upload(r, &sphere);
    let names = ["L~T", "top", "iso view"];
    let views: Vec<(&str, View)> = names
        .iter()
        .zip([Preset::Front, Preset::Top, Preset::Iso])
        .map(|(&n, p)| (n, fitted(&mesh, p, Shading::Clay, Wire::Off)))
        .collect();
    let tile = RenderSize {
        width: 96,
        height: 80,
        samples: 1,
    };
    let sheet = r
        .contact_sheet(&mesh, &views, tile, 2)
        .expect("sheet renders");
    assert_eq!(
        (sheet.width, sheet.height),
        (192, 160),
        "3 tiles in 2 columns"
    );
    for (i, (name, view)) in views.iter().enumerate() {
        let shot = r.snapshot(&mesh, view, tile).expect("renders");
        let (x0, y0) = ((i as u32 % 2) * 96, (i as u32 / 2) * 80);
        for (x, y) in (0..80).flat_map(|y| (0..96).map(move |x| (x, y))) {
            let in_label = x < box_width(name) && y < BOX_HEIGHT;
            let same = sheet.pixel(x0 + x, y0 + y) == shot.pixel(x, y);
            assert!(
                in_label || same,
                "tile {i} pixel ({x}, {y}) is the snapshot's"
            );
            if in_label {
                assert!(
                    luma(sheet.pixel(x0 + x, y0 + y)) < 60
                        || luma(sheet.pixel(x0 + x, y0 + y)) > 600,
                    "label pixels are box or ink"
                );
            }
        }
    }
    assert!(
        (96..192).all(|x| (80..160).all(|y| sheet.pixel(x, y) == BACKGROUND)),
        "the unused tile is background"
    );
    assert_glyphs(&sheet);
    let one_column = r.contact_sheet(&mesh, &views, tile, 0).expect("renders");
    let too_many = r.contact_sheet(&mesh, &views, tile, 9).expect("renders");
    assert_eq!(
        [
            (one_column.width, one_column.height),
            (too_many.width, too_many.height)
        ],
        [(96, 240), (288, 80)],
        "columns clamp to 1..views"
    );
}

/// The first label reads "L~T": L is its left column and bottom row, `~` is not in the font
/// so it shows the question mark, and T is its top row and middle column.
fn assert_glyphs(sheet: &Image) {
    let l = |col: u32, row: u32| col == 0 || row == 6;
    let question: [u32; 7] = [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0, 0b00100];
    let unknown = |col: u32, row: u32| question[row as usize] >> (4 - col) & 1 == 1;
    let t = |col: u32, row: u32| row == 0 || col == 2;
    for (glyph, inked) in [&l as &dyn Fn(u32, u32) -> bool, &unknown, &t]
        .into_iter()
        .enumerate()
    {
        for (col, row) in (0..7).flat_map(|row| (0..5).map(move |col| (col, row))) {
            let x = (PAD + glyph as u32 * ADVANCE + col) * SCALE;
            let y = (PAD + row) * SCALE;
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let lit = luma(sheet.pixel(x + dx, y + dy)) > 600;
                assert_eq!(
                    lit,
                    inked(col, row),
                    "glyph {glyph} dot ({col}, {row}) pixel ({}, {})",
                    x + dx,
                    y + dy
                );
            }
        }
    }
}

fn decode(png_bytes: &[u8]) -> (png::OutputInfo, Vec<u8>, bool) {
    let mut reader = png::Decoder::new(std::io::Cursor::new(png_bytes))
        .read_info()
        .expect("valid png");
    let srgb = reader.info().srgb.is_some();
    let mut data = vec![0; reader.output_buffer_size().expect("size known")];
    let info = reader.next_frame(&mut data).expect("one frame");
    data.truncate(info.buffer_size());
    (info, data, srgb)
}

#[test]
fn png_round_trips() {
    let image = Image {
        width: 3,
        height: 2,
        rgba: (0u8..24).map(|v| v * 10).collect(),
    };
    let (info, data, srgb) = decode(&image.png().expect("encodes"));
    assert_eq!(
        (info.width, info.height, info.color_type, info.bit_depth),
        (3, 2, png::ColorType::Rgba, png::BitDepth::Eight),
        "8-bit RGBA"
    );
    assert!(srgb && data == image.rgba, "same pixels, tagged sRGB");
    assert_eq!(
        Image::from_png(&image.png().expect("encodes")),
        Ok(image.clone()),
        "and decodes back"
    );
    assert!(
        Image::from_png(b"not a png").is_err(),
        "garbage does not decode"
    );
    let matcap = MatCapImage {
        size: 2,
        pixels: vec![
            [1, 300, 65535, 7],
            [256, 0, 1000, 65535],
            [5, 6, 7, 8],
            [40000, 1, 2, 3],
        ],
    };
    let (info, data, srgb) = decode(&matcap.png().expect("encodes"));
    assert_eq!(
        (info.width, info.bit_depth),
        (2, png::BitDepth::Sixteen),
        "16-bit samples"
    );
    let samples: Vec<u16> = data
        .chunks(2)
        .map(|b| u16::from_be_bytes([b[0], b[1]]))
        .collect();
    assert!(
        srgb && samples == matcap.pixels.concat(),
        "same 16-bit samples, tagged sRGB"
    );
    assert!(
        Image::from_png(&matcap.png().expect("encodes")).is_err(),
        "16-bit images are not frames"
    );
}

/// Images of different sizes tile into cells as large as the largest, padded with background.
#[test]
fn sheet_tiles_images_of_any_size() {
    let red = Image::filled(40, 20, [200, 0, 0, 255]);
    let blue = Image::filled(30, 30, [0, 0, 200, 255]);
    let sheet = Image::sheet(&[("", &red), ("", &blue), ("", &red)], 2);
    assert_eq!(
        (sheet.width, sheet.height),
        (80, 60),
        "2 x 2 cells of 40 x 30"
    );
    let at = |x, y| sheet.pixel(x, y);
    assert_eq!(at(39, 19), [200, 0, 0, 255], "the first image in its cell");
    assert_eq!(at(39, 25), BACKGROUND, "padding below a short image");
    assert_eq!(at(40 + 29, 29), [0, 0, 200, 255], "the second image");
    assert_eq!(at(40 + 35, 5), BACKGROUND, "padding beside a narrow image");
    assert_eq!(
        at(10, 30 + 10),
        [200, 0, 0, 255],
        "the third starts the next row"
    );
    assert_eq!(at(60, 45), BACKGROUND, "the empty cell");
    let labelled = Image::sheet(&[("A", &blue)], 5);
    assert_eq!(
        (labelled.width, labelled.height),
        (30, 30),
        "columns clamp to the image count"
    );
    assert!(
        luma(labelled.pixel(PAD * SCALE, (PAD + 1) * SCALE)) > 600,
        "the label is burned in"
    );
    let empty = Image::sheet(&[], 3);
    assert_eq!((empty.width, empty.height), (0, 0), "nothing to tile");
}
