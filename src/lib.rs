pub mod arguments;
pub mod bot;
pub mod dictionaries;
pub mod exceptions;
pub mod imaging;
pub mod mallard;
pub mod quote;
pub mod responses;
pub mod video;

pub use arguments::{
    parse_photo_arguments, parse_video_arguments, PhotoQuoteArguments, VideoQuoteArguments,
    BUBBLES_COUNT,
};
pub use exceptions::{ProcessingError, ProcessingErrorKind};
pub use mallard::Mallard;
pub use responses::{Response, ResponseType};
