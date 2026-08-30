use crate::args::LifecycleSchedule;

pub(crate) fn close_order(length: usize, schedule: LifecycleSchedule, seed: u64) -> Vec<usize> {
    let mut indices: Vec<_> = (0..length).collect();
    match schedule {
        LifecycleSchedule::All | LifecycleSchedule::Forward => {}
        LifecycleSchedule::Reverse => indices.reverse(),
        LifecycleSchedule::SeededRandom => shuffle(&mut indices, seed),
        LifecycleSchedule::BeforeAdmission | LifecycleSchedule::Interleaved => {}
    }
    indices
}

fn shuffle(indices: &mut [usize], seed: u64) {
    let mut state = seed.max(1);
    for upper in (1..indices.len()).rev() {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        indices.swap(upper, (state as usize) % (upper + 1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_schedule_is_a_total_permutation() {
        for schedule in [
            LifecycleSchedule::Forward,
            LifecycleSchedule::Reverse,
            LifecycleSchedule::SeededRandom,
            LifecycleSchedule::BeforeAdmission,
            LifecycleSchedule::Interleaved,
        ] {
            let mut actual = close_order(207, schedule, 29);
            actual.sort_unstable();
            assert_eq!(actual, (0..207).collect::<Vec<_>>());
        }
    }

    #[test]
    fn seeded_random_is_replayable_and_not_forward_order() {
        let first = close_order(32, LifecycleSchedule::SeededRandom, 29);
        let second = close_order(32, LifecycleSchedule::SeededRandom, 29);
        assert_eq!(first, second);
        assert_ne!(first, (0..32).collect::<Vec<_>>());
    }
}
