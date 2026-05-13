use std::sync::LazyLock;

use crate::responses::{Response, ResponseType};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Keyword {
    Kva,
    Kar,
    Krya,
    Hryu,
    Miu,
    Mav,
    Gaing,
    Kiss,
    Us,
    What,
    Arch,
    Woof,
}

/// Substring keyword → Keyword classification. Order matches the Python dict
/// (Python 3.7+ keeps insertion order) so behaviour is identical.
pub static TEXT_KEYWORDS: LazyLock<Vec<(&'static str, Keyword)>> = LazyLock::new(|| {
    vec![
        ("КВА", Keyword::Kva),
        ("КАР", Keyword::Kar),
        ("КРЯ", Keyword::Krya),
        ("ХРЮ", Keyword::Hryu),
        ("МИУ", Keyword::Miu),
        ("МАВ", Keyword::Mav),
        ("ГАИНЬГ", Keyword::Gaing),
        ("ЧМОК", Keyword::Kiss),
        ("МЫЫЫ", Keyword::Us),
        ("МЫМЫ", Keyword::Us),
        ("ФТОО", Keyword::What),
        ("ФТОФТО", Keyword::What),
        ("ЧИВОО", Keyword::What),
        ("ARCH", Keyword::Arch),
        ("АРЧ", Keyword::Arch),
        ("ТЯВ", Keyword::Woof),
        ("ГАВ", Keyword::Woof),
    ]
});

/// Exceptions: if the keyword is detected but any of these substrings is also
/// present in the (uppercased) input, the keyword is suppressed.
pub static EXCEPTIONS: LazyLock<Vec<(&'static str, Vec<&'static str>)>> =
    LazyLock::new(|| vec![("КАР", vec!["КАРТ"])]);

pub static CREATURES: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    vec![
        "Я жабка! Ква-ква!\u{1F438}",
        "Я уточка! Кря-кря!\u{1F986}",
        "Я котик! Миууууу!\u{1F408}",
        "Я пёсик! Тяв-тяв!\u{1F436}",
        "Я хрюшка! Хрю-хрю!\u{1F437}",
    ]
});

fn combined_text_replies() -> Vec<(Keyword, Vec<&'static str>)> {
    vec![
        (
            Keyword::Kva,
            vec!["ква", "ква!", "ква-ква", "ква)", "ква\u{1F60C}"],
        ),
        (
            Keyword::Kar,
            vec!["кар", "кар!", "кар-кар", "кар)", "кар\u{1F60C}"],
        ),
        (
            Keyword::Krya,
            vec!["кря", "кря!", "кря-кря", "кря)", "кря\u{1F60C}"],
        ),
        (Keyword::Hryu, vec!["хрю", "хрюк", "хрю-хрю"]),
        (Keyword::Woof, vec!["тяв", "тяв\u{1F60C}", "тяв-тяв"]),
        (Keyword::Miu, vec!["миy\u{1F60C}"]),
        (Keyword::Mav, vec!["мав\u{1F60C}"]),
        (
            Keyword::Gaing,
            vec!["скр пяу гаиньг", "скр пяу", "гаиньг", "ало русский рэп?"],
        ),
        (
            Keyword::Kiss,
            vec!["чмок", "ты мне нравишься!!!", "чмок\u{1F970}"],
        ),
        (
            Keyword::Us,
            vec!["МЫЫЫЫЫЫЫЫЫЫ\u{1F970}", "МЫМЫМЫМЫ\u{1F970}"],
        ),
        (Keyword::What, vec!["фтоооооо", "чивоооооо", "читоооооо"]),
    ]
}

