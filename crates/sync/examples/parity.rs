//! Swift-against-Rust parity on recordings, replayed through Sonalign. See `crates/sync/parity/README.md`.
//!
//! `cargo run --release -p ensomi-sync --example parity -- ROOT`, where ROOT holds what `parity/main.swift` wrote. Every
//! run replays a query at the Swift path's cadence, a five-second window each 100 ms of audio:
//!
//! - `swift`: the Swift extractor's frames through Sonalign, the baseline.
//! - `rust`: this crate's frames, from the same 48 kHz PCM.
//! - `converted`: this crate's frames for a reference at its own rate, through this crate's conversion.
//! - `session`: a whole `Session`, fed 20 ms blocks of the query, against the `converted` reference.

use ensomi_sync::{frames_f32le, FeatureFrame, Reference, HOP_MS};
use sonalign::{Configuration, Engine, FeatureWindow, LockPhase, State};
use std::fs;
use std::path::Path;
use std::time::Instant;

const BANDS: usize = 24;
const QUERY_MS: f64 = 5_000.0;
const INTERVAL_MS: f64 = 100.0;

fn doubles(path: &Path) -> Vec<f64> {
    fs::read(path).unwrap().as_chunks::<8>().0.iter().map(|b| f64::from_le_bytes(*b)).collect()
}

fn floats(path: &Path) -> Vec<f32> {
    fs::read(path).unwrap().as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect()
}

/// The frames the Swift exporter wrote under `directory`.
fn swift_frames(directory: &Path) -> Vec<FeatureFrame> {
    let (times, energies, pcen) = (doubles(&directory.join("times.f64")), doubles(&directory.join("energy.f64")), floats(&directory.join("pcen.f32")));
    assert_eq!(pcen.len(), times.len() * BANDS);
    times.iter().zip(&energies).zip(pcen.chunks(BANDS)).map(|((&recorded_time_ms, &energy_dbfs), bands)| FeatureFrame { recorded_time_ms, energy_dbfs, pcen_mel: bands.to_vec() }).collect()
}

#[derive(Default)]
struct Difference {
    frames: usize,
    extra_frames: i64,
    time_ms: f64,
    energy_max: f64,
    energy_mean: f64,
    pcen_max: f64,
    pcen_mean: f64,
    pcen_p999: f64,
}

fn difference(a: &[FeatureFrame], b: &[FeatureFrame]) -> Difference {
    let mut out = Difference { frames: a.len().min(b.len()), extra_frames: a.len() as i64 - b.len() as i64, ..Difference::default() };
    let mut pcen = Vec::with_capacity(out.frames * BANDS);
    for (a, b) in a.iter().zip(b) {
        out.time_ms = out.time_ms.max((a.recorded_time_ms - b.recorded_time_ms).abs());
        let energy = (a.energy_dbfs - b.energy_dbfs).abs();
        out.energy_max = out.energy_max.max(energy);
        out.energy_mean += energy;
        pcen.extend(a.pcen_mel.iter().zip(&b.pcen_mel).map(|(a, b)| f64::from((a - b).abs())));
    }
    pcen.sort_by(f64::total_cmp);
    out.energy_mean /= out.frames as f64;
    out.pcen_mean = pcen.iter().sum::<f64>() / pcen.len() as f64;
    out.pcen_max = *pcen.last().unwrap();
    out.pcen_p999 = pcen[(pcen.len() as f64 * 0.999) as usize];
    out
}

#[derive(Clone, Copy, PartialEq)]
struct Event {
    endpoint_ms: f64,
    state: State,
    phase: LockPhase,
    offset_ms: Option<f64>,
}

fn replay(reference: &[FeatureFrame], query: &[FeatureFrame]) -> Vec<Event> {
    let mut engine = Engine::new(reference, HOP_MS, Configuration::default());
    let (mut events, mut start, mut last) = (Vec::new(), 0, None);
    for end in 0..query.len() {
        let endpoint_ms = query[end].recorded_time_ms;
        if last.is_some_and(|last| endpoint_ms - last < INTERVAL_MS) {
            continue;
        }
        last = Some(endpoint_ms);
        while query[start].recorded_time_ms < endpoint_ms - QUERY_MS {
            start += 1;
        }
        let snapshot = engine.process(&FeatureWindow::new(query[start..=end].to_vec()).unwrap(), endpoint_ms);
        events.push(Event { endpoint_ms, state: snapshot.state, phase: snapshot.phase, offset_ms: snapshot.estimate.map(|estimate| estimate.offset_ms) });
    }
    events
}

/// A whole session in 20 ms blocks, on a capture clock that starts at zero, and the cost of each push.
fn listen(reference: &Reference, pcm: &[f32]) -> (Vec<Event>, Vec<f64>) {
    let mut session = reference.session(48_000.0).unwrap();
    let (mut events, mut costs) = (Vec::new(), Vec::new());
    for (index, block) in pcm.chunks(960).enumerate() {
        let started = Instant::now();
        let snapshot = session.push(block, index as f64 * 20.0).unwrap();
        costs.push(started.elapsed().as_secs_f64() * 1_000.0);
        let endpoint_ms = (index + 1) as f64 * 20.0;
        let offset_ms = snapshot.position.map(|position| position.reference_time_ms - position.capture_time_ms);
        events.push(Event { endpoint_ms, state: snapshot.state, phase: snapshot.phase, offset_ms });
    }
    (events, costs)
}

fn is_final(event: &Event) -> bool {
    event.state == State::Locked && event.phase == LockPhase::Final
}

