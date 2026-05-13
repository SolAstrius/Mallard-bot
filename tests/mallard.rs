use mallard_bot::{Mallard, ResponseType};

#[test]
fn empty_input_returns_none() {
    let m = Mallard::new(100);
    assert!(m.process("").is_none());
}

#[test]
fn exact_creature_match_returns_none() {
    let m = Mallard::new(100);
    // The Python implementation excludes inputs that are exactly equal to a
    // known creature string so the bot doesn't react to its own /inline replies.
    let creature = m.get_creature();
    // Crank random rate so the random-answer path basically never fires.
    let m = Mallard::new(1_000_000);
    assert!(m.process(creature).is_none());
}

#[test]
fn kva_keyword_always_matches() {
    // RANDOM_ANSWER_RATE has to be huge so a non-match never accidentally fires
    // the random response path; the keyword path is deterministic in that it
    // always returns *some* response when the keyword is present.
    let m = Mallard::new(1_000_000);
    for _ in 0..50 {
        let r = m.process("ква");
        assert!(r.is_some(), "ква should always produce a reply");
    }
}

#[test]
fn kar_keyword_matches() {
    let m = Mallard::new(1_000_000);
    for _ in 0..50 {
        assert!(m.process("кар").is_some());
    }
}

#[test]
fn kar_exception_blocks_kart() {
    // КАРТ is registered as an exception for КАР; on its own it should not
    // trigger the KAR reply path.
    let m = Mallard::new(1_000_000);
    for _ in 0..50 {
        assert!(m.process("карт").is_none());
    }
}

#[test]
fn keyword_match_is_case_insensitive() {
    let m = Mallard::new(1_000_000);
    assert!(m.process("КвА").is_some());
    assert!(m.process("ква-ква").is_some());
}

#[test]
fn random_answers_fire_at_expected_rate() {
    // Mirror of tests.py::test_random_answers.
    const N: usize = 5000;
    const RATE: u32 = 100;
    let m = Mallard::new(RATE);
    let mut hits = 0usize;
    for _ in 0..N {
        if m.process("aboba").is_some() {
            hits += 1;
        }
    }
    // Same bound as the Python test: 0 < hits < N * 2 / RATE.
    assert!(hits > 0 && hits < (N * 2 / RATE as usize));
}

#[test]
fn random_answer_rate_zero_never_fires() {
    let m = Mallard::new(0);
    for _ in 0..500 {
        assert!(m.process("aboba").is_none());
    }
}

#[test]
fn get_creature_returns_known_creature() {
    let m = Mallard::new(100);
    let c = m.get_creature();
    assert!(mallard_bot::dictionaries::CREATURES.contains(&c));
}

#[test]
fn response_returns_text_and_type() {
    let m = Mallard::new(1_000_000);
    let (text, ty) = m.process("ква").unwrap();
    assert!(!text.is_empty());
    matches!(
        ty,
        ResponseType::Text | ResponseType::Sticker | ResponseType::Voice
    );
}
