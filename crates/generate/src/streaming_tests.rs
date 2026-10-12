//! Streaming v1 without the model: pages per head row against the whole chart, the join and start rules, the
//! scheduler's play bookkeeping, and the stored form of `Prepared`.
use crate::export::{chart_from_decisions, chart_tokens, row_tokens, validate_chart, Chart, Page, Token, TokenKind};
use crate::fitter::Segment;
use crate::math::round;
use crate::prepared::Prepared;
use crate::schedule::{can_start, join_point, Action, Lead, Stream};
use ensomi_gameplay::{ChartToken, Judge, Ruleset, Session};

/// A small linear congruential generator, so the decisions are the same on every run.
struct Lcg(u64);

impl Lcg {
    fn below(&mut self, n: u64) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 33) % n
    }
}

/// Head times, song length, and per row the lane codes and gap releases.
type Decisions = (Vec<f64>, f64, Vec<[i32; 4]>, Vec<[f64; 4]>);

/// Decisions as the arrangement makes them, `rows` head rows and the final row: per lane, from free a tap (1) or a
/// hold (2), from held the end on the row (1), in the gap (2), in the gap and a tap (3) or a new hold (4); the final
/// row ends every hold in the gap. Gap releases sometimes fall outside the gap, to be clipped.
fn decisions(seed: u64, rows: usize) -> Decisions {
    let mut random = Lcg(seed);
    let mut head_ms = Vec::new();
    let mut t = 400.0;
    for _ in 0..rows {
        t += 40.0 + random.below(400) as f64;
        head_ms.push(t);
    }
    let song_ms = t + 300.5;
    let (mut actions, mut gaps) = (Vec::new(), Vec::new());
    let mut held = [false; 4];
    for k in 0..=rows {
        let (mut codes, mut gap) = ([0; 4], [f64::NAN; 4]);
        for lane in 0..4 {
            let code = if k == rows { if held[lane] { 2 } else { 0 } } else if held[lane] { random.below(5) as i32 } else { [0, 0, 0, 1, 2][random.below(5) as usize] };
            if held[lane] && code >= 2 {
                let (a, b) = (head_ms[k - 1], if k < rows { head_ms[k] } else { song_ms + 100.0 });
                gap[lane] = a - 30.0 + random.below((b - a + 60.0) as u64 * 2) as f64 / 2.0;
            }
            held[lane] = if held[lane] { code == 0 || code == 4 } else { code == 2 };
            codes[lane] = code;
        }
        actions.push(codes);
        gaps.push(gap);
    }
    (head_ms, song_ms, actions, gaps)
}

fn chart_token(token: &Token) -> ChartToken {
    let kind = match token.kind {
        TokenKind::Tap => ensomi_gameplay::TokenKind::Tap,
        TokenKind::HoldStart => ensomi_gameplay::TokenKind::HoldStart,
        TokenKind::HoldEnd => ensomi_gameplay::TokenKind::HoldEnd,
    };
    ChartToken { lane: token.lane as u32, time_ms: token.time_ms, kind }
}

/// The pages the arrangement commits for these decisions: one per row, covering the row's head time, the last one
/// the song.
fn pages(head_ms: &[f64], song_ms: f64, actions: &[[i32; 4]], gap: &[[f64; 4]]) -> Vec<Page> {
    let mut held = [0; 4];
    let mut pages = Vec::new();
    for k in 0..actions.len() {
        let last = k == head_ms.len();
        pages.push(Page { tokens: row_tokens(head_ms, song_ms, k, actions[k], gap[k], held), coverage_ms: if last { round(song_ms) } else { head_ms[k] }, end_of_stream: last });
        held = std::array::from_fn(|l| {
            let code = actions[k][l];
            if held[l] != 0 { (code == 0 || code == 4) as i32 } else { (code == 2) as i32 }
        });
    }
    pages
}

