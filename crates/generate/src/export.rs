use crate::fitter::Segment;
use crate::json::number_string;
use crate::math::{clip, round};

#[derive(Clone, Debug, PartialEq)]
pub struct ChartObject {
    pub lane: usize,
    pub start_ms: f64,
    pub end_ms: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct Chart {
    pub duration_ms: f64,
    pub objects: Vec<ChartObject>,
}

/// What a page of a streamed chart carries: a tap, or a hold as its start and, possibly in a later page, its end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    Tap,
    HoldStart,
    HoldEnd,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Token {
    pub lane: usize,
    pub time_ms: f64,
    pub kind: TokenKind,
}

/// Part of a chart that will not change, for `Session::append_tokens` of the gameplay crate: tokens in time order, a
/// hold's end before a start at the same time, then by lane, each after the coverage of the previous page.
#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    pub tokens: Vec<Token>,
    /// Every token at or before this time has been delivered.
    pub coverage_ms: f64,
    /// Every token has been delivered.
    pub end_of_stream: bool,
}

/// The release of a hold that row `k` closes with `code`: on the row (1), or at the gap release `gap`, rounded and
/// kept inside the gap before the row.
fn hold_end(head_ms: &[f64], song_ms: f64, k: usize, code: i32, gap: f64) -> f64 {
    let time = if k < head_ms.len() { head_ms[k] } else { song_ms };
    let upper = (if k < head_ms.len() { head_ms[k] } else { song_ms.ceil() }) - 1.0;
    if code == 1 { time } else { clip(round(gap), head_ms[k - 1] + 1.0, upper) }
}

/// What row `k` adds to the chart, given the lanes held on entry to it: the ends of the holds it closes, which lie in
/// the gap before the row or on it, then its taps and hold starts. The tokens of every row, in order, are those of
/// [`chart_tokens`] for [`chart_from_decisions`].
pub fn row_tokens(head_ms: &[f64], song_ms: f64, k: usize, codes: [i32; 4], gap: [f64; 4], held: [i32; 4]) -> Vec<Token> {
    let time = if k < head_ms.len() { head_ms[k] } else { song_ms };
    let mut tokens: Vec<Token> = (0..4).filter(|&lane| held[lane] != 0 && codes[lane] != 0).map(|lane| Token { lane, time_ms: hold_end(head_ms, song_ms, k, codes[lane], gap[lane]), kind: TokenKind::HoldEnd }).collect();
    tokens.sort_by(|a, b| a.time_ms.total_cmp(&b.time_ms).then(a.lane.cmp(&b.lane)));
    for lane in 0..4 {
        let (code, was_held) = (codes[lane], held[lane] != 0);
        let tap = if was_held { code == 3 } else { code == 1 };
        let hold = if was_held { code == 4 } else { code == 2 };
        if tap || hold {
            tokens.push(Token { lane, time_ms: time, kind: if hold { TokenKind::HoldStart } else { TokenKind::Tap } });
        }
    }
    tokens
}

/// A chart as tokens, in a page's order.
pub fn chart_tokens(chart: &Chart) -> Vec<Token> {
    let mut tokens = Vec::new();
    for o in &chart.objects {
        tokens.push(Token { lane: o.lane, time_ms: o.start_ms, kind: if o.end_ms.is_some() { TokenKind::HoldStart } else { TokenKind::Tap } });
        if let Some(end) = o.end_ms {
            tokens.push(Token { lane: o.lane, time_ms: end, kind: TokenKind::HoldEnd });
        }
    }
    tokens.sort_by(|a, b| a.time_ms.total_cmp(&b.time_ms).then((a.kind != TokenKind::HoldEnd).cmp(&(b.kind != TokenKind::HoldEnd))).then(a.lane.cmp(&b.lane)));
    tokens
}

pub fn chart_from_decisions(head_ms: &[f64], song_ms: f64, actions: &[[i32; 4]], gap: &[[f64; 4]]) -> Chart {
    let mut objects: Vec<ChartObject> = Vec::new();
    let mut open: [Option<usize>; 4] = [None; 4];
    for k in 0..actions.len() {
        let time = if k < head_ms.len() { head_ms[k] } else { song_ms };
        for lane in 0..4 {
            let code = actions[k][lane];
            let was_held = open[lane].is_some();
            if let Some(hold) = open[lane]
                && code != 0
            {
                objects[hold].end_ms = Some(hold_end(head_ms, song_ms, k, code, gap[k][lane]));
                open[lane] = None;
            }
            let tap = if was_held { code == 3 } else { code == 1 };
            let hold = if was_held { code == 4 } else { code == 2 };
            if tap || hold {
                objects.push(ChartObject { lane, start_ms: time, end_ms: if hold { Some(f64::NAN) } else { None } });
                if hold {
                    open[lane] = Some(objects.len() - 1);
                }
            }
        }
    }
    objects.sort_by(|a, b| crate::fitter::order(a.start_ms - b.start_ms).then(a.lane.cmp(&b.lane)));
    Chart { duration_ms: round(song_ms), objects }
}

