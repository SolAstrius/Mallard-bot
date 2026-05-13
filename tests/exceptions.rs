use mallard_bot::{ProcessingError, ProcessingErrorKind};

#[test]
fn wrong_source_type_message() {
    let e = ProcessingError::of(ProcessingErrorKind::WrongSourceType);
    assert_eq!(e.to_string(), "Не ква!\nЯ такое квотить не умею :(");
}

#[test]
fn file_too_large_message() {
    let e = ProcessingError::of(ProcessingErrorKind::FileTooLarge);
    assert_eq!(e.to_string(), "Не ква!\nФайл слишком большой :(");
}

#[test]
fn arguments_parsing_error_includes_additional() {
    let e = ProcessingError::new(
        ProcessingErrorKind::ArgumentsParsingError,
        "\"foo\" не подходит как аргумент для команды.",
    );
    assert_eq!(
        e.to_string(),
        "Не ква!\n\"foo\" не подходит как аргумент для команды."
    );
}

#[test]
fn none_kind_only_prefix() {
    let e = ProcessingError::of(ProcessingErrorKind::None);
    assert_eq!(e.to_string(), "Не ква!\n");
}
