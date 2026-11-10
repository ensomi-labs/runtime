//! Whole sessions against a synthetic song. The song is a function of time, so that the reference and the microphone can
//! be rendered at different rates from the same sound, and the true position of a query is known exactly and owes
//! nothing to the code under test.

use ensomi_sync::{Reference, Snapshot, State, WithholdReason};
use std::f64::consts::PI;
use std::sync::OnceLock;

/// The largest error of a locked position against the truth, a third of a hop.
const TOLERANCE_MS: f64 = 3.5;
const CLOCK_ORIGIN_MS: f64 = 7_654_321.5;

struct Lcg(u32);

impl Lcg {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        f64::from(self.0 >> 8) / f64::from(1_u32 << 24)
    }
}

struct Note {
    start: f64,
    length: f64,
    hertz: f64,
    level: f64,
}

struct Song(Vec<Note>);

impl Song {
    /// Chords with changing rhythm, and bright hits, which set the spectrum moving as music does.
    fn new(seed: u32, seconds: f64) -> Self {
        let mut random = Lcg(seed);
        let (mut notes, mut time) = (Vec::new(), 0.0);
        while time < seconds {
            for _ in 0..3 {
                let hertz = 110.0 * 2f64.powf(random.next() * 4.0);
                notes.push(Note { start: time, length: 0.15 + random.next() * 0.45, hertz, level: 0.08 + random.next() * 0.14 });
            }
            if random.next() < 0.5 {
                notes.push(Note { start: time, length: 0.06, hertz: 1_500.0 + random.next() * 5_000.0, level: 0.15 });
            }
            time += 0.09 + random.next() * 0.28;
        }
        Self(notes)
    }

    /// `seconds` of the song from `from`, at `rate`.
    fn render(&self, rate: f64, from: f64, seconds: f64) -> Vec<f32> {
        let mut out = vec![0f32; (seconds * rate) as usize];
        for note in &self.0 {
            let (begin, end) = (((note.start - from) * rate).ceil().max(0.0) as usize, (((note.start + note.length - from) * rate).ceil().max(0.0) as usize).min(out.len()));
            for (i, sample) in out.iter_mut().enumerate().take(end).skip(begin) {
                let t = from + i as f64 / rate - note.start;
                let envelope = (-5.0 * t / note.length).exp() * (t * 400.0).min(1.0);
                let tone = (2.0 * PI * note.hertz * t).sin() + 0.5 * (4.0 * PI * note.hertz * t).sin() + 0.25 * (6.0 * PI * note.hertz * t).sin();
                *sample += (note.level * envelope * tone) as f32;
            }
        }
        out
    }
}

/// A song's first 40 seconds at 44.1 kHz.
fn song() -> &'static (Song, Reference) {
    static SONG: OnceLock<(Song, Reference)> = OnceLock::new();
    SONG.get_or_init(|| {
        let song = Song::new(7, 40.0);
        let reference = Reference::from_pcm(&song.render(44_100.0, 0.0, 40.0), 44_100.0).unwrap();
        (song, reference)
    })
}

/// A room: quieter than the recording, with noise at 6 dB below the signal.
fn heard(mut pcm: Vec<f32>, seed: u32) -> Vec<f32> {
    let rms = (pcm.iter().map(|sample| f64::from(*sample).powi(2)).sum::<f64>() / pcm.len() as f64).sqrt();
    // Uniform noise of width w has an RMS of w / sqrt(12).
    let width = (rms * 0.5 * 12f64.sqrt()) as f32;
    let mut random = Lcg(seed);
    for sample in &mut pcm {
        *sample = (*sample + (random.next() as f32 - 0.5) * width) * 0.4;
    }
    pcm
}

/// Pushes `pcm` in blocks of `block_ms`, each stamped by `stamp` from its true capture time, until a final lock.
fn listen(reference: &Reference, pcm: &[f32], rate: f64, block_ms: f64, stamp: impl Fn(usize, f64) -> f64) -> Option<(Snapshot, f64)> {
    let mut session = reference.session(rate).unwrap();
    let size = (rate * block_ms / 1_000.0) as usize;
    for (index, block) in pcm.chunks(size).enumerate() {
        let time = index as f64 * size as f64 / rate * 1_000.0;
        let snapshot = session.push(block, stamp(index, CLOCK_ORIGIN_MS + time)).unwrap();
        if snapshot.is_final() {
            return Some((snapshot, time));
        }
    }
    None
}

fn error_ms(snapshot: &Snapshot, offset_s: f64) -> f64 {
    let position = snapshot.position.unwrap();
    position.reference_time_ms - (offset_s * 1_000.0 + position.capture_time_ms - CLOCK_ORIGIN_MS)
}

#[test]
fn locks_on_a_noisy_excerpt_at_its_known_position() {
    let (song, reference) = song();
    let (offset, rate) = (12.345, 48_000.0);
    let pcm = heard(song.render(rate, offset, 10.0), 1);
    let (snapshot, heard_ms) = listen(reference, &pcm, rate, 20.0, |_, time| time).expect("a final lock");
    assert!(error_ms(&snapshot, offset).abs() <= TOLERANCE_MS, "{}", error_ms(&snapshot, offset));
    // At 2.5 s Sonalign's first search can run; a lock needs another second of fresh audio.
    assert!((3_400.0..=6_000.0).contains(&heard_ms), "{heard_ms}");
    assert!(snapshot.confidence > 0.25, "{}", snapshot.confidence);
    assert_eq!(snapshot.state, State::Locked);
    assert_eq!(snapshot.withhold_reason, None);
}

