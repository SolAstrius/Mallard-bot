use mallard_bot::{
    parse_photo_arguments, parse_video_arguments, ProcessingErrorKind, BUBBLES_COUNT,
};

#[test]
fn video_no_args() {
    let a = parse_video_arguments("/qva").unwrap();
    assert!(a.starting_point.is_none());
    assert!(a.end_point.is_none());
    assert!(a.speed.is_none());
    assert!(a.reverse.is_none());
    assert!(a.speech_bubble.is_none());
    assert!(a.is_emoji.is_none());
}

#[test]
fn video_starting_point() {
    let a = parse_video_arguments("/qva s5").unwrap();
    assert_eq!(a.starting_point, Some(5));
}

#[test]
fn video_all_args() {
    let a = parse_video_arguments("/qva s5 e7 x2.5 r").unwrap();
    assert_eq!(a.starting_point, Some(5));
    assert_eq!(a.end_point, Some(7));
    assert_eq!(a.speed, Some(2.5));
    assert_eq!(a.reverse, Some(true));
}

#[test]
fn video_emoji_flag() {
    let a = parse_video_arguments("/qva j").unwrap();
    assert_eq!(a.is_emoji, Some(true));
}

#[test]
fn video_bubble_specific() {
    // b1 in the user-facing language is the 0th bubble internally
    let a = parse_video_arguments("/qva b1").unwrap();
    assert_eq!(a.speech_bubble, Some(0));
}

#[test]
fn video_bubble_zero_is_error() {
    let err = parse_video_arguments("/qva b0").unwrap_err();
    assert_eq!(err.kind, ProcessingErrorKind::ArgumentsParsingError);
}

#[test]
fn video_bubble_too_large_is_error() {
    let bad = format!("/qva b{}", BUBBLES_COUNT + 1);
    let err = parse_video_arguments(&bad).unwrap_err();
    assert_eq!(err.kind, ProcessingErrorKind::ArgumentsParsingError);
}

#[test]
fn video_duplicate_s_is_error() {
    let err = parse_video_arguments("/qva s5 s6").unwrap_err();
    assert_eq!(err.kind, ProcessingErrorKind::ArgumentsParsingError);
}

#[test]
fn video_e_without_s_is_error() {
    let err = parse_video_arguments("/qva e7").unwrap_err();
    assert_eq!(err.kind, ProcessingErrorKind::ArgumentsParsingError);
}

#[test]
fn video_unknown_token_is_error() {
    let err = parse_video_arguments("/qva foo").unwrap_err();
    assert_eq!(err.kind, ProcessingErrorKind::ArgumentsParsingError);
}

#[test]
fn photo_no_args() {
    let a = parse_photo_arguments("/snap").unwrap();
    assert!(a.speech_bubble.is_none());
    assert!(a.is_emoji.is_none());
}

#[test]
fn photo_emoji_flag() {
    let a = parse_photo_arguments("/snap j").unwrap();
    assert_eq!(a.is_emoji, Some(true));
}

#[test]
fn photo_bubble_specific() {
    let a = parse_photo_arguments("/snap b1").unwrap();
    assert_eq!(a.speech_bubble, Some(0));
}

#[test]
fn photo_duplicate_j_is_error() {
    let err = parse_photo_arguments("/snap j j").unwrap_err();
    assert_eq!(err.kind, ProcessingErrorKind::ArgumentsParsingError);
}

#[test]
fn photo_unknown_token_is_error() {
    let err = parse_photo_arguments("/snap nope").unwrap_err();
    assert_eq!(err.kind, ProcessingErrorKind::ArgumentsParsingError);
}
