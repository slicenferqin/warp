use crate::{
    I18nCheckMode, I18nCheckOptions, LanguagePreference, Locale, check_bundles, current_locale,
    set_current_locale, set_language_preference, tr_in_locale, try_translate_in_locale,
};
use settings_value::SettingsValue;

#[test]
fn locale_parse_accepts_supported_tags() {
    assert_eq!(Locale::parse("en"), Some(Locale::En));
    assert_eq!(Locale::parse("en-US"), Some(Locale::En));
    assert_eq!(Locale::parse("zh-CN"), Some(Locale::ZhCn));
    assert_eq!(Locale::parse("zh_CN.UTF-8"), Some(Locale::ZhCn));
    assert_eq!(Locale::parse("zh-Hans-CN"), Some(Locale::ZhCn));
    assert_eq!(Locale::parse("fr-FR"), None);
}

#[test]
fn language_preference_resolves_system_locale() {
    assert_eq!(
        LanguagePreference::System.resolve(Some("zh_CN.UTF-8")),
        Locale::ZhCn
    );
    assert_eq!(
        LanguagePreference::System.resolve(Some("en_US.UTF-8")),
        Locale::En
    );
    assert_eq!(
        LanguagePreference::System.resolve(Some("de-DE")),
        Locale::En
    );
    assert_eq!(
        LanguagePreference::ZhCn.resolve(Some("en-US")),
        Locale::ZhCn
    );
    assert_eq!(LanguagePreference::En.resolve(Some("zh-CN")), Locale::En);
}

#[test]
fn language_preference_serializes_as_settings_value() {
    let value = LanguagePreference::ZhCn.to_file_value();
    assert_eq!(value, serde_json::Value::String("zh-CN".to_string()));
    assert_eq!(
        LanguagePreference::from_file_value(&value),
        Some(LanguagePreference::ZhCn)
    );
}

#[test]
fn translates_static_keys_for_supported_locales() {
    assert_eq!(tr_in_locale(Locale::En, "settings-title"), "Settings");
    assert_eq!(tr_in_locale(Locale::ZhCn, "settings-title"), "设置");
    assert_eq!(tr_in_locale(Locale::En, "common-save"), "Save");
    assert_eq!(tr_in_locale(Locale::ZhCn, "common-save"), "保存");
    assert_eq!(
        tr_in_locale(Locale::ZhCn, "settings-account-log-out"),
        "退出登录"
    );
    assert_eq!(
        tr_in_locale(Locale::ZhCn, "settings-tooltip-local-only"),
        "此设置不会同步到你的其他设备"
    );
    assert_eq!(
        tr_in_locale(Locale::ZhCn, "settings-privacy-secret-redaction-title"),
        "敏感信息遮盖"
    );
    assert_eq!(
        tr_in_locale(Locale::ZhCn, "settings-privacy-add-regex-invalid"),
        "正则表达式无效"
    );
    assert_eq!(
        tr_in_locale(Locale::En, "settings-privacy-policy-title"),
        "Privacy policy"
    );
}

#[test]
fn zh_cn_translation_uses_locale_specific_value() {
    assert_eq!(tr_in_locale(Locale::ZhCn, "settings-title"), "设置");
}

#[test]
fn missing_key_returns_error_for_try_translate() {
    let error = try_translate_in_locale(Locale::En, "missing-key").expect_err("key is absent");
    assert!(error.to_string().contains("missing en translation"));
}

#[test]
fn current_locale_can_be_changed() {
    let original = current_locale();
    set_current_locale(Locale::ZhCn);
    assert_eq!(current_locale(), Locale::ZhCn);
    set_language_preference(LanguagePreference::En);
    assert_eq!(current_locale(), Locale::En);
    set_current_locale(original);
}

#[test]
fn bundled_resources_pass_key_parity() {
    let report = check_bundles(I18nCheckOptions {
        check_parity: true,
        mode: I18nCheckMode::Hard,
    })
    .expect("bundled resources should be valid");

    assert_eq!(report.locale_count, 2);
    assert_eq!(report.resource_count, 4);
    assert!(report.message_count > 0);
}