#[test]
fn pages_per_head_row_make_the_whole_chart() {
    for seed in 0..40 {
        let (head_ms, song_ms, actions, gap) = decisions(seed, 60);
        let chart = chart_from_decisions(&head_ms, song_ms, &actions, &gap);
        validate_chart(&chart).unwrap();
        let pages = pages(&head_ms, song_ms, &actions, &gap);

        // Concatenated, the pages are the whole chart's tokens in order: hold ends of a row before its starts.
        let tokens: Vec<Token> = pages.iter().flat_map(|p| p.tokens.clone()).collect();
        assert_eq!(tokens, chart_tokens(&chart), "seed {seed}");

        // A streamed session takes every page, a hold's end in a later page than its start, and ends with the chart.
        let mut session = Session::streaming(Ruleset { judge: Judge::C });
        for page in &pages {
            let tokens: Vec<ChartToken> = page.tokens.iter().map(chart_token).collect();
            session.append_tokens(&tokens, page.coverage_ms, page.end_of_stream).unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        }
        let objects: Vec<(usize, f64, Option<f64>)> = session.visible_objects(f64::NEG_INFINITY, f64::INFINITY).iter().map(|o| (o.lane as usize, o.start_time_ms, o.end_time_ms)).collect();
        let expected: Vec<(usize, f64, Option<f64>)> = chart.objects.iter().map(|o| (o.lane, o.start_ms, o.end_ms)).collect();
        assert_eq!(objects, expected, "seed {seed}");
        assert!(tokens.iter().any(|t| t.kind == TokenKind::HoldEnd), "seed {seed} has no hold");
    }
}

#[test]
fn a_hold_end_comes_in_the_page_of_the_row_that_closes_it() {
    // As the sample chart of tests.rs: lane 1 holds from row 0 and is closed on row 1, lane 2 holds from row 0 and is
    // closed in the gap before row 2, with a tap on that row.
    let head_ms = [1000.0, 1500.0, 2000.0];
    let actions = [[1, 2, 2, 0], [0, 1, 0, 1], [1, 0, 3, 0], [0, 0, 0, 0]];
    let mut gap = [[f64::NAN; 4]; 4];
    gap[2][2] = 1740.4;
    let pages = pages(&head_ms, 2600.0, &actions, &gap);
    let token = |lane, time_ms, kind| Token { lane, time_ms, kind };
    use TokenKind::{HoldEnd, HoldStart, Tap};
    assert_eq!(pages[0].tokens, [token(0, 1000.0, Tap), token(1, 1000.0, HoldStart), token(2, 1000.0, HoldStart)]);
    assert_eq!(pages[1].tokens, [token(1, 1500.0, HoldEnd), token(3, 1500.0, Tap)]);
    assert_eq!(pages[2].tokens, [token(2, 1740.0, HoldEnd), token(0, 2000.0, Tap), token(2, 2000.0, Tap)]);
    assert_eq!((pages[3].tokens.len(), pages[3].coverage_ms, pages[3].end_of_stream), (0, 2600.0, true));
    assert_eq!(chart_from_decisions(&head_ms, 2600.0, &actions, &gap).objects.len(), 6);
}

fn tap(lane: usize, time_ms: f64) -> Token {
    Token { lane, time_ms, kind: TokenKind::Tap }
}

fn start(lane: usize, time_ms: f64) -> Token {
    Token { lane, time_ms, kind: TokenKind::HoldStart }
}

fn end(lane: usize, time_ms: f64) -> Token {
    Token { lane, time_ms, kind: TokenKind::HoldEnd }
}

#[test]
fn the_join_point_is_the_first_time_no_hold_is_open() {
    // Lane 1 holds over 1000 to 2000 and lane 2 over 1800 to 2600.
    let tokens = [start(1, 1000.0), start(2, 1800.0), end(1, 2000.0), tap(0, 2200.0), end(2, 2600.0), tap(0, 2600.0)];
    // Outside every hold: the time asked for.
    assert_eq!(join_point(&tokens, 3000.0, 700.0), Some(700.0));
    assert_eq!(join_point(&tokens, 3000.0, 2800.0), Some(2800.0));
    // Inside a hold: its end, then the end of a hold open there, and so on. A hold that ends at a time is not open
    // at it.
    assert_eq!(join_point(&tokens, 3000.0, 1500.0), Some(2600.0));
    assert_eq!(join_point(&tokens[..4], 3000.0, 2600.0), None);
    // Undecided while a hold open there has no end yet, or while the chart does not reach the candidate.
    assert_eq!(join_point(&tokens[..4], 2200.0, 1500.0), None);
    assert_eq!(join_point(&tokens, 2500.0, 1500.0), None);
    assert_eq!(join_point(&[], 1000.0, 1500.0), None);
    assert_eq!(join_point(&[], f64::INFINITY, 1500.0), Some(1500.0));
}

