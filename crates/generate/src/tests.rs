//! Deterministic pieces of the pipeline against small values from the TypeScript runtime, so that `cargo test`
//! checks them without the model package or the recordings.
//!
//! Where only +, -, *, / and sqrt are involved the two runtimes agree to the last bit and the values are compared
//! exactly. Where `exp`, `log`, `sin` or `cos` enter (libm here, V8 there) the last bit may differ: f32 results are
//! compared to one f32 ulp and f64 results to 1e-9 relative.
use crate::audio::{features, load_resampler, mel_pcm, resample, resampler_files, sinc_resample, spectrogram, Resampler};
use crate::controls::{action_tables, chance, fallback_action, log_normalize, logit, tilt, transition, Playability, Row};
use crate::export::{audio_filename, chart_from_decisions, osu_text, validate_chart, Chart, ChartObject, OsuOptions};
use crate::fitter::Segment;
use crate::grid::Grid;
use crate::json::{number_string, stringify};
use crate::models::Masks;
use crate::r2::ChartState;
use crate::random::{RandomSource, SeededRandom};
use serde_json::json;
use std::rc::Rc;

fn close(got: f64, want: f64, relative: f64) -> bool {
    got == want || (got - want).abs() <= want.abs() * relative
}

fn assert_close32(got: &[f32], want: &[f32]) {
    assert_eq!(got.len(), want.len());
    for (i, (&g, &w)) in got.iter().zip(want).enumerate() {
        assert!(close(g as f64, w as f64, f32::EPSILON as f64), "value {i}: got {g}, want {w}");
    }
}

fn assert_close64(got: &[f64], want: &[f64]) {
    assert_eq!(got.len(), want.len());
    for (i, (&g, &w)) in got.iter().zip(want).enumerate() {
        assert!(close(g, w, 1e-9), "value {i}: got {g}, want {w}");
    }
}

/// The sum of the values in order, as the TypeScript scripts computed it.
fn total(values: &[f32]) -> f64 {
    values.iter().map(|&x| x as f64).sum()
}

/// A sawtooth plus a slow ramp. Only exact arithmetic, so that both runtimes build the same samples.
fn signal(n: usize) -> Vec<f64> {
    (0..n).map(|i| 0.5 * ((i * 7 % 100) as f64 / 50.0 - 1.0) + i as f64 / 4000.0).collect()
}