pub fn validate_chart(chart: &Chart) -> Result<(), String> {
    let integer = |x: f64| x.is_finite() && x.fract() == 0.0;
    if !integer(chart.duration_ms) || chart.duration_ms < 0.0 {
        return Err("Chart duration must be nonnegative whole milliseconds".to_string());
    }
    let mut end = [-1.0f64; 4];
    let mut previous: Option<&ChartObject> = None;
    for object in &chart.objects {
        let (lane, start) = (object.lane, object.start_ms);
        if !integer(start) || start < 0.0 || start > chart.duration_ms {
            return Err(format!("Invalid chart object: lane {lane} start {}", number_string(start)));
        }
        if previous.is_some_and(|p| start < p.start_ms || (start == p.start_ms && lane <= p.lane)) {
            return Err("Chart objects must be sorted by start then lane, with no duplicate heads".to_string());
        }
        if start < end[lane] {
            return Err(format!("Head inside an open hold in lane {lane} at {}", number_string(start)));
        }
        if let Some(release) = object.end_ms
            && (!integer(release) || release <= start || release > chart.duration_ms)
        {
            return Err(format!("Invalid hold release in lane {lane} at {}", number_string(start)));
        }
        end[lane] = object.end_ms.unwrap_or(start);
        previous = Some(object);
    }
    Ok(())
}

/// `audio.<extension>`: the extension of the original file name, lowercased, or `bin`.
pub fn audio_filename(name: &str) -> String {
    let ext = name.rsplit_once('.').map(|(_, e)| e).filter(|e| !e.is_empty() && e.chars().all(|c| c.is_ascii_alphanumeric())).map(str::to_lowercase);
    format!("audio.{}", ext.as_deref().unwrap_or("bin"))
}

fn metadata(value: &str) -> String {
    value.replace(['\r', '\n'], " ")
}

fn python_float(x: f64) -> String {
    if x.fract() == 0.0 { format!("{}.0", number_string(x)) } else { number_string(x) }
}

pub struct OsuOptions<'a> {
    pub title: &'a str,
    pub artist: &'a str,
    pub setting: &'a str,
    pub seed: String,
    pub audio_name: &'a str,
    pub bookmarks: &'a [f64],
    pub random_algorithm: &'a str,
}

pub fn osu_text(chart: &Chart, segments: &[Segment], options: &OsuOptions) -> String {
    let mut lines: Vec<String> = vec![
        "osu file format v14".into(),
        "[General]".into(),
        format!("AudioFilename:{}", options.audio_name),
        "Mode:3".into(),
        "[Metadata]".into(),
        format!("Title:{}", metadata(options.title)),
        format!("Artist:{}", metadata(options.artist)),
        "Creator:Ensomi".into(),
        format!("Version:Ensomi {} s{}", options.setting, options.seed),
        "BeatmapID:0".into(),
        "BeatmapSetID:-1".into(),
    ];
    if !options.bookmarks.is_empty() {
        lines.push("[Editor]".into());
        lines.push(format!("Bookmarks:{}", options.bookmarks.iter().map(|b| number_string(b.trunc())).collect::<Vec<_>>().join(",")));
    }
    lines.push("[Difficulty]".into());
    lines.push("CircleSize:4".into());
    lines.push("[TimingPoints]".into());
    for segment in segments {
        lines.push(format!("{},{},4,2,0,100,1,0", number_string(round(segment.offset_ms)), python_float(segment.beat_length_ms)));
    }
    lines.push(format!("// {} {} seed={} random={}", crate::RUNTIME_NAME, crate::RUNTIME_VERSION, options.seed, options.random_algorithm));
    lines.push("[HitObjects]".into());
    for object in &chart.objects {
        let prefix = format!("{},192,{}", 64 + 128 * object.lane, number_string(object.start_ms));
        lines.push(match object.end_ms {
            None => format!("{prefix},1,0,0:0:0:0:"),
            Some(end) => format!("{prefix},128,0,{}:0:0:0:0:", number_string(end)),
        });
    }
    lines.join("\n") + "\n"
}