fn combined_sticker_replies() -> Vec<(Keyword, Vec<&'static str>)> {
    vec![
        (
            Keyword::Kva,
            vec![
                "CAACAgIAAxkBAAIESWM1aZ9RdG-lmZp1s6G43v0AAWkz9wACaxEAAoQoUUjd6i8SNVbr1SoE",
                "CAACAgQAAxkBAAIEiWM1dv6UlBwAAakXetRKlhnhymykfwACawAD8YWLBHZImbEd8HQ_KgQ",
                "CAACAgQAAxkBAAIEi2M1d3bG7EYGTY7qYCzWRQI-0xEqAAIiAQACqCEhBsMhKQ89A7XmKgQ",
            ],
        ),
        (
            Keyword::Krya,
            vec![
                "CAACAgIAAxkBAAIER2M1aUsxYHmoj3SHqYn-X5mvCF98AAJqHQACYzEZSHXmSO3qgEwmKgQ",
                "CAACAgIAAxkBAAIF-WObMH_7uR5FLesxAq6mLbTXgtcZAAL2AANWnb0K99tOIUA-pYosBA",
                "CAACAgIAAxkBAAIF_GObMIpWbfQUrqTOwEszPdmL14uTAAIJAQACVp29CtZmXIPXP6gdLAQ",
                "CAACAgIAAxkBAAIF_2ObMJTxuaUT2odpn6I13qe0ZFBnAAILAQACVp29Ck6x56YI--1JLAQ",
            ],
        ),
        (
            Keyword::Miu,
            vec![
                "CAACAgIAAxkBAAIElWM1eNbip3RITb16zOxw-wJDobgXAAIiEAACV2HJS1e96adku96ZKgQ",
                "CAACAgIAAxkBAAIEl2M1eN3VUarEYilDZ81I1IDILRcqAAI6FAACh9vJS7eEtmgl-WtUKgQ",
            ],
        ),
        (
            Keyword::Mav,
            vec![
                "CAACAgIAAxkBAAIElWM1eNbip3RITb16zOxw-wJDobgXAAIiEAACV2HJS1e96adku96ZKgQ",
                "CAACAgIAAxkBAAIEl2M1eN3VUarEYilDZ81I1IDILRcqAAI6FAACh9vJS7eEtmgl-WtUKgQ",
            ],
        ),
        (
            Keyword::Woof,
            vec![
                "CAACAgIAAxkBAAI5vmOfEDYHgMiG9S5Gx4TKGTGF9mT9AALHLwACeIv4SEdXNRvd2S1SLAQ",
                "CAACAgIAAxkBAAI5v2OfEDdh1Ocm8I6DdVGR-mm-qASWAAJ1IwACi5H4SH5HSf9EkicCLAQ",
                "CAACAgIAAxkBAAI5wGOfEDfkM_gSvLpw5bNcpVdamKPjAAKYKAACgBz5SGvBWQcrxBnfLAQ",
                "CAACAgIAAxkBAAI5wWOfEDgTSHy3vI8O_vURd34kfeAGAALsIgAC_lcAAUkD28bbNUrhISwE",
                "CAACAgIAAxkBAAI5wmOfEDhXnlh2b8W9vyyqAAF13e0vCgACHiIAAq-d-UjzvXUoB6uYECwE",
                "CAACAgIAAxkBAAI7nWOhYJEd8ZXuAAGHURhFvoPJxXal-QACGSAAAmHeEElmhbbxu-s_ECwE",
            ],
        ),
        (
            Keyword::Kiss,
            vec![
                "CAACAgIAAxkBAAIES2M1cN1wtBwRBVJUrc41Q8IqUpdRAALbIQACh2hISahd3FVgrVqvKgQ",
                "CAACAgIAAxkBAAIEk2M1eL_ZB1rJK_YU3kPSepBCvIjPAAKfFQACXVnIS0QNrXBbo2y5KgQ",
            ],
        ),
        (
            Keyword::Arch,
            vec!["CAACAgIAAxkBAAILu2NIDKKE4m9XSO6rZsFurosK4O4yAAJ2IAACIutBSpGemvhk_ISKKgQ"],
        ),
        (
            Keyword::Us,
            vec![
                "CAACAgIAAxkBAAIFR2NIKRInpx4s4nZlKXaXFAJPikHyAAK8IwACSIZISahn5qW-aEGrKgQ",
                "CAACAgIAAxkBAAIFSGNIKRvYzrUHB7ymrb-2XrATv8lzAALbIQACh2hISahd3FVgrVqvKgQ",
                "CAACAgIAAxkBAAIFSWNIKSTf0R73OxeQHRrMaSuwSzHRAAJZIgAC7tFJSQF3F2uCltD_KgQ",
            ],
        ),
        (
            Keyword::What,
            vec![
                "CAACAgIAAxkBAAIF42NO3T-ikTjWnaIKw4NXp3qlE7e_AALnIAACnBRISUlZeonw23gyKgQ",
                "CAACAgIAAxkBAAIF5GNO3URR4d7GRGkiPjPZxlpNIS3UAAK_FQACcPWhSzBi-XEOAbBAKgQ",
                "CAACAgIAAxkBAAIF_2ObMJTxuaUT2odpn6I13qe0ZFBnAAILAQACVp29Ck6x56YI--1JLAQ",
            ],
        ),
    ]
}

