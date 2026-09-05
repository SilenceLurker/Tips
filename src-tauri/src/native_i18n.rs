use crate::model::LocalePreference;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativeLanguage {
    SimplifiedChinese,
    English,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeTexts {
    pub main_title: &'static str,
    pub tray_show: &'static str,
    pub tray_quit: &'static str,
    pub tray_tooltip: &'static str,
    pub sticky_fallback_title: &'static str,
}

const ZH_CN: NativeTexts = NativeTexts {
    main_title: "Tips · 本地便签",
    tray_show: "显示主面板",
    tray_quit: "退出 Tips",
    tray_tooltip: "Tips · 本地便签",
    sticky_fallback_title: "Tips 便签",
};

const EN_US: NativeTexts = NativeTexts {
    main_title: "Tips · Local Notes",
    tray_show: "Show Tips",
    tray_quit: "Quit Tips",
    tray_tooltip: "Tips · Local Notes",
    sticky_fallback_title: "Tips Sticky Note",
};

pub fn texts_for_preference(preference: LocalePreference) -> NativeTexts {
    texts_for_locale(preference, sys_locale::get_locale().as_deref())
}

fn texts_for_locale(preference: LocalePreference, system_locale: Option<&str>) -> NativeTexts {
    match resolve_language(preference, system_locale) {
        NativeLanguage::SimplifiedChinese => ZH_CN,
        NativeLanguage::English => EN_US,
    }
}

fn resolve_language(preference: LocalePreference, system_locale: Option<&str>) -> NativeLanguage {
    match preference {
        LocalePreference::SimplifiedChinese => NativeLanguage::SimplifiedChinese,
        LocalePreference::English => NativeLanguage::English,
        LocalePreference::System => system_locale
            .filter(|locale| locale_is_chinese(locale))
            .map(|_| NativeLanguage::SimplifiedChinese)
            .unwrap_or(NativeLanguage::English),
    }
}

fn locale_is_chinese(locale: &str) -> bool {
    locale
        .split(['-', '_', '.', '@'])
        .next()
        .is_some_and(|language| language.eq_ignore_ascii_case("zh"))
}

#[cfg(test)]
mod tests {
    use super::{EN_US, ZH_CN, texts_for_locale};
    use crate::model::LocalePreference;

    #[test]
    fn explicit_locale_overrides_the_system_locale() {
        assert_eq!(
            texts_for_locale(LocalePreference::SimplifiedChinese, Some("en-US")),
            ZH_CN
        );
        assert_eq!(
            texts_for_locale(LocalePreference::English, Some("zh-CN")),
            EN_US
        );
    }

    #[test]
    fn system_locale_recognizes_common_chinese_forms() {
        for locale in ["zh-CN", "zh_Hans_CN", "ZH-cn", "zh-TW", "zh_CN.UTF-8"] {
            assert_eq!(
                texts_for_locale(LocalePreference::System, Some(locale)),
                ZH_CN
            );
        }
    }

    #[test]
    fn system_locale_falls_back_to_english() {
        for locale in [Some("en-US"), Some("de-DE"), Some("C"), None] {
            assert_eq!(texts_for_locale(LocalePreference::System, locale), EN_US);
        }
    }
}