fn resampler(rate: f64) -> Resampler {
    let files = resampler_files(rate).unwrap().map(|name| std::fs::read(format!("{}/../../assets/resampler/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap());
    load_resampler(&files).unwrap()
}

#[test]
fn resample_with_the_soxr_tables() {
    let pcm = signal(2000);

    // From the TypeScript runtime's resample() on this signal: the length, the sum of the output, and samples at the
    // start (inside the 128 boundary frames), across the end of the boundary frames, in the middle and at the end.
    let out = resample(&pcm, &resampler(48000.0));
    assert_eq!(out.len(), 919);
    assert_eq!(total(&out), 224.91424872458447);
    assert_eq!(out[..5], [-0.3384899, -0.38842845, -0.16620761, -0.06615617, 0.1384731]);
    assert_eq!(out[126..131], [-0.27614424, -0.06637459, 0.0812237, 0.1995863, 0.45045596]);
    assert_eq!(out[459..464], [0.37499136, -0.27880275, 0.083738714, 0.07756946, 0.3778401]);
    assert_eq!(out[914..], [0.3198952, 0.39946988, 0.594929, 0.7276658, 0.85427594]);

    let out = resample(&pcm, &resampler(44100.0));
    assert_eq!(out.len(), 1000);
    assert_eq!(total(&out), 244.6665113743511);
    assert_eq!(out[..5], [-0.34787184, -0.41259766, -0.18015589, -0.11425315, 0.09994672]);
    assert_eq!(out[126..131], [0.21171814, 0.32059437, 0.5389048, -0.112957865, -0.31562057]);
    assert_eq!(out[500..505], [0.011036088, -0.19130808, 0.08506644, 0.1256678, 0.35738915]);
    assert_eq!(out[995..], [0.33577204, 0.4070367, 0.61027527, 0.6781527, 0.9300217]);
}

#[test]
fn mel_pcm_converts_to_24_khz_and_normalizes_the_peak() {
    // From the TypeScript runtime's melPcm() on 1000 samples at 44100 Hz.
    let out = mel_pcm(&signal(1000), 44100.0);
    assert_eq!(out.len(), 544);
    assert_eq!(total(&out), 94.35721022251528);
    assert_eq!(out[..5], [-0.7254525, -0.5381633, -0.35087416, -0.16358499, 0.023704162]);
    assert_eq!(out[272..277], [-0.2742936, -0.37718543, -0.18989626, -0.0026070948, 0.18468207]);
    assert_eq!(out[539..], [0.11096249, 0.29825166, 0.4855408, 0.67283, 0.86011916]);
    assert_eq!(out.iter().fold(0.0, |peak, x| f32::max(peak, x.abs())), 1.0);

    // Silence has no peak to divide by.
    assert_eq!(mel_pcm(&[0.0f64; 100], 48000.0), vec![0.0f32; 50]);
}

#[test]
fn sinc_resample_converts_a_rate_without_a_table() {
    // From the TypeScript runtime's sincResample() on 600 samples at 37800 Hz to 22050 Hz.
    let out = sinc_resample(&signal(600), 37800.0, 22050.0);
    assert_eq!(out.len(), 350);
    assert!(close(total(&out), 24.313378170481883, 1e-6));
    assert_close32(&out[..5], &[-0.40016297, -0.39347047, -0.26436558, -0.12290692, -0.042264983]);
    assert_close32(&out[175..180], &[-0.20070826, -0.37155908, -0.1534588, -0.074290276, 0.05216983]);
    assert_close32(&out[345..], &[0.0011810467, 0.20743373, 0.25821903, 0.42764148, 0.5342164]);
}

/// A periodic Hann window.
fn hann(n: usize) -> Vec<f32> {
    (0..n).map(|j| (0.5 - 0.5 * libm::cos(2.0 * std::f64::consts::PI * j as f64 / n as f64)) as f32).collect()
}

/// 128 bands of five overlapping weights each, spread over the bins; BeatThis stores bin-major, the rows model band-major.
fn mel_filter(bins: usize, beatthis: bool) -> Vec<f32> {
    let mut filter = vec![0f32; bins * 128];
    for m in 0..128 {
        for (t, weight) in [0.0625, 0.25, 0.375, 0.25, 0.0625].into_iter().enumerate() {
            let k = m * (bins - 5) / 127 + t;
            filter[if beatthis { k * 128 + m } else { m * bins + k }] = weight;
        }
    }
    filter
}

#[test]
fn spectrogram_of_both_frontends() {
    // From the TypeScript runtime's spectrogram() on the same pcm, window and filter bank: the sum of all values and
    // bands 0, 1, 17, 64, 100 and 127 of three frames. The pcm goes silent at sample 1800, so the last frame is empty.
    let mut pcm: Vec<f32> = signal(1800).into_iter().map(|x| x as f32).collect();
    pcm.resize(3000, 0.0);
    let bands = [0, 1, 17, 64, 100, 127];
    let frame = |out: &[f32], f: usize| bands.map(|m| out[f * 128 + m]);

    let out = spectrogram(pcm.clone(), hann(1024), &mel_filter(513, true), true);
    assert_eq!(out.len(), 7 * 128);
    assert!(close(total(&out), 2227.321885973215, 1e-6));
    assert_close32(&frame(&out, 0), &[5.128721, 4.2468405, 7.021379, 3.9149306, 3.6046515, 3.8370392]);
    assert_close32(&frame(&out, 4), &[7.459535, 5.991428, 6.6736765, 3.3170223, 3.070223, 3.6686122]);
    assert_eq!(frame(&out, 6), [0.0; 6]);

    // The rows model floors the power at 1e-5 before the log.
    let out = spectrogram(pcm, hann(960), &mel_filter(481, false), false);
    assert_eq!(out.len(), 13 * 128);
    assert!(close(total(&out), -8317.161031486467, 1e-6));
    assert_close32(&frame(&out, 0), &[6.0679793, -2.6880267, 6.2745576, 1.4208835, 1.0254316, -0.33186176]);
    assert_close32(&frame(&out, 5), &[8.38604, 5.118827, 6.3058968, 0.98196894, 0.7644428, 0.3324455]);
    assert_close32(&frame(&out, 12), &[-11.512925; 6]);
}

#[test]
fn features_standardize_log_mel_and_interpolate_beats() {
    // From the TypeScript runtime's features(): four frames against beat arrays of three, so the last frames clamp.
    // The values are the first and the last four of each 130-value frame.
    let log_mel: Vec<f32> = (0..4 * 128).map(|i| (i * 5 % 17) as f32 * 0.25 - 1.0).collect();
    let out = features(&log_mel, &[0.1, 0.9, 0.4], &[0.7, 0.2, 0.55]);
    assert_eq!(out.len(), 520);
    assert_eq!(total(&out), 4.004913330078125);
    let frame = |f: usize| [&out[f * 130..f * 130 + 4], &out[f * 130 + 126..f * 130 + 130]].concat();
    assert_eq!(frame(0), [-1.6269531, -0.6074219, 0.41210938, 1.4316406, -1.4228516, -0.40356445, 0.89990234, 0.19995117]);
    assert_eq!(frame(1), [0.61621094, 1.6357422, -0.81152344, 0.20825195, 0.8198242, -1.6269531, 0.64990234, 0.375]);
    assert_eq!(frame(2), [-0.6074219, 0.41210938, 1.4316406, -1.015625, -0.40356445, 0.61621094, 0.39990234, 0.5498047]);
    assert_eq!(frame(3), [1.6357422, -0.81152344, 0.20825195, 1.2275391, -1.6269531, -0.6074219, 0.39990234, 0.5498047]);
}

#[test]
fn seeded_random_follows_mulberry32() {
    assert_eq!(SeededRandom::new(0).algorithm(), "mulberry32-u53-v1");

    // From the TypeScript runtime's SeededRandom after reset(seed): 4 orientations, 6 multinomial draws over 16
    // weights, 6 gumbel draws over 16 logits and 2 more orientations, in one stream.
    let weights: Vec<f32> = (0..16).map(|i| ((i % 5) + 1) as f32 / 8.0).collect();
    let logits: Vec<f64> = (0..16).map(|i| -((i % 4) as f64) * 0.5).collect();
    let draws = |random: &mut SeededRandom| {
        let mut out = Vec::new();
        for _ in 0..4 {
            out.push(random.orientation("o").unwrap() as usize);
        }
        for _ in 0..6 {
            out.push(random.multinomial(&weights, "m").unwrap());
        }
        for _ in 0..6 {
            out.push(random.gumbel(&logits, "g").unwrap());
        }
        for _ in 0..2 {
            out.push(random.orientation("o").unwrap() as usize);
        }
        out
    };
    for (seed, want) in [
        (0, [0, 1, 0, 0, 4, 14, 8, 13, 3, 4, 5, 14, 0, 9, 0, 4, 1, 0]),
        (1, [1, 1, 1, 1, 0, 14, 3, 4, 4, 11, 1, 0, 8, 7, 6, 0, 0, 1]),
        (42, [0, 0, 0, 1, 8, 9, 10, 1, 9, 12, 12, 2, 3, 7, 5, 14, 1, 1]),
        (2147483648, [0, 1, 0, 1, 14, 2, 7, 11, 8, 10, 0, 9, 0, 2, 5, 14, 0, 0]),
        (u32::MAX, [1, 0, 0, 0, 8, 13, 3, 2, 8, 7, 9, 0, 7, 12, 9, 5, 1, 1]),
    ] {
        let mut random = SeededRandom::new(seed);
        assert_eq!(draws(&mut random), want, "seed {seed}");
        random.reset(seed);
        assert_eq!(draws(&mut random), want, "seed {seed} after reset");
    }
}

/// Six head rows and the final row. Lane 0 only taps. Lane 1 holds from 1000, closed on a row (code 1); holds from
/// 2000, closed in the gap with a new hold (code 4) whose release 2210.5 rounds half to even; and that hold is closed on
/// a row. Lane 2 holds, is closed in the gap and taps at once (code 3), and holds again until the final row, whose gap
/// release lies past the song and is clipped. Lane 3 taps, then holds with a gap release before the previous row, which
/// is clipped, then taps.
fn sample_chart() -> Chart {
    let head_ms = [1000.0, 1500.0, 2000.0, 2500.0, 3000.0, 3500.0];
    let actions = [[1, 2, 2, 0], [0, 1, 0, 1], [1, 2, 3, 0], [0, 4, 0, 2], [1, 0, 0, 2], [0, 1, 2, 1], [0, 0, 2, 0]];
    let mut gap = [[f64::NAN; 4]; 7];
    gap[2][2] = 1740.4;
    gap[3][1] = 2210.5;
    gap[4][3] = 2100.0;
    gap[6][2] = 4500.0;
    chart_from_decisions(&head_ms, 4000.0, &actions, &gap)
}

#[test]
fn chart_from_decisions_closes_holds_on_rows_and_in_gaps() {
    // From the TypeScript runtime's chartFromDecisions() on the decisions of `sample_chart`.
    let object = |lane, start_ms, end_ms| ChartObject { lane, start_ms, end_ms };
    let chart = sample_chart();
    assert_eq!(chart.duration_ms, 4000.0);
    assert_eq!(
        chart.objects,
        [
            object(0, 1000.0, None),
            object(1, 1000.0, Some(1500.0)),
            object(2, 1000.0, Some(1740.0)),
            object(3, 1500.0, None),
            object(0, 2000.0, None),
            object(1, 2000.0, Some(2210.0)),
            object(2, 2000.0, None),
            object(1, 2500.0, Some(3500.0)),
            object(3, 2500.0, Some(2501.0)),
            object(0, 3000.0, None),
            object(2, 3500.0, Some(3999.0)),
            object(3, 3500.0, None),
        ]
    );
}

#[test]
fn validate_chart_rejects_illegal_charts() {
    assert_eq!(validate_chart(&sample_chart()), Ok(()));

    // The messages are those of the TypeScript runtime's validateChart().
    let tap = |lane, start_ms| ChartObject { lane, start_ms, end_ms: None };
    let hold = |lane, start_ms, end_ms| ChartObject { lane, start_ms, end_ms: Some(end_ms) };
    let invalid = |duration_ms, objects| validate_chart(&Chart { duration_ms, objects });
    assert_eq!(invalid(4000.5, vec![]), Err("Chart duration must be nonnegative whole milliseconds".into()));
    assert_eq!(invalid(4000.0, vec![hold(1, 1000.0, 2000.0), tap(1, 1500.0)]), Err("Head inside an open hold in lane 1 at 1500".into()));
    assert_eq!(invalid(4000.0, vec![hold(0, 1000.0, 1000.0)]), Err("Invalid hold release in lane 0 at 1000".into()));
    assert_eq!(invalid(4000.0, vec![hold(0, 1000.0, 4001.0)]), Err("Invalid hold release in lane 0 at 1000".into()));
    assert_eq!(invalid(4000.0, vec![hold(0, 1000.0, 1500.5)]), Err("Invalid hold release in lane 0 at 1000".into()));
    assert_eq!(invalid(4000.0, vec![hold(0, 10.0, f64::NAN)]), Err("Invalid hold release in lane 0 at 10".into()));
    let unsorted = "Chart objects must be sorted by start then lane, with no duplicate heads";
    assert_eq!(invalid(4000.0, vec![tap(0, 2000.0), tap(0, 1000.0)]), Err(unsorted.into()));
    assert_eq!(invalid(4000.0, vec![tap(2, 2000.0), tap(2, 2000.0)]), Err(unsorted.into()));

    // The TypeScript runtime puts the object's JSON in this message; here it names the lane and the start.
    assert!(invalid(4000.0, vec![tap(0, 4500.0)]).is_err());
    assert!(invalid(4000.0, vec![tap(0, 10.5)]).is_err());
}

#[test]
fn osu_text_writes_the_chart_file() {
    // From the TypeScript runtime's osuText() on `sample_chart`. The generator comment is the one line that differs:
    // it names this runtime. Timing points round their offset half to even (1.5 and 2000.5) and write a whole beat
    // length with ".0"; the newlines in the title and artist become spaces; bookmarks are truncated.
    let segments = [Segment { offset_ms: 1.5, beat_length_ms: 500.0 }, Segment { offset_ms: 2000.5, beat_length_ms: 333.3333333333333 }];
    let options = OsuOptions {
        title: "Night\nShift",
        artist: "A\r\nB",
        setting: "r3",
        seed: "4294967295".to_string(),
        audio_name: "audio.mp3",
        bookmarks: &[1000.7, 2500.0, 3999.9],
        random_algorithm: "mulberry32-u53-v1",
    };
    let runtime = format!("{} {}", crate::RUNTIME_NAME, crate::RUNTIME_VERSION);
    let want = format!(
        r"osu file format v14
[General]
AudioFilename:audio.mp3
Mode:3
[Metadata]
Title:Night Shift
Artist:A  B
Creator:Ensomi
Version:Ensomi r3 s4294967295
BeatmapID:0
BeatmapSetID:-1
[Editor]
Bookmarks:1000,2500,3999
[Difficulty]
CircleSize:4
[TimingPoints]
2,500.0,4,2,0,100,1,0
2000,333.3333333333333,4,2,0,100,1,0
// {runtime} seed=4294967295 random=mulberry32-u53-v1
[HitObjects]
64,192,1000,1,0,0:0:0:0:
192,192,1000,128,0,1500:0:0:0:0:
320,192,1000,128,0,1740:0:0:0:0:
448,192,1500,1,0,0:0:0:0:
64,192,2000,1,0,0:0:0:0:
192,192,2000,128,0,2210:0:0:0:0:
320,192,2000,1,0,0:0:0:0:
192,192,2500,128,0,3500:0:0:0:0:
448,192,2500,128,0,2501:0:0:0:0:
64,192,3000,1,0,0:0:0:0:
320,192,3500,128,0,3999:0:0:0:0:
448,192,3500,1,0,0:0:0:0:
"
    );
    assert_eq!(osu_text(&sample_chart(), &segments, &options), want);
}

#[test]
fn audio_filename_keeps_a_plain_extension() {
    // From the TypeScript runtime's audioFilename().
    for (name, want) in [
        ("song.MP3", "audio.mp3"),
        ("track.flac", "audio.flac"),
        ("noextension", "audio.bin"),
        ("trailing.", "audio.bin"),
        (".ogg", "audio.ogg"),
        ("a.tar.gz", "audio.gz"),
        ("dir.v2/name", "audio.bin"),
        ("song.mp3é", "audio.bin"),
        ("x.m4a\n", "audio.bin"),
        ("my song.WaV", "audio.wav"),
        ("a.b-c", "audio.bin"),
        ("", "audio.bin"),
    ] {
        assert_eq!(audio_filename(name), want, "{name:?}");
    }
}

#[test]
fn number_string_matches_javascript() {
    // JavaScript's String(x): fixed notation from 1e-6 up to 1e21, exponent notation beyond.
    for (x, want) in [
        (0.0, "0"),
        (-0.0, "0"),
        (1.0, "1"),
        (-1.0, "-1"),
        (7.0, "7"),
        (0.5, "0.5"),
        (-1.5, "-1.5"),
        (0.1, "0.1"),
        (100.0, "100"),
        (123456789.0, "123456789"),
        (100000000000000000000.0, "100000000000000000000"),
        (123456789012345680000.0, "123456789012345680000"),
        (1.0e21, "1e+21"),
        (1.5e21, "1.5e+21"),
        (1.5e300, "1.5e+300"),
        (5.0e-324, "5e-324"),
        (1.7976931348623157e308, "1.7976931348623157e+308"),
        (0.000001, "0.000001"),
        (1.0e-7, "1e-7"),
        (1.5e-7, "1.5e-7"),
        (123456.789, "123456.789"),
        (4.35, "4.35"),
        (9007199254740992.0, "9007199254740992"),
        (0.30000000000000004, "0.30000000000000004"),
        (0.3333333333333333, "0.3333333333333333"),
        (0.6666666666666666, "0.6666666666666666"),
        (1.0e-10, "1e-10"),
        (0.0123456789, "0.0123456789"),
        (f64::NAN, "NaN"),
        (f64::INFINITY, "Infinity"),
        (f64::NEG_INFINITY, "-Infinity"),
        (-1.0e21, "-1e+21"),
        (-2.5e-8, "-2.5e-8"),
    ] {
        assert_eq!(number_string(x), want, "{x:?}");
    }
}

#[test]
fn stringify_matches_json_stringify_with_two_spaces() {
    // JSON.stringify(value, null, 2): escapes, empty containers, insertion order, JavaScript number formatting.
    let value = json!({
        "name": "Night\nShift \"quoted\" \\ \t \u{1} é",
        "count": 3,
        "ratio": 0.1,
        "big": 1e21,
        "small": 1e-7,
        "neg": -2.5,
        "negzero": -0.0,
        "flags": [true, false, null],
        "nested": {"empty_list": [], "empty_object": {}, "list": [1, [2, 3], {"k": "v"}]},
        "z": 5,
        "a": 123456789012345680000.0
    });
    let want = r#"{
  "name": "Night\nShift \"quoted\" \\ \t \u0001 é",
  "count": 3,
  "ratio": 0.1,
  "big": 1e+21,
  "small": 1e-7,
  "neg": -2.5,
  "negzero": 0,
  "flags": [
    true,
    false,
    null
  ],
  "nested": {
    "empty_list": [],
    "empty_object": {},
    "list": [
      1,
      [
        2,
        3
      ],
      {
        "k": "v"
      }
    ]
  },
  "z": 5,
  "a": 123456789012345680000
}"#;
    assert_eq!(stringify(&value), want);
}

/// The release's 625 actions: four lanes with codes 0 to 4, the last lane varying fastest.
fn actions() -> Vec<Row> {
    (0..625).map(|i| [i / 125, i / 25 % 5, i / 5 % 5, i % 5]).collect()
}

#[test]
fn action_tables_for_every_held_pattern() {
    // From the TypeScript runtime's actionTables(). Its support and final-row sets are inline in arrange.ts; these
    // are that formula on the same patterns (bit l of the pattern: lane l is held).
    let tables = action_tables(&actions());
    assert_eq!(tables.len(), 16);
    assert!(tables.iter().all(|t| [t.heads.len(), t.ln.len(), t.features.len(), t.masks.len(), t.support.len(), t.final_row.len()] == [625; 6]));
    assert_eq!(tables.iter().map(|t| t.support.iter().filter(|&&s| s).count()).collect::<Vec<_>>(), [80, 132, 132, 216, 132, 216, 216, 348, 132, 216, 216, 348, 216, 348, 348, 544]);
    let final_rows: Vec<Vec<usize>> = tables.iter().map(|t| (0..625).filter(|&i| t.final_row[i]).collect()).collect();
    assert_eq!(final_rows, [[0], [250], [50], [300], [10], [260], [60], [310], [2], [252], [52], [302], [12], [262], [62], [312]]);

    // Per action: heads, long heads, [long heads, heads, every lane covered], head mask, allowed on a head row.
    for (pattern, i, heads, ln, features, mask, support) in [
        (0, 0, [0, 0, 0, 0], [0, 0, 0, 0], [0.0, 0.0, 0.0], 0, false),
        (0, 7, [0, 0, 1, 1], [0, 0, 0, 1], [1.0, 2.0, 0.0], 12, true),
        (0, 13, [0, 0, 1, 0], [0, 0, 1, 0], [1.0, 1.0, 0.0], 4, false),
        (0, 98, [0, 0, 0, 0], [0, 0, 0, 0], [0.0, 0.0, 0.0], 0, false),
        (0, 624, [0, 0, 0, 0], [0, 0, 0, 0], [0.0, 0.0, 0.0], 0, false),
        (5, 0, [0, 0, 0, 0], [0, 0, 0, 0], [0.0, 0.0, 0.0], 0, false),
        (5, 7, [0, 0, 0, 1], [0, 0, 0, 1], [1.0, 1.0, 0.0], 8, true),
        (5, 13, [0, 0, 0, 0], [0, 0, 0, 0], [0.0, 0.0, 0.0], 0, false),
        (5, 98, [0, 0, 1, 0], [0, 0, 1, 0], [1.0, 1.0, 0.0], 4, false),
        (5, 624, [1, 0, 1, 0], [1, 0, 1, 0], [2.0, 2.0, 0.0], 5, false),
    ] {
        let table = &tables[pattern];
        assert_eq!((table.heads[i], table.ln[i], table.features[i], table.masks[i], table.support[i]), (heads, ln, features, mask, support), "pattern {pattern} action {i}");
    }

    // The sizes of each concept's groups in order, for patterns 0 and 5.
    for (pattern, concept, sizes) in [
        (0, "jack", vec![81, 108, 54, 12, 1, 108, 108, 36, 4, 54, 36, 6, 12, 4, 1]),
        (0, "stream_subtype", vec![81, 108, 54, 12, 1, 108, 162, 84, 15]),
        (0, "stream", vec![255, 1, 354, 15]),
        (0, "chord", vec![255, 1, 354, 15]),
        (5, "jack", vec![81, 108, 45, 9, 6, 6, 1, 108, 90, 18, 18, 18, 4, 45, 9, 18, 18, 6, 6, 6, 4, 1]),
        (5, "stream_subtype", vec![81, 108, 45, 9, 6, 6, 1, 108, 135, 27, 42, 42, 15]),
        (5, "stream", vec![240, 16, 285, 84]),
        (5, "chord", vec![240, 16, 285, 84]),
    ] {
        assert_eq!(tables[pattern].groups[concept].iter().map(Vec::len).collect::<Vec<_>>(), sizes, "pattern {pattern} {concept}");
    }
    assert_eq!(tables[5].groups["stream"].iter().map(|g| g[0]).collect::<Vec<_>>(), [0, 26, 2, 27]);
}

#[test]
fn tilt_moves_probability_inside_each_group_and_keeps_its_mass() {
    // From the TypeScript runtime's tilt(): two groups, one with a zero-probability member, then a negative weight.
    let logp = [-2.3, -1.6, f64::NEG_INFINITY, -1.2, -1.9, -0.7];
    let feature = [true, false, true, false, true, true];
    let groups = [vec![0, 1, 2], vec![3, 4, 5]];
    let (out, error) = tilt(&logp, &feature, &groups, 0.8);
    assert_close64(&out, &[-1.8412106111881128, -1.9412106111881129, f64::NEG_INFINITY, -1.8075346938375065, -1.7075346938375064, -0.5075346938375066]);
    assert!(error < 1e-12);

    let (out, error) = tilt(&[-0.5, -1.5, -2.5, -3.5], &[true, true, false, false], &[vec![0, 1, 2, 3]], -1.5);
    assert_close64(&out, &[-0.8471489731371342, -1.8471489731371342, -1.3471489731371342, -2.347148973137134]);
    assert!(error < 1e-12);

    // A zero weight leaves the input as it is.
    assert_eq!(tilt(&logp, &feature, &groups, 0.0), (logp.to_vec(), 0.0));
}

#[test]
fn log_normalize_subtracts_the_log_sum() {
    // From the TypeScript runtime's logNormalize(); a -inf entry stays -inf.
    let out = log_normalize(&[-1.0, -2.5, f64::NEG_INFINITY, -0.3, -4.0]);
    assert_close64(&out, &[-1.1898748746680807, -2.6898748746680807, f64::NEG_INFINITY, -0.4898748746680808, -4.189874874668081]);
}

#[test]
fn chance_of_a_repeated_lane() {
    // From the TypeScript runtime's CHANCE table: the chance that b heads repeat a lane of a previous row of a heads.
    let want = [
        [0.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 0.25, 0.5, 0.75, 1.0],
        [0.0, 0.5, 0.8333333333333334, 1.0, 1.0],
        [0.0, 0.75, 1.0, 1.0, 1.0],
        [0.0, 1.0, 1.0, 1.0, 1.0],
    ];
    for a in 0..5 {
        for b in 0..5 {
            assert_eq!(chance(a, b), want[a as usize][b as usize], "chance({a}, {b})");
        }
    }
}

#[test]
fn transition_tells_jacks_from_streams() {
    // From the TypeScript runtime's transition(): (heads, previous row, row before that) to (jack, stream step).
    for (heads, previous, before, want) in [
        ([1, 0, 0, 0], [1, 0, 0, 0], None, (true, false)),
        ([0, 1, 0, 0], [1, 0, 0, 0], None, (false, true)),
        ([1, 0, 0, 0], [0, 1, 0, 0], Some([1, 0, 0, 0]), (false, false)),
        ([0, 0, 0, 1], [1, 1, 1, 0], Some([0, 0, 0, 1]), (false, true)),
        ([1, 1, 0, 0], [0, 0, 1, 1], Some([1, 1, 0, 0]), (false, false)),
        ([0, 1, 1, 0], [1, 0, 0, 1], Some([1, 0, 0, 1]), (false, true)),
        ([1, 0, 1, 0], [0, 1, 0, 1], Some([0, 1, 0, 0]), (false, true)),
        ([0, 0, 0, 0], [0, 1, 0, 0], None, (false, true)),
    ] {
        assert_eq!(transition(&heads, &previous, before.as_ref()), want, "{heads:?} after {previous:?} after {before:?}");
    }
}

#[test]
fn logit_clips_to_the_open_interval() {
    // From the TypeScript runtime's logit(), which clips to [0.01, 0.99] first.
    for (v, want) in [
        (0.5, 0.0),
        (0.25, -1.0986122886681098),
        (0.9, 2.1972245773362196),
        (0.0, -4.59511985013459),
        (1.0, 4.595119850134589),
        (0.005, -4.59511985013459),
        (0.995, 4.595119850134589),
        (0.01, -4.59511985013459),
        (0.123, -1.9643226369997655),
    ] {
        assert_close64(&[logit(v)], &[want]);
    }
}

#[test]
fn fallback_action_takes_the_best_single_head_of_the_longest_idle_lane() {
    // From the TypeScript runtime's fallbackAction(): among finite single-head actions, the lane idle longest, then
    // the highest log probability.
    let heads = [[1, 0, 0, 0], [0, 1, 0, 0], [0, 0, 1, 0], [0, 0, 0, 1], [1, 1, 0, 0], [0, 1, 0, 0]];
    assert_eq!(fallback_action(&[-0.5, -1.0, f64::NEG_INFINITY, -0.2, -0.1, -3.0], &heads, &[50.0, 120.0, 120.0, 30.0]), Ok(1));
    assert_eq!(fallback_action(&[-0.5, -3.0, -0.2, -0.2, -0.1, -1.0], &heads, &[50.0, 120.0, 120.0, 30.0]), Ok(2));
    assert_eq!(fallback_action(&[-0.5, -3.0, -0.2, -0.7, -0.1, -1.0], &heads, &[50.0, 60.0, 20.0, 60.0]), Ok(3));

    // The TypeScript runtime returns undefined here.
    let error = fallback_action(&[f64::NEG_INFINITY; 6], &heads, &[50.0, 60.0, 20.0, 60.0]).unwrap_err();
    assert!(error.starts_with("generation_failed"), "{error}");
}

#[test]
fn playability_rejects_reattacks_quads_and_jack_runs() {
    // From the TypeScript runtime's Playability.rejections() after the same rows, for the candidate head sets (lane
    // bit masks) 1, 2, 4, 8, 15 and 6: per rejection kind (re-attack floor, quad run cap, jack run cap) one character
    // per candidate, 1 when it is forbidden.
    let config = Masks { reattack_floor_ms: 300.0, quad_run_cap: 2.0, gap_tercile_edges_ms: vec![100.0, 200.0], jack_run_caps: vec![1.0, 2.0, 3.0] };
    let head_ms = [1000.0, 1060.0, 1120.0, 1180.0, 1400.0];
    let mut chart = ChartState::new(&head_ms, 2000.0, Rc::new(Grid::new(&[Segment { offset_ms: 0.0, beat_length_ms: 500.0 }])));
    let mut guard = Playability::new(config);
    let candidates = [1, 2, 4, 8, 15, 6];
    let bits = |s: &str| s.chars().map(|c| c == '1').collect::<Vec<_>>();

    assert_eq!(guard.rejections(&chart, 0, &candidates), [bits("000000"), bits("000000"), bits("000000")]);
    for row in [[1, 1, 1, 1], [1, 1, 1, 1]] {
        chart.append(row, [f64::NAN; 4]);
        guard.observe(&row);
    }
    assert_eq!(guard.rejections(&chart, 2, &candidates), [bits("111111"), bits("000010"), bits("111111")]);
    for row in [[1, 1, 0, 0], [1, 0, 0, 0]] {
        chart.append(row, [f64::NAN; 4]);
        guard.observe(&row);
    }
    assert_eq!(guard.rejections(&chart, 4, &candidates), [bits("110011"), bits("000000"), bits("100010")]);
}
