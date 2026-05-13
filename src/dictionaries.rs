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
    // Food (the chat eventually formalized its meal-time vocabulary)
    Pelmen,
    Borsch,
    Chai,
    // Universal interjections
    Blin,
    DaNu,
    OyVse,
    // Mood — added piecemeal over years
    Sad,
    Tired,
    Hungry,
    Cold,
    Sleepy,
    Hug,
    MorningCozy,
    // Affection / variants
    Brat,
    Cozy,
    Bread,
    Bunny,
    Goyda,
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
        // ---- food (added one meal-time discussion at a time) ----
        ("ПЕЛЬМЕН", Keyword::Pelmen),
        ("ПЕЛЬМЕШ", Keyword::Pelmen),
        ("БОРЩ", Keyword::Borsch),
        ("БОРЩИК", Keyword::Borsch),
        ("ЧАЙ", Keyword::Chai),
        ("ЧАЁК", Keyword::Chai),
        // ---- universal interjections ----
        ("БЛИИИ", Keyword::Blin),
        ("БЛЯЯЯ", Keyword::Blin),
        ("ДА НУ", Keyword::DaNu),
        ("ДА ЛАДНО", Keyword::DaNu),
        ("ОЙ ВСЁ", Keyword::OyVse),
        ("ОЙ ВСЕ", Keyword::OyVse),
        ("ГОЙДА", Keyword::Goyda),
        // ---- mood / care signals ----
        ("ГРУСТНО", Keyword::Sad),
        ("ПЕЧАЛЬ", Keyword::Sad),
        ("УСТАЛ", Keyword::Tired),
        ("ВЫМОТ", Keyword::Tired),
        ("ХОЧУ ЕСТЬ", Keyword::Hungry),
        ("ГОЛОДН", Keyword::Hungry),
        ("ХОЛОДНО", Keyword::Cold),
        ("ЗАМЁРЗ", Keyword::Cold),
        ("ЗАМЕРЗ", Keyword::Cold),
        ("СПАТЬ", Keyword::Sleepy),
        ("СПОКОЙНОЙ", Keyword::Sleepy),
        ("ОБНИМ", Keyword::Hug),
        ("ДОБРОЕ УТРО", Keyword::MorningCozy),
        ("ДОБРУТРО", Keyword::MorningCozy),
        // ---- affection ----
        ("БРАТИШ", Keyword::Brat),
        ("БРАТАН", Keyword::Brat),
        ("УЮТНО", Keyword::Cozy),
        ("ЛАПКИ", Keyword::Cozy),
        ("ПУШИСТ", Keyword::Cozy),
        ("ХЛЕБУШ", Keyword::Bread),
        ("ХЛЕБОБУЛ", Keyword::Bread),
        ("ЗАЙКА", Keyword::Bunny),
        ("ЗАЙЧИК", Keyword::Bunny),
    ]
});

/// Exceptions: if the keyword is detected but any of these substrings is also
/// present in the (uppercased) input, the keyword is suppressed.
pub static EXCEPTIONS: LazyLock<Vec<(&'static str, Vec<&'static str>)>> =
    LazyLock::new(|| vec![("КАР", vec!["КАРТ"])]);

pub static CREATURES: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    vec![
        // canonical five — never touch
        "Я жабка! Ква-ква!\u{1F438}",
        "Я уточка! Кря-кря!\u{1F986}",
        "Я котик! Миууууу!\u{1F408}",
        "Я пёсик! Тяв-тяв!\u{1F436}",
        "Я хрюшка! Хрю-хрю!\u{1F437}",
        // accrued over the years
        "Я ёжик! *шорох в листьях*\u{1F994}",
        "Я зайка! Прыг-прыг!\u{1F430}",
        "Я лисичка! Тяф-тяф!\u{1F98A}",
        "Я мишка! Ры... мурр.\u{1F43B}",
        "Я сова! Ух-ху-ху!\u{1F989}",
        "Я утёнок! Пи-пи!\u{1F425}",
        "Я черепашка. ......\u{1F422}",
        "Я хомячок! Щёчки полны!\u{1F439}",
        "Я пингвин. *смотрит вдаль*\u{1F427}",
        "Я кит! Бульк.\u{1F40B}",
        "Я грибочек!\u{1F344}",
    ]
});

