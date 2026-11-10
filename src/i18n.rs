//! Bundled English and Dutch translations. This fork ships only these two languages.
use serde::{Deserialize, Deserializer, Serialize};
use std::{collections::BTreeMap, sync::OnceLock};
use time::OffsetDateTime;

type Messages = BTreeMap<String, BTreeMap<String, String>>;
fn messages() -> &'static Messages {
    static CATALOG: OnceLock<Messages> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("../assets/messages.json"))
            .expect("Bundled translations must be valid")
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    En,
    Nl,
}
impl Language {
    /// Sorted by displayed name.
    pub const ALL: [Self; 2] = [Self::En, Self::Nl];
    pub fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Nl => "nl",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::En => "English",
            Self::Nl => "Nederlands",
        }
    }
    pub fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|language| *language == self)
            .unwrap()
    }
    pub fn geocoding_code(self) -> &'static str {
        self.code()
    }
    pub fn t(self, key: &str) -> &str {
        messages()
            .get(key)
            .and_then(|entry| entry.get(self.code()).or_else(|| entry.get("en")))
            .map(String::as_str)
            .unwrap_or(key)
    }
    pub fn format(self, key: &str, values: &[(&str, &str)]) -> String {
        let mut value = self.t(key).to_owned();
        for (name, replacement) in values {
            value = value.replace(&format!("{{{name}}}"), replacement);
        }
        value
    }
    pub fn greeting(self, hour: u8, name: &str) -> String {
        let key = match hour {
            0..=4 => "greetingNight",
            5..=11 => "morning",
            12..=17 => "afternoon",
            _ => "evening",
        };
        let name = name.trim();
        if name.is_empty() {
            self.t(key).to_owned()
        } else {
            self.format(&format!("{key}Named"), &[("name", name)])
        }
    }
    /// The localized date in sentence case ("Thursday, October 22"), as tvOS shows dates.
    pub fn date(self, now: OffsetDateTime) -> String {
        let date = self.date_words(now);
        let mut chars = date.chars();
        chars.next().map_or(String::new(), |first| {
            first.to_uppercase().chain(chars).collect()
        })
    }

    /// The localized date inside a line, in the language's own casing ("maandag 9 november").
    pub fn date_inline(self, now: OffsetDateTime) -> String {
        self.date_words(now)
    }

    fn date_words(self, now: OffsetDateTime) -> String {
        let weekday = now.weekday().number_days_from_monday() as usize;
        let month = now.month() as usize - 1;
        match self {
            Self::En => format!(
                "{}, {} {}",
                [
                    "Monday",
                    "Tuesday",
                    "Wednesday",
                    "Thursday",
                    "Friday",
                    "Saturday",
                    "Sunday"
                ][weekday],
                [
                    "January",
                    "February",
                    "March",
                    "April",
                    "May",
                    "June",
                    "July",
                    "August",
                    "September",
                    "October",
                    "November",
                    "December"
                ][month],
                now.day()
            ),
            Self::Nl => format!(
                "{} {} {}",
                [
                    "maandag",
                    "dinsdag",
                    "woensdag",
                    "donderdag",
                    "vrijdag",
                    "zaterdag",
                    "zondag"
                ][weekday],
                now.day(),
                [
                    "januari",
                    "februari",
                    "maart",
                    "april",
                    "mei",
                    "juni",
                    "juli",
                    "augustus",
                    "september",
                    "oktober",
                    "november",
                    "december"
                ][month]
            ),
        }
    }
    pub fn clock(self, now: OffsetDateTime, hour24: bool, seconds: bool) -> String {
        let base = self.clock_base(now, hour24);
        if !seconds {
            return base;
        }
        let tail = format!(":{:02}", now.second());
        match base.find(' ') {
            Some(space) => format!("{}{tail}{}", &base[..space], &base[space..]),
            None => format!("{base}{tail}"),
        }
    }
    fn clock_base(self, now: OffsetDateTime, hour24: bool) -> String {
        if hour24 {
            return if self == Self::En {
                format!("{:02}:{:02}", now.hour(), now.minute())
            } else {
                format!("{}:{:02}", now.hour(), now.minute())
            };
        }
        let hour = match now.hour() % 12 {
            0 => 12,
            value => value,
        };
        format!(
            "{hour}:{:02} {}",
            now.minute(),
            if now.hour() >= 12 { "PM" } else { "AM" }
        )
    }
}
/// Dutch, or English for anything else, including languages earlier versions offered: a saved
/// choice this fork no longer ships falls back to English instead of failing to load.
pub fn deserialize_language<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Language, D::Error> {
    Ok(
        match serde_json::Value::deserialize(deserializer)?.as_str() {
            Some("nl") => Language::Nl,
            _ => Language::En,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_english_and_dutch_ship_and_removed_languages_fall_back_to_english() {
        assert_eq!(Language::ALL, [Language::En, Language::Nl]);
        let names: Vec<_> = Language::ALL
            .iter()
            .map(|language| language.name())
            .collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(
            names, sorted,
            "Language choices are sorted by displayed name"
        );
        for (saved, expected) in [
            ("nl", Language::Nl),
            ("en", Language::En),
            ("de", Language::En),
            ("zh-tw", Language::En),
            ("vi", Language::En),
        ] {
            let preferences: crate::model::Preferences =
                serde_json::from_value(serde_json::json!({"language": saved})).unwrap();
            assert_eq!(preferences.language, expected, "{saved}");
        }
    }

    #[test]
    fn messages_hold_only_the_shipped_languages() {
        for (key, entry) in messages() {
            let codes: Vec<_> = entry.keys().map(String::as_str).collect();
            assert_eq!(codes, ["en", "nl"], "{key}");
        }
    }

    #[test]
    fn homebrew_description_lists_every_supported_language() {
        let description = include_str!("../assets/homebrew-description.html");
        for language in Language::ALL {
            let name = match language {
                Language::En => "English",
                Language::Nl => "Dutch",
            };
            assert!(
                description.contains(name),
                "Homebrew description is missing {name}"
            );
        }
    }
    #[test]
    fn every_message_has_all_languages_and_no_empty_values() {
        for (key, entry) in messages() {
            for language in Language::ALL {
                assert!(
                    !entry
                        .get(language.code())
                        .unwrap_or_else(|| panic!("Missing {key} / {}", language.code()))
                        .is_empty()
                );
            }
        }
    }
    #[test]
    fn every_translation_preserves_the_english_message_placeholders() {
        fn placeholders(message: &str) -> std::collections::BTreeSet<&str> {
            message
                .split('{')
                .skip(1)
                .map(|part| {
                    part.split_once('}')
                        .expect("Unclosed message placeholder")
                        .0
                })
                .collect()
        }

        for (key, entry) in messages() {
            let expected = placeholders(&entry["en"]);
            for language in Language::ALL {
                assert_eq!(
                    placeholders(&entry[language.code()]),
                    expected,
                    "Mismatched placeholders in {key} / {}",
                    language.code()
                );
            }
        }
    }
    #[test]
    fn localization_keeps_user_provided_names_intact() {
        assert_eq!(
            Language::Nl.format("favoriteAdded", &[("app", "Netflix")]),
            "Netflix toegevoegd aan favorieten."
        );
        assert_eq!(Language::Nl.greeting(2, "Ana"), "Nog wakker, Ana?");
        assert_eq!(Language::En.greeting(15, ""), "Good afternoon.");
        assert_eq!(Language::En.greeting(15, "  Ana "), "Good afternoon, Ana.");
    }
    #[test]
    fn clock_and_date_follow_the_selected_locale() {
        let now = OffsetDateTime::parse(
            "2026-10-21T20:41:00+02:00",
            &time::format_description::well_known::Rfc3339,
        )
        .unwrap();
        assert_eq!(Language::Nl.date(now), "Woensdag 21 oktober");
        assert_eq!(Language::Nl.clock(now, true, false), "20:41");
        assert_eq!(Language::En.date(now), "Wednesday, October 21");
        assert_eq!(Language::En.clock(now, true, false), "20:41");
        assert_eq!(Language::En.clock(now, true, true), "20:41:00");
        assert_eq!(Language::En.clock(now, false, true), "8:41:00 PM");
    }

    #[test]
    fn dutch_dates_cover_leap_days_and_month_boundaries() {
        for (date, expected) in [
            ("2024-02-29T00:05:00+01:00", "Donderdag 29 februari"),
            ("2026-05-01T12:00:00+02:00", "Vrijdag 1 mei"),
            ("2026-12-27T20:41:00+01:00", "Zondag 27 december"),
        ] {
            let now = OffsetDateTime::parse(date, &time::format_description::well_known::Rfc3339)
                .unwrap();
            assert_eq!(Language::Nl.date(now), expected);
        }
    }
}
