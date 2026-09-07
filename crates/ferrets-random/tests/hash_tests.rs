//! The mixer's outputs are pinned: they are what every peer computes.

use ferrets_random::hash;

#[test]
fn mix_of_known_inputs_is_pinned() {
    // The ids 1 to 8, each mixed with 0. Neither the values nor their parities
    // run in sequence: 1364076727 (odd) is followed by 821347078 (even), 543
    // million away.
    let outputs: Vec<u32> = (1..=8).map(|id| hash::mix(id, 0)).collect();
    assert_eq!(
        outputs,
        vec![
            1364076727, 821347078, 2247144487, 614249093, 3423425485, 1558924552, 415870660,
            1228498187
        ]
    );
    // The second input mixes in too: the same id with 1 lands elsewhere again.
    assert_eq!(hash::mix(1, 1), 920564995);
}
