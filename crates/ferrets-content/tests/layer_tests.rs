//! `LayerLevel`: where a layer stands on the vertical scale against another.

use ferrets_content::layer::LayerLevel;

//
// ─── Below ────────────────────────────────────────────────────────────────────
//

#[test]
fn level_is_below_higher_level() {
    assert!(LayerLevel::At(1).below(LayerLevel::At(2)));
    assert!(!LayerLevel::At(2).below(LayerLevel::At(1)));
}

#[test]
fn level_is_never_below_itself() {
    assert!(!LayerLevel::At(2).below(LayerLevel::At(2)));
}

#[test]
fn range_is_below_level_above_its_top() {
    // 1..=3 tops out at 3, under 4.
    assert!(LayerLevel::Range { from: 1, to: 3 }.below(LayerLevel::At(4)));
    assert!(!LayerLevel::At(4).below(LayerLevel::Range { from: 1, to: 3 }));
}

#[test]
fn ranges_sharing_level_are_neither_below_other() {
    // Both hold 3.
    let low = LayerLevel::Range { from: 1, to: 3 };
    let high = LayerLevel::Range { from: 3, to: 5 };
    assert!(!low.below(high));
    assert!(!high.below(low));
}