fn combined_text_replies() -> Vec<(Keyword, Vec<&'static str>)> {
    vec![
        (
            Keyword::Kva,
            vec![
                "ква", "ква!", "ква-ква", "ква)", "ква\u{1F60C}",
                "квак", "квак-квак", "ква?", "кваква", "(ква)", "к в а",
            ],
        ),
        (
            Keyword::Kar,
            vec![
                "кар", "кар!", "кар-кар", "кар)", "кар\u{1F60C}",
                "кар?", "карр", "кар-кар-кар", "(кар)",
            ],
        ),
        (
            Keyword::Krya,
            vec![
                "кря", "кря!", "кря-кря", "кря)", "кря\u{1F60C}",
                "кря?", "крякря", "кря-кря-кря", "крякс", "(кря)", "к р я",
            ],
        ),
        (
            Keyword::Hryu,
            vec![
                "хрю", "хрюк", "хрю-хрю",
                "хрю?", "хрюшечка", "хрю-хрю-хрю", "хрюк-хрюк",
            ],
        ),
        (
            Keyword::Woof,
            vec![
                "тяв", "тяв\u{1F60C}", "тяв-тяв",
                "тяф", "гав!", "гав-гав", "гав)", "тявкс",
            ],
        ),
        (Keyword::Miu, vec!["миy\u{1F60C}", "мур", "мур-мур", "мяу"]),
        (Keyword::Mav, vec!["мав\u{1F60C}", "мав?", "мав-мав"]),
        (
            Keyword::Gaing,
            vec!["скр пяу гаиньг", "скр пяу", "гаиньг", "ало русский рэп?"],
        ),
        (
            Keyword::Kiss,
            vec![
                "чмок", "ты мне нравишься!!!", "чмок\u{1F970}",
                "чмок-чмок", "*целует в лобик*",
            ],
        ),
        (
            Keyword::Us,
            vec![
                "МЫЫЫЫЫЫЫЫЫЫ\u{1F970}", "МЫМЫМЫМЫ\u{1F970}",
                "МЫЫЫЫЫЫ\u{1F970}\u{1F970}\u{1F970}",
            ],
        ),
        (
            Keyword::What,
            vec![
                "фтоооооо", "чивоооооо", "читоооооо",
                "фтоооо?", "чевоо?", "ыыы что?",
            ],
        ),
        // ---- new keyword pools ----
        (
            Keyword::Pelmen,
            vec![
                "пельмеши\u{1F60C}", "сметанки бы", "пельмень не бьёт пельменя",
                "вам с уксусом или с майонезом?", "лепить будем?",
                "пельмешки лучшие\u{1F970}",
            ],
        ),
        (
            Keyword::Borsch,
            vec![
                "со сметаной?", "вчерашний всегда вкуснее",
                "борщик это серьёзно", "с пампушками\u{1F60C}",
            ],
        ),
        (
            Keyword::Chai,
            vec![
                "наливай", "с печеньками?", "чай это уют",
                "с лимоном или без?", "*заварила вторую кружку*",
            ],
        ),
        (
            Keyword::Blin,
            vec!["блииин", "ну блин\u{1F60C}", "блииинский блин"],
        ),
        (
            Keyword::DaNu,
            vec!["да ладно", "не может быть!", "не ври", "правда что ли?"],
        ),
        (
            Keyword::OyVse,
            vec!["ой всё\u{1F644}", "ой всё-всё", "пойду в свою комнату"],
        ),
        (
            Keyword::Goyda,
            vec!["гойда", "гоооойдааа", "г о й д а"],
        ),
        (
            Keyword::Sad,
            vec![
                "обниму\u{1F970}", "всё пройдёт",
                "хочешь чаю?", "я рядом",
                "ничего, ничего", "*кладёт лапку на плечо*",
            ],
        ),
        (
            Keyword::Tired,
            vec![
                "отдохни", "поспи минут двадцать",
                "ты молодец что дошёл досюда", "выпей водички",
                "*заворачивает в плед*",
            ],
        ),
        (
            Keyword::Hungry,
            vec![
                "иди поешь!", "пельмешек?", "у тебя есть что-то в холодильнике?",
                "не голодай, пожалуйста", "чайку с печеньками?",
            ],
        ),
        (
            Keyword::Cold,
            vec![
                "*кутает в плед*", "горячий чай выручит",
                "носочки шерстяные надень", "обнимашки греют",
            ],
        ),
        (
            Keyword::Sleepy,
            vec![
                "сладких снов\u{1F31B}", "баю-бай",
                "*выключает свет*", "увидимся утром",
                "не сиди в телефоне", "*укладывает в гнёздышко*",
            ],
        ),
        (
            Keyword::Hug,
            vec![
                "*обнимает крылышками*", "обнимаю крепко",
                "обнимашки\u{1F970}", "иди сюда",
                "🦆🤝🫂",
            ],
        ),
        (
            Keyword::MorningCozy,
            vec![
                "доброе утречко\u{1F60C}", "доброго дня тебе",
                "ква, как спалось?", "*наливает кофе*",
                "пусть будет хороший день",
            ],
        ),
        (
            Keyword::Brat,
            vec!["братишка\u{1F60C}", "братюнь", "по-братски"],
        ),
        (
            Keyword::Cozy,
            vec!["уютненько", "лапушки\u{1F970}", "пушистенько"],
        ),
        (
            Keyword::Bread,
            vec!["хлебушек\u{1F35E}", "ты булочка", "тёпленький"],
        ),
        (
            Keyword::Bunny,
            vec!["зайка\u{1F430}", "зайчонок", "ушастенький"],
        ),
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
        // canonical 2022 set — never touch
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
        // ---- cozy expansions (drip-fed over years) ----
        "сегодня твой день",
        "ты сегодня молодец",
        "ты заслуживаешь хорошего",
        "верю в тебя",
        "ты не один",
        "обнимаю",
        "иди поешь",
        "помни о себе",
        "ты лучший",
        "позвони маме",
        "проветри комнату",
        "попей водички",
        "ты делаешь больше чем кажется",
        "сделай перерыв",
        "посмотри в окно — там жизнь",
        "ты — главный человек в своей жизни",
        "выпрямись, плечи назад",
        "ты не обязан быть продуктивным сегодня",
        // ---- soft chaos / dadaist ----
        "ты теперь жабка",
        "ты — пельмень дня",
        "сегодня ты — лужа после дождя",
        "купи себе цветы",
        "поздравляю, ты выиграл нифига",
        "потеряешь телефон, найдёшь нового друга",
        "ты не туда зашёл",
        "доверься утке",
        "Валера передаёт привет",
        "вот это уже интересно",
        "хрюши за тебя",
        "это всё не зря",
        // ---- backhanded (added with a grin) ----
        "ты очень умный для понедельника",
        "у тебя красивые глаза но не сегодня",
        "ты милашка, в каком-то смысле",
        "твой интеллект поражает воображение",
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
