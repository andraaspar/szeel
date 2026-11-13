use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use szeel::parse::parse;
use szeel::render::render;

/// `n` stacked open paths with `points` points each, each with an implicit width.
fn many_paths(width: u32, height: u32, n: usize, points: usize) -> String {
    let mut s = format!("image width={width} height={height}\n");
    for i in 0..n {
        s += "path {\n";
        for j in 0..points {
            let x = (j as f64 + 0.5) / points as f64 * width as f64;
            let y = height as f64 * (0.5 + 0.4 * ((j as f64 * 0.9 + i as f64 * 0.37).sin()));
            s += &format!("point {x:.2} {y:.2}\n");
        }
        let hue = (i * 37) % 360;
        s += &format!("gradient 1.5 {{ stop 0 0.7 0.15 {hue}; stop 30 0.7 0.15 {hue} 0.5; stop 60 0.6 0.1 {hue} 0; }}\n");
        s += &format!("gradient 3.5 {{ stop 0 0.5 0.1 {}; stop 40 0.5 0.1 {} 0; }}\n}}\n", hue + 120, hue + 120);
    }
    // An opaque background path that fills everything else.
    s += "path closed=#true { point 100 100; point 900 200; point 500 700\n";
    s += "gradient 1 { stop 0 0.3 0.05 250; stop 500 0.1 0.02 280; }\n";
    s += "gradient 3 { stop 0 0.9 0.03 80; }\n}\n";
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
        g.bench_function(name, |b| b.iter(|| render(black_box(&scene), |_, _| {})));
    }
    let scene = parse(&many_paths(1920, 1080, 20, 12)).unwrap().scene.unwrap();
    let canvas = render(&scene, |_, _| {});
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