#[test]
fn a_microphone_at_another_rate_locks_at_the_same_place() {
    let (song, reference) = song();
    let offset = 21.0;
    for rate in [16_000.0, 44_100.0, 96_000.0] {
        let pcm = heard(song.render(rate, offset, 10.0), 2);
        let (snapshot, _) = listen(reference, &pcm, rate, 20.0, |_, time| time).expect("a final lock");
        assert!(error_ms(&snapshot, offset).abs() <= TOLERANCE_MS, "{rate} Hz: {}", error_ms(&snapshot, offset));
    }
}

#[test]
fn the_position_is_anchored_to_capture_and_not_to_when_blocks_arrive() {
    let (song, reference) = song();
    let (offset, rate) = (5.5, 48_000.0);
    let pcm = heard(song.render(rate, offset, 10.0), 3);
    // Delivered in bursts of 60 ms: stamped on arrival, the blocks of a burst run 40, 20 and 0 ms late.
    let bursts = |index: usize, time: f64| time + [40.0, 20.0, 0.0][index % 3];
    let (snapshot, _) = listen(reference, &pcm, rate, 20.0, bursts).expect("a final lock");
    assert!(error_ms(&snapshot, offset).abs() <= TOLERANCE_MS, "{}", error_ms(&snapshot, offset));
    // A first block stamped 80 ms late, as a recording starting up may be, does not stay in the timeline.
    let start = |index: usize, time: f64| if index == 0 { time + 80.0 } else { time };
    let (snapshot, _) = listen(reference, &pcm, rate, 20.0, start).expect("a final lock");
    assert!(error_ms(&snapshot, offset).abs() <= TOLERANCE_MS, "{}", error_ms(&snapshot, offset));
}

#[test]
fn block_size_does_not_matter() {
    let (song, reference) = song();
    let (offset, rate) = (17.0, 48_000.0);
    let pcm = heard(song.render(rate, offset, 8.0), 4);
    for block_ms in [5.0, 50.0, 200.0] {
        let (snapshot, _) = listen(reference, &pcm, rate, block_ms, |_, time| time).expect("a final lock");
        assert!(error_ms(&snapshot, offset).abs() <= TOLERANCE_MS, "{block_ms} ms: {}", error_ms(&snapshot, offset));
    }
}

#[test]
fn silence_is_not_a_lock() {
    let (_, reference) = song();
    let mut session = reference.session(48_000.0).unwrap();
    let silence = vec![0.0; 960];
    let mut last = None;
    for block in 0..500 {
        let snapshot = session.push(&silence, block as f64 * 20.0).unwrap();
        assert!(!snapshot.is_final());
        assert_eq!(snapshot.position, None);
        last = Some(snapshot);
    }
    let last = last.unwrap();
    assert_eq!(last.state, State::Listening);
    assert_eq!(last.withhold_reason, Some(WithholdReason::InsufficientEnergy));
}

#[test]
fn another_song_is_not_a_lock() {
    let (_, reference) = song();
    let pcm = heard(Song::new(99, 16.0).render(48_000.0, 0.0, 15.0), 5);
    assert!(listen(reference, &pcm, 48_000.0, 20.0, |_, time| time).is_none());
}

#[test]
fn a_gap_drops_the_lock_and_the_position_after_it_is_right() {
    let (song, reference) = song();
    let (offset, rate) = (8.0, 48_000.0);
    let mut session = reference.session(rate).unwrap();
    let first = heard(song.render(rate, offset, 5.0), 6);
    let mut locked = false;
    for (index, block) in first.chunks(960).enumerate() {
        locked |= session.push(block, CLOCK_ORIGIN_MS + index as f64 * 20.0).unwrap().is_final();
    }
    assert!(locked);
    // Three seconds of sound never arrived. The song goes on meanwhile.
    let resume_s = 8.0;
    let second = heard(song.render(rate, offset + resume_s, 8.0), 7);
    let (mut lost, mut relocked) = (false, None);
    for (index, block) in second.chunks(960).enumerate() {
        let snapshot = session.push(block, CLOCK_ORIGIN_MS + resume_s * 1_000.0 + index as f64 * 20.0).unwrap();
        lost |= snapshot.state == State::Lost;
        if snapshot.is_final() && lost {
            relocked = Some(snapshot);
            break;
        }
    }
    let snapshot = relocked.expect("a second lock");
    assert!(lost);
    assert!(error_ms(&snapshot, offset).abs() <= TOLERANCE_MS, "{}", error_ms(&snapshot, offset));
}

#[test]
fn refused_blocks_change_nothing() {
    let (song, reference) = song();
    let pcm = heard(song.render(48_000.0, 3.0, 6.0), 8);
    let mut session = reference.session(48_000.0).unwrap();
    let mut stamped = 0.0;
    for block in pcm.chunks(960) {
        assert!(session.push(&[0.0, f32::NAN], stamped).is_err());
        assert!(session.push(block, f64::NAN).is_err());
        assert!(!session.push(&[], stamped).unwrap().is_final());
        let snapshot = session.push(block, stamped).unwrap();
        if snapshot.is_final() {
            return;
        }
        stamped += 20.0;
    }
    panic!("the session never locked");
}

#[test]
fn a_song_too_short_to_index_is_refused() {
    assert_eq!(Reference::from_pcm(&[0.1; 1_023], 48_000.0).err(), Some(ensomi_sync::Error::TooShort));
    assert_eq!(Reference::from_pcm(&[0.1; 1_024], 48_000.0).err(), Some(ensomi_sync::Error::TooShort));
    assert!(Reference::from_pcm(&[0.1; 1_536], 48_000.0).is_ok());
}
