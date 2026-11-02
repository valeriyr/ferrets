//! Times written for a player: seconds as an amount and as words.

use ferrets_demo::time;

//
// ─── Seconds ────────────────────────────────────────────────────────────────
//

#[test]
fn seconds_read_whole_numbers_whole() {
    assert_eq!(time::seconds_text(6.0), "6 Seconds");
    assert_eq!(time::seconds_text(0.0), "0 Seconds");
}

#[test]
fn seconds_read_to_tenth() {
    assert_eq!(time::seconds_text(2.5), "2.5 Seconds");
    // 2.95 rounds to 3.0, a whole number: written whole.
    assert_eq!(time::seconds_text(2.95), "3 Seconds");
}

#[test]
fn one_second_reads_singular() {
    assert_eq!(time::seconds_text(1.0), "1 Second");
    assert_eq!(time::seconds_words(1.0), "1 second");
}

#[test]
fn seconds_in_words_read_lowercase() {
    assert_eq!(time::seconds_words(2.5), "2.5 seconds");
}
