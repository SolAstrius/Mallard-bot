use std::sync::OnceLock;

use rand::Rng;
use regex::Regex;

use crate::exceptions::{ProcessingError, ProcessingErrorKind};
use crate::mask::Mask;

// Static bubble image filenames sorted, matching content/bubbles/bubble_images
// in the Python source.
pub const BUBBLE_NAMES: &[&str] = &[
    "content/bubbles/bubble_images/bubbleRight.png",
    "content/bubbles/bubble_images/bubbleUp.png",
];
pub const BUBBLES_COUNT: usize = BUBBLE_NAMES.len();

#[derive(Debug, Clone, Default, PartialEq)]
pub struct VideoQuoteArguments {
    pub starting_point: Option<i64>,
    pub end_point: Option<i64>,
    pub speed: Option<f64>,
    pub final_length: Option<f64>,
    pub reverse: Option<bool>,
    pub speech_bubble: Option<usize>,
    pub is_emoji: Option<bool>,
    pub mask: Option<Mask>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PhotoQuoteArguments {
    pub speech_bubble: Option<usize>,
    pub is_emoji: Option<bool>,
}

fn re_s() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^s(\d+)$").unwrap())
}
fn re_e() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^e(\d+)$").unwrap())
}
fn re_x() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^x(\d+\.?\d*)$").unwrap())
}
fn re_b() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^b(\d*)$").unwrap())
}

fn err_args(msg: impl Into<String>) -> ProcessingError {
    ProcessingError::new(ProcessingErrorKind::ArgumentsParsingError, msg)
}

fn parse_bubble_value(raw: &str) -> Result<usize, ProcessingError> {
    if raw.is_empty() {
        let idx = rand::thread_rng().gen_range(0..BUBBLES_COUNT);
        return Ok(idx);
    }
    let n: i64 = raw
        .parse()
        .map_err(|_| err_args(format!("\"{}\" не подходит как аргумент для команды.", raw)))?;
    if n < 1 {
        return Err(err_args("Номер не может быть меньше 1."));
    }
    if (n as usize) > BUBBLES_COUNT {
        return Err(err_args(format!(
            "Количество пузырьков в коллекции: {}.",
            BUBBLES_COUNT
        )));
    }
    Ok((n - 1) as usize)
}

pub fn parse_video_arguments(line: &str) -> Result<VideoQuoteArguments, ProcessingError> {
    let mut result = VideoQuoteArguments::default();
    let mut seen_s = 0u32;
    let mut seen_e = 0u32;
    let mut seen_x = 0u32;
    let mut seen_b = 0u32;
    let mut seen_r = 0u32;
    let mut seen_j = 0u32;
    let mut seen_mask = 0u32;

    let mut words = line.split_whitespace();
    let _ = words.next(); // drop the command itself

    for word in words {
        if let Some(c) = re_s().captures(word) {
            seen_s += 1;
            if seen_s > 1 {
                return Err(err_args(
                    "Несколько вхождений аргумента 's*', не знаю, что делать :(",
                ));
            }
            result.starting_point = Some(c[1].parse().unwrap());
        } else if let Some(c) = re_e().captures(word) {
            seen_e += 1;
            if seen_e > 1 {
                return Err(err_args(
                    "Несколько вхождений аргумента 'e*', не знаю, что делать :(",
                ));
            }
            result.end_point = Some(c[1].parse().unwrap());
        } else if let Some(c) = re_x().captures(word) {
            seen_x += 1;
            if seen_x > 1 {
                return Err(err_args(
                    "Несколько вхождений аргумента 'x*', не знаю, что делать :(",
                ));
            }
            result.speed = Some(c[1].parse().unwrap());
        } else if let Some(c) = re_b().captures(word) {
            seen_b += 1;
            if seen_b > 1 {
                return Err(err_args(
                    "Несколько вхождений аргумента 'b*', не знаю, что делать :(",
                ));
            }
            result.speech_bubble = Some(parse_bubble_value(&c[1])?);
        } else if word == "r" {
            seen_r += 1;
            if seen_r > 1 {
                return Err(err_args(
                    "Несколько вхождений аргумента 'r*', не знаю, что делать :(",
                ));
            }
            result.reverse = Some(true);
        } else if word == "j" {
            seen_j += 1;
            if seen_j > 1 {
                return Err(err_args(
                    "Несколько вхождений аргумента 'j*', не знаю, что делать :(",
                ));
            }
            result.is_emoji = Some(true);
        } else if word == "c" || word.starts_with("m:") {
            seen_mask += 1;
            if seen_mask > 1 {
                return Err(err_args(
                    "Несколько вхождений аргумента маски, не знаю, что делать :(",
                ));
            }
            let m = if word == "c" {
                Mask::Circle
            } else {
                let name = &word[2..];
                Mask::from_preset(name).ok_or_else(|| {
                    err_args(format!(
                        "Не знаю такой маски: \"{name}\". Доступны: \
                         circle, square, triangle, diamond, hexagon, star."
                    ))
                })?
            };
            result.mask = Some(m);
        } else {
            return Err(err_args(format!(
                "\"{}\" не подходит как аргумент для команды.",
                word
            )));
        }

        if seen_e == 1 && seen_s == 0 {
            return Err(err_args(
                "Параметр \"e*\" должен использоваться только вместе с \"s*\".",
            ));
        }
    }

    Ok(result)
}

pub fn parse_photo_arguments(line: &str) -> Result<PhotoQuoteArguments, ProcessingError> {
    let mut result = PhotoQuoteArguments::default();
    let mut seen_b = 0u32;
    let mut seen_j = 0u32;

    let mut words = line.split_whitespace();
    let _ = words.next();

    for word in words {
        if let Some(c) = re_b().captures(word) {
            seen_b += 1;
            if seen_b > 1 {
                return Err(err_args(
                    "Несколько вхождений аргумента 'b*', не знаю, что делать :(",
                ));
            }
            result.speech_bubble = Some(parse_bubble_value(&c[1])?);
        } else if word == "j" {
            seen_j += 1;
            if seen_j > 1 {
                return Err(err_args(
                    "Несколько вхождений аргумента 'j*', не знаю, что делать :(",
                ));
            }
            result.is_emoji = Some(true);
        } else {
            return Err(err_args(format!(
                "\"{}\" не подходит как аргумент для команды.",
                word
            )));
        }
    }

    Ok(result)
}
