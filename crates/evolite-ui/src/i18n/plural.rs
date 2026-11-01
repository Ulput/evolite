//! CLDR plural categories for whole numbers, per interface language
//! (unicode.org/cldr, "Language Plural Rules"; only the integer rules: amounts in DASH are
//! written with digits and need no plural word).

use super::Lang;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Category {
    Zero,
    One,
    Two,
    Few,
    Many,
    Other,
}

impl Category {
    /// The name used in the catalogs.
    pub fn name(self) -> &'static str {
        match self {
            Category::Zero => "zero",
            Category::One => "one",
            Category::Two => "two",
            Category::Few => "few",
            Category::Many => "many",
            Category::Other => "other",
        }
    }
}

/// The category of `n` in `lang`.
pub fn category(lang: Lang, n: u64) -> Category {
    let (n10, n100) = (n % 10, n % 100);
    match lang {
        // one: 1, 21, 31…; few: 2–4, 22–24…; many: the rest.
        Lang::Ru | Lang::Uk => {
            if n10 == 1 && n100 != 11 {
                Category::One
            } else if (2..=4).contains(&n10) && !(12..=14).contains(&n100) {
                Category::Few
            } else {
                Category::Many
            }
        }
        Lang::En | Lang::De | Lang::Es | Lang::Tr => {
            if n == 1 {
                Category::One
            } else {
                Category::Other
            }
        }
        // 0 and 1 are "one".
        Lang::Fr | Lang::PtBr | Lang::Hi => {
            if n <= 1 {
                Category::One
            } else {
                Category::Other
            }
        }
        Lang::Ar => match n {
            0 => Category::Zero,
            1 => Category::One,
            2 => Category::Two,
            _ if (3..=10).contains(&n100) => Category::Few,
            _ if (11..=99).contains(&n100) => Category::Many,
            _ => Category::Other,
        },
        Lang::ZhHans | Lang::Ja | Lang::Ko | Lang::Vi | Lang::Id => Category::Other,
    }
}