fn basic_voice_replies() -> Vec<(Keyword, Vec<&'static str>)> {
    vec![
        (Keyword::Hryu, vec!["hryak", "hryak2", "hryak3"]),
        (Keyword::Miu, vec!["purring1", "purring2"]),
        (Keyword::Mav, vec!["purring1", "purring2"]),
        (Keyword::Us, vec!["us"]),
        (Keyword::Gaing, vec!["gaing"]),
    ]
}

fn build_basic_replies() -> Vec<(Keyword, Vec<Response>)> {
    let mut groups: Vec<(Keyword, Vec<Response>)> = Vec::new();

    let mut push = |kw: Keyword, text: &'static str, ty: ResponseType| {
        if let Some(slot) = groups.iter_mut().find(|(k, _)| *k == kw) {
            slot.1.push(Response::new(text, ty));
        } else {
            groups.push((kw, vec![Response::new(text, ty)]));
        }
    };

    for (kw, items) in combined_text_replies() {
        for t in items {
            push(kw, t, ResponseType::Text);
        }
    }
    for (kw, items) in combined_sticker_replies() {
        for t in items {
            push(kw, t, ResponseType::Sticker);
        }
    }
    for (kw, items) in basic_voice_replies() {
        for t in items {
            push(kw, t, ResponseType::Voice);
        }
    }

    groups
}

pub static BASIC_REPLIES: LazyLock<Vec<(Keyword, Vec<Response>)>> =
    LazyLock::new(build_basic_replies);