/// First final lock and last state against the baseline, and, for runs with the same cadence, every call compared.
fn compare(name: &str, id: &str, run: &[Event], baseline: &[Event]) -> Option<(f64, f64)> {
    let (a, b) = (run.iter().find(|event| is_final(event)), baseline.iter().find(|event| is_final(event)));
    let lock = a.zip(b).map(|(a, b)| (a.endpoint_ms - b.endpoint_ms, a.offset_ms.unwrap() - b.offset_ms.unwrap()));
    let (mut state_differences, mut worst_offset) = (0, 0.0f64);
    if run.len() == baseline.len() {
        for (a, b) in run.iter().zip(baseline) {
            state_differences += usize::from(a.state != b.state || a.phase != b.phase || a.offset_ms.is_some() != b.offset_ms.is_some());
            if let (Some(a), Some(b)) = (a.offset_ms, b.offset_ms) {
                worst_offset = worst_offset.max((a - b).abs());
            }
        }
    }
    let locked = |event: Option<&Event>| event.map_or("no".to_string(), |event| format!("{:.0}", event.endpoint_ms));
    println!(
        "{id}\t{name}\tfinal at {} (baseline {})\tlock delta {:?}\toffset delta {:?}\tstate differences {state_differences}/{}\tworst offset delta {worst_offset:.3}\tlast {:?}/{:?}",
        locked(a),
        locked(b),
        lock.map(|lock| lock.0),
        lock.map(|lock| lock.1),
        run.len(),
        run.last().map(|event| event.state),
        baseline.last().map(|event| event.state),
    );
    lock
}

fn describe(what: &str, id: &str, d: &Difference) {
    println!(
        "{id}\t{what}\tframes {} extra {}\ttime max {:.2e}\tenergy max {:.2e} mean {:.2e}\tpcen max {:.2e} p99.9 {:.2e} mean {:.2e}",
        d.frames, d.extra_frames, d.time_ms, d.energy_max, d.energy_mean, d.pcen_max, d.pcen_p999, d.pcen_mean
    );
}

fn percentile(sorted: &[f64], at: f64) -> f64 {
    sorted[((sorted.len() - 1) as f64 * at) as usize]
}

fn main() {
    let root = std::env::args().nth(1).expect("the export directory");
    let root = Path::new(&root);
    let (mut same_pcm, mut converted_locks, mut session_locks, mut costs) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let (mut build_ms_per_minute, mut wrong_swift, mut wrong_rust, mut cases) = (Vec::new(), 0, 0, 0);
    for line in fs::read_to_string(root.join("cases.tsv")).unwrap().lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        let (id, query_directory, reference_directory, wrong_directory) = (fields[0], root.join(fields[1]), root.join(fields[2]), root.join(fields[3]));
        cases += 1;
        let swift_reference = swift_frames(&reference_directory);
        let swift_query = swift_frames(&query_directory);
        let reference_pcm = fs::read(reference_directory.join("pcm.f32")).unwrap();
        let source_pcm = fs::read(reference_directory.join("source.f32")).unwrap();
        let source_rate: f64 = fs::read_to_string(reference_directory.join("source-rate")).unwrap().trim().parse().unwrap();
        let query_pcm = fs::read(query_directory.join("pcm.f32")).unwrap();

        let rust_reference = frames_f32le(&reference_pcm, 48_000.0).unwrap();
        let converted_reference = frames_f32le(&source_pcm, source_rate).unwrap();
        let rust_query = frames_f32le(&query_pcm, 48_000.0).unwrap();
        describe("reference, same PCM", id, &difference(&rust_reference, &swift_reference));
        describe(&format!("reference, own conversion from {source_rate}"), id, &difference(&converted_reference, &swift_reference));
        describe("query, same PCM", id, &difference(&rust_query, &swift_query));

        let baseline = replay(&swift_reference, &swift_query);
        same_pcm.extend(compare("rust", id, &replay(&rust_reference, &rust_query), &baseline));
        converted_locks.extend(compare("converted", id, &replay(&converted_reference, &rust_query), &baseline));

        let started = Instant::now();
        let reference = Reference::from_f32le(&source_pcm, source_rate).unwrap();
        build_ms_per_minute.push(started.elapsed().as_secs_f64() * 1_000.0 / (reference.duration_ms() / 60_000.0));
        let samples: Vec<f32> = query_pcm.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect();
        let (events, cost) = listen(&reference, &samples);
        session_locks.extend(compare("session", id, &events, &baseline));
        costs.extend(cost);

        let wrong = swift_frames(&wrong_directory);
        wrong_swift += usize::from(replay(&wrong, &swift_query).iter().any(is_final));
        wrong_rust += usize::from(replay(&frames_f32le(&fs::read(wrong_directory.join("pcm.f32")).unwrap(), 48_000.0).unwrap(), &rust_query).iter().any(is_final));
    }
    let summary = |name: &str, locks: &[(f64, f64)]| {
        let worst = |pick: fn(&(f64, f64)) -> f64| locks.iter().map(pick).map(f64::abs).fold(0.0, f64::max);
        println!("{name}: {} locks compared; worst |lock delta| {:.1} ms, worst |offset delta| {:.3} ms", locks.len(), worst(|lock| lock.0), worst(|lock| lock.1));
    };
    println!("{cases} cases");
    summary("rust", &same_pcm);
    summary("converted", &converted_locks);
    summary("session", &session_locks);
    println!("final locks against the wrong reference: swift {wrong_swift}, rust {wrong_rust}, of {cases}");
    costs.sort_by(f64::total_cmp);
    build_ms_per_minute.sort_by(f64::total_cmp);
    println!("reference build: {:.1} ms per minute of audio, median", percentile(&build_ms_per_minute, 0.5));
    println!("push of 20 ms: p50 {:.3} ms, p99 {:.3} ms, max {:.3} ms, over {} pushes", percentile(&costs, 0.5), percentile(&costs, 0.99), percentile(&costs, 1.0), costs.len());
}
