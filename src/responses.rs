#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseType {
    Text,
    Sticker,
    Voice,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub text: String,
    pub response_type: ResponseType,
}

impl Response {
    pub fn new(text: impl Into<String>, response_type: ResponseType) -> Self {
        Self {
            text: text.into(),
            response_type,
        }
    }
}
