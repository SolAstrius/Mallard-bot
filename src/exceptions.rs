use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessingErrorKind {
    None,
    FileTooLarge,
    Unexpected,
    WrongSourceType,
    ArgumentsParsingError,
}

impl ProcessingErrorKind {
    pub fn base_message(self) -> &'static str {
        match self {
            ProcessingErrorKind::None => "",
            ProcessingErrorKind::FileTooLarge => "Файл слишком большой :(",
            ProcessingErrorKind::Unexpected => "Что-то сломалось :(",
            ProcessingErrorKind::WrongSourceType => "Я такое квотить не умею :(",
            ProcessingErrorKind::ArgumentsParsingError => "",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessingError {
    pub kind: ProcessingErrorKind,
    pub additional: String,
}

impl ProcessingError {
    pub fn new(kind: ProcessingErrorKind, additional: impl Into<String>) -> Self {
        Self {
            kind,
            additional: additional.into(),
        }
    }

    pub fn of(kind: ProcessingErrorKind) -> Self {
        Self::new(kind, "")
    }
}

impl fmt::Display for ProcessingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Не ква!\n{}{}",
            self.kind.base_message(),
            self.additional
        )
    }
}

impl std::error::Error for ProcessingError {}