#[test]
fn the_start_rule_asks_more_of_a_device_slower_than_playback() {
    // Faster than playback (r = 4): the lead alone.
    assert!(can_start(1500.0, 1000.0, 60_000.0, 500.0, 1.0, 4.0));
    assert!(!can_start(1499.0, 1000.0, 60_000.0, 500.0, 1.0, 4.0));
    // Half as fast as playback, with 9750 ms of the song left at c0 = 10250: (E − c0)(p/r − 1) = 9750 more.
    assert!(can_start(10_250.0, 0.0, 20_000.0, 500.0, 1.0, 0.5));
    assert!(!can_start(10_249.0, 0.0, 20_000.0, 500.0, 1.0, 0.5));
    // Played at 1.5 times its speed on a device with r = 1: c0 ≥ 500 + (20000 − c0) / 2.
    assert!(can_start(7_000.0, 0.0, 20_000.0, 500.0, 1.5, 1.0));
    assert!(!can_start(6_999.0, 0.0, 20_000.0, 500.0, 1.5, 1.0));
}

fn page(tokens: Vec<Token>, coverage_ms: f64) -> Page {
    Page { tokens, coverage_ms, end_of_stream: false }
}

fn committed(action: Option<Action>) -> Page {
    match action {
        Some(Action::Commit(page)) => page,
        other => panic!("not a page: {other:?}"),
    }
}

/// L = max(300, 450) + 50 = 500 ms; the song ends at 10 s.
fn stream() -> Stream {
    let mut stream = Stream::new(Lead { settle_ms: 300.0, render_ms: 450.0, margin_ms: 50.0 });
    stream.end_ms = Some(10_000.0);
    stream
}

#[test]
fn play_from_the_start_is_ready_once_the_rule_holds() {
    let mut s = stream();
    s.start(0.0);
    assert!(s.next().is_none());
    s.commit(page(vec![tap(0, 300.0)], 300.0), 0.0);
    assert_eq!(committed(s.next()).tokens, [tap(0, 300.0)]);
    // One page gives no throughput yet.
    assert!(s.next().is_none());
    s.commit(page(vec![tap(1, 600.0)], 600.0), 100.0);
    assert_eq!(committed(s.next()).coverage_ms, 600.0);
    assert_eq!(s.rate(), Some(3.0));
    assert!(matches!(s.next(), Some(Action::Ready { join_ms: 0.0 })));
    assert!(s.next().is_none());

    // Play under way: rest while the chart stays far enough ahead of what is heard.
    s.heard(0.0, 1000.0);
    s.heard(50.0, 1050.0);
    assert!(s.idle());
    s.heard(150.0, 1150.0);
    assert!(!s.idle());
}

#[test]
fn the_chart_falling_behind_withdraws_ready_until_the_rule_holds_again() {
    let mut s = stream();
    s.start(0.0);
    s.commit(page(vec![], 1000.0), 0.0);
    s.commit(page(vec![], 2000.0), 100.0);
    while !matches!(s.next(), Some(Action::Ready { .. }) | None) {}
    // Heard 1800 with the chart through 2000: less than the settle lead left.
    s.heard(1800.0, 0.0);
    assert!(!s.idle());
    s.commit(page(vec![], 2200.0), 200.0);
    committed(s.next());
    assert!(s.next().is_none());
    s.commit(page(vec![], 2400.0), 300.0);
    committed(s.next());
    assert!(matches!(s.next(), Some(Action::Ready { join_ms: 1800.0 })));
}

#[test]
fn play_at_a_position_joins_where_no_hold_is_open_and_drops_what_is_before() {
    let mut s = stream();
    // Asked for 2500: the join point is the first clean time from 3000. Lane 1 holds from 2800, so nothing goes to
    // the session until that hold's end is known.
    s.start(2500.0);
    s.commit(page(vec![tap(0, 1000.0)], 1000.0), 0.0);
    s.commit(page(vec![start(1, 2800.0)], 2800.0), 50.0);
    s.commit(page(vec![tap(0, 3000.0)], 3000.0), 100.0);
    assert!(s.next().is_none());
    s.commit(page(vec![end(1, 3400.0), tap(2, 3400.0)], 3400.0), 150.0);
    let first = committed(s.next());
    assert_eq!((first.tokens, first.coverage_ms), (vec![tap(2, 3400.0)], 3400.0));
    assert!(s.next().is_none());
    s.commit(page(vec![start(3, 4000.0)], 4000.0), 200.0);
    assert_eq!(committed(s.next()).tokens, [start(3, 4000.0)]);
    assert!(matches!(s.next(), Some(Action::Ready { join_ms: 3400.0 })));

    // Starting again sends what was committed from the new join point on, for a new session.
    s.commit(page(vec![end(3, 4500.0)], 4500.0), 250.0);
    s.start(0.0);
    let again = committed(s.next());
    assert_eq!(again.tokens.len(), 7);
    assert_eq!(again.coverage_ms, 4500.0);
}

