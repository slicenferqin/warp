use settings::{
    macros::define_settings_group, RespectUserSyncSetting, Setting as _, SupportedPlatforms,
    SyncToCloud,
};
use warp_i18n::{set_language_preference, LanguagePreference};
use warpui::{AppContext, SingletonEntity as _};

define_settings_group!(LanguageSettings, settings: [
    language_preference: LanguagePreferenceSetting {
        type: LanguagePreference,
        default: LanguagePreference::System,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        storage_key: "LanguagePreference",
        toml_path: "language",
        description: "The UI language preference. Use system to follow the operating system locale.",
    },
]);

impl LanguageSettings {
    pub fn register_and_subscribe_to_events(app: &mut AppContext) {
        let handle = Self::register(app);
        Self::apply_current_preference(app);

        app.subscribe_to_model(&handle, |settings, event, ctx| {
            if matches!(
                event,
                LanguageSettingsChangedEvent::LanguagePreferenceSetting { .. }
            ) {
                let preference = *settings.as_ref(ctx).language_preference.value();
                set_language_preference(preference);
            }
        });
    }

    pub fn apply_current_preference(ctx: &AppContext) {
        let preference = *Self::as_ref(ctx).language_preference.value();
        set_language_preference(preference);
    }
}