pub static RANDOM_RESPONSES: LazyLock<Vec<Response>> = LazyLock::new(|| {
    let text_items: Vec<&str> = vec![
        "очень умный",
        "этот человек прав во всём",
        "у тебя красивые глаза",
        "ты милашка",
        "ты отправляешься в Бразилию",
        "напиши в лс",
        "ты мне нравишься",
        "сделай дз",
        "может быть ты и не прав, но я все равно тебя поддержу",
        "идейно",
        "все будет хорошо, не переживай",
        "я знаю, что тебе нужна поддержка, поэтому я здесь\u{2764}\u{FE0F}\u{FE0F}",
        "другие тоже могут ошибаться, не злись на них",
        "выпей чаю с печеньками",
        "ляг спать сегодня пораньше",
        "твоя красота неописуема",
        "твой интеллект поражает",
    ];

    let sticker_items: Vec<&str> = vec![
        "CAACAgIAAxkBAAEFBu9ipzuFG-20sV5OsUY4_hi5rJ18gAAC6RMAAuPsyUmr2ECISdqixiQE",
        "CAACAgIAAxkBAAEFBvNipzxf-63InNz8z0z_008NtJfLnAACbRgAAidlUUsdYl5S0stX3CQE",
        "CAACAgIAAxkBAAEFBvFipzxcQfLts3PjuWFbILGHmb_C0gAC0hYAAsFiEUnTxi3655QiryQE",
        "CAACAgIAAxkBAAEFBvVipzxh0njUgH15X4R9YINTk29ZdAACfyYAAglGKUmYCpp8HdAu_iQE",
        "CAACAgIAAxkBAAEFBvdipzxjTmbUT3R8VgK4ob7NAZOFOgACVRYAAmcMAUmsUhPTHzF5jyQE",
        "CAACAgIAAxkBAAEFBvlipzxrwNXzkvbQIyfT5rGCei2j8wACaxEAAoQoUUjd6i8SNVbr1SQE",
        "CAACAgIAAxkBAAEFBvtipzyjmJhpvn-uy8-mT6kfg933PQACbRQAAvh48Ev_35tLbqKxRyQE",
        "CAACAgIAAxkBAAEFBv1ipzylTIkGlWmCbCpuZZvFzKZBeQACewAD98zUGO4bzlUFJLDEJAQ",
        "CAACAgIAAxkBAAEFBv9ipzy6tbC_PLclcKaCRaMdeHdvkwAC-xQAAlp-iUgIYSgt4y85eyQE",
        "CAACAgIAAxkBAAEFBwFipz0IlP1KOq1tmW1Vy4q7XOC2IgACkygAAhGBkUrzg2u5NfjyRyQE",
        "CAACAgIAAxkBAAID6WMH4dKBcpmIl_dBAWaxm0yEsRmyAAISGQACrcYYSE_oIr7KN-mmKQQ",
        "CAACAgIAAxkBAAID6mMH4dOy-c0LvTJEYYR3U5FjdPzsAAJ0GgACmwUZSOuvkMV9VT8YKQQ",
        "CAACAgIAAxkBAAID62MH4dTXA5YsctmZlVUu3YM-gDikAAJGHAACPT4ZSKODGsM_nvvnKQQ",
        "CAACAgIAAxkBAAID7GMH4diNKWofKaJMBbFohO7z5di7AAJ-HQACL6kgSN4Hywr1O8dXKQQ",
        "CAACAgIAAxkBAAID7WMH4dvqg2m10EyBwXddhXc-1b4tAAIGHwAC-IIZSGxrn7WAl1D2KQQ",
        "CAACAgIAAxkBAAID7mMH4d4pFa6aX3uX9Jq02y42rXfhAAJ6HQAC6VYhSIMXOJAX7sKkKQQ",
        "CAACAgIAAxkBAAID72MH4d8wJ8C1AyLG_0OCcfPStsZiAALnHQACA7woSCTavYd7l5r_KQQ",
        "CAACAgIAAxkBAAID8GMH4d9_Rp46uGXN85mt-uPRNhBaAAL7IAACDo8gSGhCetlzmDBvKQQ",
        "CAACAgIAAxkBAAID8WMH4eBkZrSXradn0MbZOhplzO6JAAJVHwACp3NASCCGtxiyz6VgKQQ",
        "CAACAgIAAxkBAAIF-WObMH_7uR5FLesxAq6mLbTXgtcZAAL2AANWnb0K99tOIUA-pYosBA",
        "CAACAgIAAxkBAAIF_GObMIpWbfQUrqTOwEszPdmL14uTAAIJAQACVp29CtZmXIPXP6gdLAQ",
        "CAACAgIAAxkBAAIF_2ObMJTxuaUT2odpn6I13qe0ZFBnAAILAQACVp29Ck6x56YI--1JLAQ",
        "CAACAgIAAxkBAAI5wWOfEDgTSHy3vI8O_vURd34kfeAGAALsIgAC_lcAAUkD28bbNUrhISwE",
    ];

    let voice_items: Vec<&str> = vec![
        "hryak", "hryak2", "hryak3", "us", "loud", "gaing", "purring1", "purring2", "valera",
    ];

    let mut out = Vec::new();
    for t in text_items {
        out.push(Response::new(t, ResponseType::Text));
    }
    for t in sticker_items {
        out.push(Response::new(t, ResponseType::Sticker));
    }
    for t in voice_items {
        out.push(Response::new(t, ResponseType::Voice));
    }
    out
});

pub fn replies_for(keyword: Keyword) -> Option<&'static Vec<Response>> {
    BASIC_REPLIES
        .iter()
        .find(|(k, _)| *k == keyword)
        .map(|(_, v)| v)
}
