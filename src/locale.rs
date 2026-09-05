//! UI language follows the user's preferred OS language, not the region or keyboard.

use crate::cluster::{NodeRole, NodeStatus};
use std::sync::OnceLock;

/// Use the same language in background tasks and the terminal UI.
pub fn text<'a>(english: &'a str, russian: &'a str) -> &'a str {
    Language::detect().text(english, russian)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Language {
    #[default]
    English,
    Russian,
}

impl Language {
    pub fn detect() -> Self {
        static LANGUAGE: OnceLock<Language> = OnceLock::new();
        *LANGUAGE.get_or_init(|| Self::from_locale(sys_locale::get_locale().as_deref()))
    }

    pub fn from_locale(locale: Option<&str>) -> Self {
        let primary = locale
            .unwrap_or("")
            .trim()
            .split(['-', '_', '.', '@'])
            .next();
        if primary.is_some_and(|value| value.eq_ignore_ascii_case("ru")) {
            Self::Russian
        } else {
            Self::English
        }
    }

    pub fn text<'a>(self, english: &'a str, russian: &'a str) -> &'a str {
        match self {
            Self::English => english,
            Self::Russian => russian,
        }
    }

    pub fn role(self, role: NodeRole) -> &'static str {
        match role {
            NodeRole::Automatic => self.text("Automatic", "Автоматически"),
            NodeRole::Main => self.text("Main", "Основной"),
            NodeRole::Worker => self.text("Worker", "Дополнительный"),
        }
    }

    pub fn node_status(self, status: NodeStatus) -> &'static str {
        match status {
            NodeStatus::Discovered => self.text("Discovered", "Обнаружен"),
            NodeStatus::Pairing => self.text("Pairing", "Подключение"),
            NodeStatus::Trusted => self.text("Trusted", "Доверенный"),
            NodeStatus::Ready => self.text("Ready", "Готов"),
            NodeStatus::Busy => self.text("Busy", "Занят"),
            NodeStatus::Offline => self.text("Offline", "Не в сети"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_russian_language_independent_of_region() {
        for locale in ["ru", "ru-RU", "ru-KZ", "RU_ru.UTF-8", "ru_RU@variant"] {
            assert_eq!(Language::from_locale(Some(locale)), Language::Russian);
        }
    }

    #[test]
    fn english_and_unsupported_languages_fall_back_to_english() {
        for locale in [
            None,
            Some(""),
            Some("en-RU"),
            Some("en-US"),
            Some("de-DE"),
            Some("C.UTF-8"),
        ] {
            assert_eq!(Language::from_locale(locale), Language::English);
        }
    }
}
