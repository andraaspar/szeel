use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use szeel::parse::parse;
use szeel::render::render;

/// `n` stacked open paths with `points` points each, each with an implicit width.
fn many_paths(width: u32, height: u32, n: usize, points: usize) -> String {
    many_paths_eased(width, height, n, points, "linear")
}

/// [`many_paths`] with `easing` on every gradient and every stop.
fn many_paths_eased(width: u32, height: u32, n: usize, points: usize, easing: &str) -> String {
    let e = format!("easing=\"{easing}\"");
    let mut s = format!("image width={width} height={height}\n");
    for i in 0..n {
        s += "path {\n";
        for j in 0..points {
            let x = (j as f64 + 0.5) / points as f64 * width as f64;
            let y = height as f64 * (0.5 + 0.4 * ((j as f64 * 0.9 + i as f64 * 0.37).sin()));
            s += &format!("point {x:.2} {y:.2}\n");
        }
        let hue = (i * 37) % 360;
        s += &format!(
            "gradient 1.5 {e} {{ stop 0 0.7 0.15 {hue} {e}; stop 30 0.7 0.15 {hue} 0.5 {e}; stop 60 0.6 0.1 {hue} 0; }}\n"
        );
        s += &format!(
            "gradient 3.5 {e} {{ stop 0 0.5 0.1 {} {e}; stop 40 0.5 0.1 {} 0; }}\n}}\n",
            hue + 120,
            hue + 120
        );
    }
    // An opaque background path that fills everything else.
    s += "path closed=#true { point 100 100; point 900 200; point 500 700\n";
    s += "gradient 1 { stop 0 0.3 0.05 250; stop 500 0.1 0.02 280; }\n";
    s += "gradient 3 { stop 0 0.9 0.03 80; }\n}\n";
    s
}

/// `n` groups of three strokes, each group under a soft filled mask of its
/// own, in front of an opaque background.
fn masked_groups(width: u32, height: u32, n: usize) -> String {
    let mut s = format!("image width={width} height={height}\n");
    for i in 0..n {
        let (cx, cy) = ((i * 397) % width as usize, (i * 211) % height as usize);
        let r = 150 + (i * 37) % 100;
        s += "group {\n    mask { path closed=#true {\n";
        for (dx, dy) in [(0i64, -1i64), (1, 0), (0, 1), (-1, 0)] {
            s += &format!(
                "        point {} {}\n",
                cx as i64 + dx * r as i64,
                cy as i64 + dy * r as i64
            );
        }
        s += "        gradient 1 { stop 0 0 0 0 0.8; stop 60 0 0 0 0; }\n        gradient 3 { stop 0 0 0 0 0.8; }\n    } }\n";
        for k in 0..3 {
            let y = cy as i64 + (k as i64 - 1) * 60;
            let hue = (i * 37 + k * 90) % 360;
            s += &format!(
                "    path {{ point {} {y}; point {} {y}; gradient 1.5 {{ stop 0 0.7 0.15 {hue} 0.7; stop 40 0.7 0.15 {hue} 0; }} }}\n",
                cx as i64 - 2 * r as i64,
                cx as i64 + 2 * r as i64
            );
        }
        s += "}\n";
    }
    s += "path { point 0 0; point 1 0; gradient 0 { stop 0 0.2 0.03 260; } }\n";
    s
}

/// A full-image path under `depth` masks nested inside each other.
fn nested_mask(width: u32, height: u32, depth: usize) -> String {
    let (cx, cy) = (width / 2, height / 2);
    let mut s = format!("image width={width} height={height}\npath {{\n");
    for i in 0..depth {
        let r = 100 + 40 * i;
        s += &format!(
            "mask {{ path {{ point {} {cy}; point {} {cy}; gradient 0 {{ stop 0 0 0 0 0.95; stop {r} 0 0 0 0; }} }}\n",
            cx - 1,
            cx + 1
        );
    }
    s += &"}\n".repeat(depth);
    s += "point 0 0\npoint 1 0\ngradient 0 { stop 0 0.7 0.1 30; }\n}\n";
    s
}

/// `n` strokes that all share one mask through `use`.
fn shared_mask(width: u32, height: u32, n: usize) -> String {
    let mut s =
        format!("image width={width} height={height}\ndefs {{ path id=\"m\" closed=#true {{\n");
    s += &format!(
        "point {} {}; point {} {}; point {} {}\n",
        width / 4,
        height / 4,
        width * 3 / 4,
        height / 3,
        width / 2,
        height * 3 / 4
    );
    s += "gradient 1 { stop 0 0 0 0; stop 80 0 0 0 0; }\ngradient 3 { stop 0 0 0 0; } } }\n";
    for i in 0..n {
        let y = (i as u32 * height) / n as u32;
        s += &format!(
            "path {{ mask {{ use \"m\" }}; point 0 {y}; point {width} {}; gradient 1.5 {{ stop 0 0.7 0.15 {}; stop 50 0.7 0.15 {} 0; }} }}\n",
            height - y,
            i * 17 % 360,
            i * 17 % 360
        );
    }
    s
}

fn benches(c: &mut Criterion) {
    let mut g = c.benchmark_group("render");
    g.sample_size(10);
    for (name, w, h, n, pts) in [
        ("1920x1080, 20 paths x 12 points", 1920, 1080, 20, 12),
        ("4096x4096, 50 paths x 40 points", 4096, 4096, 50, 40),
    ] {
        let scene = parse(&many_paths(w, h, n, pts)).unwrap().scene.unwrap();
        g.bench_function(name, |b| b.iter(|| render(black_box(&scene))));
    }
    for (name, easing) in [
        ("1920x1080, 20 paths x 12 points, ease", "ease"),
        ("1920x1080, 20 paths x 12 points, steps(4)", "steps(4)"),
        (
            "1920x1080, 20 paths x 12 points, linear-shaped bezier",
            "cubic-bezier(0.25, 0.25, 0.75, 0.75)",
        ),
    ] {
        let scene = parse(&many_paths_eased(1920, 1080, 20, 12, easing))
            .unwrap()
            .scene
            .unwrap();
        g.bench_function(name, |b| b.iter(|| render(black_box(&scene))));
    }
    for (name, src) in [
        (
            "1920x1080, 20 masked groups x 3 paths",
            masked_groups(1920, 1080, 20),
        ),
        ("1920x1080, 16 nested masks", nested_mask(1920, 1080, 16)),
        (
            "1920x1080, 40 paths sharing a mask",
            shared_mask(1920, 1080, 40),
        ),
    ] {
        let scene = parse(&src).unwrap().scene.unwrap();
        g.bench_function(name, |b| b.iter(|| render(black_box(&scene))));
    }
    let scene = parse(&many_paths(1920, 1080, 20, 12))
        .unwrap()
        .scene
        .unwrap();
    let canvas = render(&scene);
    g.bench_function("png encode 1920x1080", |b| {
        b.iter(|| {
            let mut out = Vec::with_capacity(1 << 22);
            canvas.write_png(&mut out).unwrap();
            black_box(out)
        })
    });
    g.finish();
}

criterion_group!(group, benches);
criterion_main!(group);
