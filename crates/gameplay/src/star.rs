//! osu!mania difficulty version 20241007, ported from ensomi-model r2-1.0.1's
//! `osu_core/difficulty.py`. The legacy unstable sort is part of the calculator.
use crate::ChartObject;

/// Measures four-key objects at clock rate 1, in whole audio milliseconds.
/// Objects must have lanes 0 to 3. Input order among simultaneous objects matters to the legacy sort.
/// Empty and one-note charts have rating zero. This does not depend on generation or chart duration.
pub fn star_rating(objects: &[ChartObject]) -> f64 {
    let mut objects = objects.to_vec();
    if objects.len() < 2 {
        return 0.0;
    }
    let last = objects.len() as isize - 1;
    legacy_sort(&mut objects, 0, last, 32);

    let mut previous: [Option<ChartObject>; 4] = [None; 4];
    let mut individual = [0.0; 4];
    let mut highest: f64 = 0.0;
    let mut overall = 1.0;
    let mut current_strain = 0.0;
    let mut section_end = (objects[1].start_ms as f64 / 400.0).ceil() * 400.0;
    let mut peak: f64 = 0.0;
    let mut peaks = Vec::new();

    // The first raw object only supplies delta time. It is never a previous per-column object.
    for i in 1..objects.len() {
        let object = objects[i];
        let start = object.start_ms as f64;
        let end = object.end_ms.unwrap_or(object.start_ms) as f64;
        let delta = start - objects[i - 1].start_ms as f64;
        while start > section_end {
            peaks.push(peak);
            peak = if i == 1 {
                0.0
            } else {
                let elapsed = section_end - objects[i - 1].start_ms as f64;
                decay(highest, elapsed, 0.125) + decay(overall, elapsed, 0.30)
            };
            section_end += 400.0;
        }

        let mut hold_factor = 1.0;
        let mut overlapping = false;
        let mut closest_end = (end - start).abs();
        for prev in previous.iter().flatten() {
            let prev_start = prev.start_ms as f64;
            let prev_end = prev.end_ms.unwrap_or(prev.start_ms) as f64;
            if prev_end - 1.0 > end && start - 1.0 > prev_start {
                hold_factor = 1.25;
            }
            overlapping |=
                prev_end - 1.0 > start && end - 1.0 > prev_end && start - 1.0 > prev_start;
            closest_end = closest_end.min((end - prev_end).abs());
        }
        let hold_addition = if overlapping {
            1.0 / (1.0 + (0.27 * (30.0 - closest_end)).exp())
        } else {
            0.0
        };
        let column = object.lane as usize;
        let column_delta = start - previous[column].map_or(0.0, |prev| prev.start_ms as f64);
        individual[column] = decay(individual[column], column_delta, 0.125);
        individual[column] += 2.0 * hold_factor;
        highest = if delta <= 1.0 {
            highest.max(individual[column])
        } else {
            individual[column]
        };
        overall = decay(overall, delta, 0.30);
        overall += (1.0 + hold_addition) * hold_factor;
        // Keep the reference's StrainDecaySkill arithmetic, including the subtraction and addition.
        current_strain += highest + overall - current_strain;
        peak = peak.max(current_strain);
        previous[column] = Some(object);
    }
    peaks.push(peak);
    peaks.retain(|&value| value > 0.0);
    peaks.sort_by(|a, b| b.total_cmp(a));
    let mut weight = 1.0;
    let mut difficulty = 0.0;
    for strain in peaks {
        difficulty += strain * weight;
        weight *= 0.9;
    }
    difficulty * 0.018
}

fn decay(value: f64, ms: f64, base: f64) -> f64 {
    value * base.powf(ms / 1000.0)
}

// The reference's depth-limited quicksort, including its heapsort fallback. A stable sort or Rust's
// unstable sort produces a different order for equal start times, changing long-note strain.
fn swap_if_greater(keys: &mut [ChartObject], a: isize, b: isize) {
    if a != b && keys[a as usize].start_ms > keys[b as usize].start_ms {
        keys.swap(a as usize, b as usize);
    }
}

fn down_heap(keys: &mut [ChartObject], mut i: isize, n: isize, lo: isize) {
    let value = keys[(lo + i - 1) as usize];
    while i <= n / 2 {
        let mut child = 2 * i;
        if child < n
            && keys[(lo + child - 1) as usize].start_ms < keys[(lo + child) as usize].start_ms
        {
            child += 1;
        }
        if value.start_ms >= keys[(lo + child - 1) as usize].start_ms {
            break;
        }
        keys[(lo + i - 1) as usize] = keys[(lo + child - 1) as usize];
        i = child;
    }
    keys[(lo + i - 1) as usize] = value;
}

fn heap_sort(keys: &mut [ChartObject], lo: isize, hi: isize) {
    let n = hi - lo + 1;
    for i in (1..=n / 2).rev() {
        down_heap(keys, i, n, lo);
    }
    for i in (2..=n).rev() {
        keys.swap(lo as usize, (lo + i - 1) as usize);
        down_heap(keys, 1, i - 1, lo);
    }
}

fn legacy_sort(keys: &mut [ChartObject], mut left: isize, mut right: isize, mut depth: u32) {
    while left < right {
        if depth == 0 {
            heap_sort(keys, left, right);
            return;
        }
        let (mut i, mut j) = (left, right);
        let middle = i + ((j - i) >> 1);
        swap_if_greater(keys, i, middle);
        swap_if_greater(keys, i, j);
        swap_if_greater(keys, middle, j);
        let pivot = keys[middle as usize].start_ms;
        while i <= j {
            while keys[i as usize].start_ms < pivot {
                i += 1;
            }
            while pivot < keys[j as usize].start_ms {
                j -= 1;
            }
            if i > j {
                break;
            }
            if i < j {
                keys.swap(i as usize, j as usize);
            }
            i += 1;
            j -= 1;
        }
        depth -= 1;
        if j - left <= right - i {
            if left < j {
                legacy_sort(keys, left, j, depth);
            }
            left = i;
        } else {
            if i < right {
                legacy_sort(keys, i, right, depth);
            }
            right = j;
        }
    }
}