#[test]
fn the_whole_chart_is_ready_when_it_ends() {
    let mut s = stream();
    s.start(0.0);
    s.commit(Page { tokens: vec![tap(0, 500.0)], coverage_ms: 10_000.0, end_of_stream: true }, 900.0);
    assert!(committed(s.next()).end_of_stream);
    assert!(matches!(s.next(), Some(Action::Ready { join_ms: 0.0 })));
    assert!(!s.idle());
}

fn prepared() -> Prepared {
    let features = (0..260).map(|i| if i == 7 { -0.0 } else { (i as f32 - 100.0) / 3.0 }).collect();
    Prepared { features, segments: vec![Segment { offset_ms: -12.5, beat_length_ms: 444.444_444_444_444_46 }, Segment { offset_ms: 30_978.0, beat_length_ms: 500.0 }], song_ms: 61_234.567_891 }
}

#[test]
fn prepared_bytes_round_trip_bit_for_bit() {
    let original = prepared();
    let bytes = original.to_bytes();
    let header = u64::from_le_bytes(bytes[..8].try_into().unwrap()) as usize;
    assert_eq!(header % 8, 0);
    assert_eq!(bytes.len(), 8 + header + 260 * 4 + 4 * 8 + 8);
    let read = Prepared::from_bytes(&bytes).unwrap();
    assert_eq!(read.features.iter().map(|x| x.to_bits()).collect::<Vec<_>>(), original.features.iter().map(|x| x.to_bits()).collect::<Vec<_>>());
    let segments = |p: &Prepared| p.segments.iter().map(|s| (s.offset_ms.to_bits(), s.beat_length_ms.to_bits())).collect::<Vec<_>>();
    assert_eq!(segments(&read), segments(&original));
    assert_eq!(read.song_ms.to_bits(), original.song_ms.to_bits());
}

#[test]
fn prepared_bytes_of_another_format_runtime_or_shape_are_refused() {
    let bytes = prepared().to_bytes();
    // The header with one text replaced by another of the same length.
    let replaced = |from: &str, to: &str| {
        let at = bytes.windows(from.len()).position(|w| w == from.as_bytes()).unwrap();
        let mut changed = bytes.clone();
        changed[at..at + to.len()].copy_from_slice(to.as_bytes());
        changed
    };
    assert!(Prepared::from_bytes(&replaced("ensomi-prepared-v1", "ensomi-prepared-v9")).unwrap_err().contains("not an ensomi-prepared-v1"));
    let version = format!("\"runtime\":\"{}\"", crate::RUNTIME_VERSION);
    let other: String = version.chars().map(|c| if c.is_ascii_digit() { '9' } else { c }).collect();
    assert!(Prepared::from_bytes(&replaced(&version, &other)).unwrap_err().contains("made by runtime"));
    assert!(Prepared::from_bytes(&replaced("[2,130]", "[4, 65]")).is_err());
    assert!(Prepared::from_bytes(&bytes[..bytes.len() - 1]).is_err());
    assert!(Prepared::from_bytes(&bytes[..12]).is_err());
    assert!(Prepared::from_bytes(&[]).is_err());
}

#[test]
fn chart_tokens_put_a_hold_end_before_a_start_at_the_same_time() {
    let chart = Chart { duration_ms: 3000.0, objects: vec![crate::ChartObject { lane: 1, start_ms: 1000.0, end_ms: Some(2000.0) }, crate::ChartObject { lane: 0, start_ms: 2000.0, end_ms: None }, crate::ChartObject { lane: 1, start_ms: 2000.0, end_ms: None }] };
    assert_eq!(chart_tokens(&chart), [start(1, 1000.0), end(1, 2000.0), tap(0, 2000.0), tap(1, 2000.0)]);
}
