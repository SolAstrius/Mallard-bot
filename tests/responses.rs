use mallard_bot::{Response, ResponseType};

#[test]
fn response_construction() {
    let r = Response::new("ква", ResponseType::Text);
    assert_eq!(r.text, "ква");
    assert_eq!(r.response_type, ResponseType::Text);
}

#[test]
fn response_type_variants_distinct() {
    assert_ne!(ResponseType::Text, ResponseType::Sticker);
    assert_ne!(ResponseType::Sticker, ResponseType::Voice);
    assert_ne!(ResponseType::Voice, ResponseType::Text);
}
