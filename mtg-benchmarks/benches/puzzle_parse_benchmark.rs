//! Benchmark for PZL file parsing performance

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use mtg_forge_rs::puzzle::PuzzleFile;
use std::fs;
use std::path::PathBuf;

fn load_sample_puzzles() -> Vec<(String, String)> {
    let puzzle_dir = PathBuf::from("forge-java/forge-gui/res/puzzle");

    if !puzzle_dir.exists() {
        eprintln!("Warning: puzzle directory not found, using synthetic samples");
        return vec![(
            "synthetic.pzl".to_string(),
            r#"
[metadata]
Name=Test Puzzle
Goal=Win
Turns=1
Difficulty=Easy

[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Lightning Bolt;Grizzly Bears
p0battlefield=Mountain;Mountain;Mountain
p1life=10
p1battlefield=Forest|Tapped;Plains
"#
            .to_string(),
        )];
    }

    // Load a representative sample of puzzles
    let mut puzzles = Vec::new();

    for entry in fs::read_dir(&puzzle_dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();

        if path.extension().and_then(|s| s.to_str()) == Some("pzl") {
            if let Ok(contents) = fs::read_to_string(&path) {
                let filename = path.file_name().unwrap().to_string_lossy().to_string();
                puzzles.push((filename, contents));

                // Sample first 20 puzzles for benchmark
                if puzzles.len() >= 20 {
                    break;
                }
            }
        }
    }

    puzzles
}

fn bench_parse_single(c: &mut Criterion) {
    let puzzles = load_sample_puzzles();

    if puzzles.is_empty() {
        return;
    }

    let mut group = c.benchmark_group("parse_single_puzzle");

    for (filename, contents) in puzzles.iter().take(5) {
        let size = contents.len();
        group.throughput(Throughput::Bytes(size as u64));

        group.bench_with_input(BenchmarkId::from_parameter(filename), &contents, |b, contents| {
            b.iter(|| {
                let result = PuzzleFile::parse(black_box(contents));
                black_box(result)
            });
        });
    }

    group.finish();
}

fn bench_parse_batch(c: &mut Criterion) {
    let puzzles = load_sample_puzzles();

    if puzzles.is_empty() {
        return;
    }

    let total_size: usize = puzzles.iter().map(|(_, c)| c.len()).sum();

    c.benchmark_group("parse_batch")
        .throughput(Throughput::Bytes(total_size as u64))
        .bench_function("parse_20_puzzles", |b| {
            b.iter(|| {
                for (_, contents) in &puzzles {
                    let result = PuzzleFile::parse(black_box(contents));
                    black_box(result).ok();
                }
            });
        });
}

fn bench_parse_sections_only(c: &mut Criterion) {
    let simple_puzzle = r#"
[metadata]
Name=Simple Puzzle
Goal=Win
Turns=1
Difficulty=Easy

[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p1life=10
"#;

    c.benchmark_group("parse_components").bench_function("full_parse", |b| {
        b.iter(|| {
            let result = PuzzleFile::parse(black_box(simple_puzzle));
            black_box(result)
        });
    });
}

criterion_group!(
    benches,
    bench_parse_single,
    bench_parse_batch,
    bench_parse_sections_only
);
criterion_main!(benches);
