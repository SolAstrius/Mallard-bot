use mallard_bot::triggers;

fn init_triggers() {
    // Idempotent: init() takes the OnceLock; subsequent calls overwrite the
    // pack in place. Safe to call from every test.
    triggers::init();
}

#[test]
fn empty_text_no_hit() {
    init_triggers();
    let pack = triggers::current();
    let ctx = triggers::MsgCtx::new("", 0);
    assert!(triggers::scan(&pack, &[], &ctx).is_none());
}

#[test]
fn creature_intro_present() {
    init_triggers();
    let pack = triggers::current();
    assert!(!pack.creatures.is_empty());
    assert!(pack.creatures.iter().any(|c| c.contains("Я уточка")));
}

#[test]
fn kva_fires() {
    init_triggers();
    let pack = triggers::current();
    // Sample many times to flatten rate variance — kva should be rate=1
    // (always when matched) and have a non-empty pool.
    for _ in 0..10 {
        let ctx = triggers::MsgCtx::new("ква", 0);
        if triggers::scan(&pack, &[], &ctx).is_some() {
            return;
        }
    }
    panic!("kva should produce a reply");
}

#[test]
fn kar_exception_blocks_kart() {
    init_triggers();
    let pack = triggers::current();
    // "карт" contains "КАР" but is excluded; nothing should fire from
    // creatures.kar, and the random group fires only ~1-in-150 so a few tries
    // is reliable-ish — we just don't want kar specifically.
    let ctx = triggers::MsgCtx::new("карт", 0);
    let hit = triggers::scan(&pack, &[], &ctx);
    if let Some(h) = hit {
        // If something fired, it must not be a kar reply text.
        assert!(
            !["кар", "кар!", "кар-кар", "карр", "кар-кар-кар"]
                .iter()
                .any(|s| h.reply_text == *s),
            "карт triggered kar reply: {:?}",
            h.reply_text
        );
    }
}

#[test]
fn random_fires_in_expected_band() {
    init_triggers();
    let pack = triggers::current();
    const N: usize = 5000;
    let mut hits = 0usize;
    for _ in 0..N {
        let ctx = triggers::MsgCtx::new("aboba", 0);
        if triggers::scan(&pack, &[], &ctx).is_some() {
            hits += 1;
        }
    }
    // Built-in random.default rate is 150, so expected ≈ N/150 ≈ 33.
    // Loose bound to keep the test stable across RNG variance.
    assert!(hits > 0 && hits < N * 2 / 50, "got {hits} hits out of {N}");
}

#[test]
fn random_rate_zero_never_fires() {
    init_triggers();
    let pack = triggers::current();
    let rules = vec![(
        "ambient.triggers.random.default.rate".to_string(),
        "0".to_string(),
    )];
    for _ in 0..500 {
        let ctx = triggers::MsgCtx::new("aboba", 0);
        assert!(triggers::scan(&pack, &rules, &ctx).is_none());
    }
}

#[test]
fn group_kill_switch() {
    init_triggers();
    let pack = triggers::current();
    let rules = vec![(
        "ambient.triggers.creatures".to_string(),
        "off".to_string(),
    )];
    // With creatures group off, "ква" should not produce a creatures.kva reply.
    for _ in 0..20 {
        let ctx = triggers::MsgCtx::new("ква", 0);
        if let Some(h) = triggers::scan(&pack, &rules, &ctx) {
            assert!(
                !h.reply_text.starts_with("ква"),
                "creatures group should be off: {:?}",
                h.reply_text
            );
        }
    }
}
