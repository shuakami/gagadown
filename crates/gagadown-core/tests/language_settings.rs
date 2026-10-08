use gagadown_core::config::Settings;

#[test]
fn old_settings_default_to_chinese() {
    let settings: Settings = serde_json::from_str("{}").unwrap();
    assert_eq!(settings.language, "zh-CN");
}

#[test]
fn language_preferences_round_trip_and_unknown_values_are_sanitized() {
    for language in ["zh-CN", "en", "system"] {
        let settings = Settings { language: language.to_owned(), ..Settings::default() }.sanitized();
        let json = serde_json::to_string(&settings).unwrap();
        let restored: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.language, language);
    }
    let settings = Settings { language: "unsupported".to_owned(), ..Settings::default() }.sanitized();
    assert_eq!(settings.language, "zh-CN");
}
