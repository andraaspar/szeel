//! Golden-image tests: every `examples/*.kdl` is rendered and compared with
//! `tests/golden/<name>.png`. Set `UPDATE_GOLDEN=1` to (re)write the references.

use std::fs;
use std::path::Path;

fn render(src: &str) -> (u32, u32, Vec<u8>) {
    let scene = szeel::parse::parse(src)
        .unwrap()
        .scene
        .expect("example has errors");
    let canvas = szeel::render::render(&scene);
    let mut png = Vec::new();
    canvas.write_png(&mut png).unwrap();
    decode(&png)
}

fn decode(png: &[u8]) -> (u32, u32, Vec<u8>) {
    let mut reader = png::Decoder::new(std::io::Cursor::new(png))
        .read_info()
        .unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    buf.truncate(info.buffer_size());
    (info.width, info.height, buf)
}

#[test]
fn examples_match_golden_images() {
    let update = std::env::var_os("UPDATE_GOLDEN").is_some();
    let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    let mut entries: Vec<_> = fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("examples"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "kdl"))
        .collect();
    entries.sort();
    assert!(!entries.is_empty());
    for path in entries {
        let name = path.file_stem().unwrap().to_str().unwrap();
        let (w, h, px) = render(&fs::read_to_string(&path).unwrap());
        let reference = golden.join(format!("{name}.png"));
        if update {
            let scene = szeel::parse::parse(&fs::read_to_string(&path).unwrap())
                .unwrap()
                .scene
                .unwrap();
            let canvas = szeel::render::render(&scene);
            canvas
                .write_png(fs::File::create(&reference).unwrap())
                .unwrap();
            continue;
        }
        let (rw, rh, rpx) = decode(
            &fs::read(&reference).unwrap_or_else(|_| panic!("missing {}", reference.display())),
        );
        assert_eq!((w, h), (rw, rh), "{name}: size");
        // Allow ±1 per channel for floating-point differences between platforms.
        let bad = px
            .iter()
            .zip(&rpx)
            .filter(|(a, b)| a.abs_diff(**b) > 1)
            .count();
        assert_eq!(
            bad, 0,
            "{name}: {bad} channel values differ from the golden image"
        );
    }
}
