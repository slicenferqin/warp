use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;
use std::sync::{OnceLock, RwLock};

use fluent_bundle::{FluentArgs, FluentBundle, FluentResource};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use unic_langid::LanguageIdentifier;

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    schemars::JsonSchema,
    Serialize,
    Deserialize,
)]
pub enum Locale {
    #[serde(rename = "en")]
    #[default]
    En,
    #[serde(rename = "zh-CN")]
    ZhCn,
}

impl settings_value::SettingsValue for Locale {}

impl Locale {
    pub const fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::ZhCn => "zh-CN",
        }
    }

    pub fn parse(locale: &str) -> Option<Self> {
        let normalized = locale.split(['.', '@']).next().unwrap_or(locale);
        let normalized = normalized.replace('_', "-").to_ascii_lowercase();

        if normalized == "zh"
            || normalized.starts_with("zh-cn")
            || normalized.starts_with("zh-hans")
        {
            Some(Self::ZhCn)
        } else if normalized == "en" || normalized.starts_with("en-") {
            Some(Self::En)
        } else {
            None
        }
    }

    pub fn from_system_locale(system_locale: Option<&str>) -> Self {
        system_locale.and_then(Self::parse).unwrap_or(Self::En)
    }

    fn language_identifier(self) -> LanguageIdentifier {
        LanguageIdentifier::from_str(self.code()).expect("supported locales must be valid BCP-47")
    }
}

impl std::fmt::Display for Locale {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}

impl FromStr for Locale {
    type Err = ParseLocaleError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value).ok_or_else(|| ParseLocaleError(value.to_string()))
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[error("unsupported locale: {0}")]
pub struct ParseLocaleError(String);

#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Hash, schemars::JsonSchema, Serialize, Deserialize,
)]
pub enum LanguagePreference {
    #[serde(rename = "zh-CN")]
    ZhCn,
    #[serde(rename = "en")]
    En,
    #[serde(rename = "system")]
    #[default]
    System,
}

impl settings_value::SettingsValue for LanguagePreference {}

impl LanguagePreference {
    pub const fn code(self) -> &'static str {
        match self {
            Self::ZhCn => "zh-CN",
            Self::En => "en",
            Self::System => "system",
        }
    }

    pub fn resolve(self, system_locale: Option<&str>) -> Locale {
        match self {
            Self::ZhCn => Locale::ZhCn,
            Self::En => Locale::En,
            Self::System => Locale::from_system_locale(system_locale),
        }
    }

    pub fn resolve_current_system(self) -> Locale {
        self.resolve(current_system_locale().as_deref())
    }
}

impl std::fmt::Display for LanguagePreference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}

impl FromStr for LanguagePreference {
    type Err = ParseLanguagePreferenceError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "zh-CN" | "zh_cn" | "zh" => Ok(Self::ZhCn),
            "en" => Ok(Self::En),
            "system" => Ok(Self::System),
            other => Err(ParseLanguagePreferenceError(other.to_string())),
        }
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[error("unsupported language preference: {0}")]
pub struct ParseLanguagePreferenceError(String);

pub fn current_system_locale() -> Option<String> {
    #[cfg(not(target_family = "wasm"))]
    {
        sys_locale::get_locale()
    }

    #[cfg(target_family = "wasm")]
    {
        None
    }
}

pub fn current_locale() -> Locale {
    *current_locale_cell()
        .read()
        .expect("current locale lock should not be poisoned")
}

pub fn set_current_locale(locale: Locale) {
    *current_locale_cell()
        .write()
        .expect("current locale lock should not be poisoned") = locale;
}

pub fn set_language_preference(preference: LanguagePreference) -> Locale {
    let locale = preference.resolve_current_system();
    set_current_locale(locale);
    locale
}

fn current_locale_cell() -> &'static RwLock<Locale> {
    static CURRENT_LOCALE: OnceLock<RwLock<Locale>> = OnceLock::new();
    CURRENT_LOCALE.get_or_init(|| RwLock::new(LanguagePreference::System.resolve_current_system()))
}

#[macro_export]
macro_rules! t {
    ($key:literal) => {
        $crate::translate_static($key)
    };
    ($locale:expr, $key:literal) => {
        $crate::translate_static_in_locale($locale, $key)
    };
}

pub fn translate_static(key: &'static str) -> String {
    tr(key)
}

pub fn translate_static_in_locale(locale: Locale, key: &'static str) -> String {
    tr_in_locale(locale, key)
}

pub fn tr(key: impl AsRef<str>) -> String {
    tr_in_locale(current_locale(), key.as_ref())
}

pub fn tr_in_locale(locale: Locale, key: &str) -> String {
    try_translate_in_locale(locale, key).unwrap_or_else(|_| key.to_string())
}

pub fn tr_with_args(key: impl AsRef<str>, args: &[(&str, &str)]) -> String {
    tr_with_args_in_locale(current_locale(), key.as_ref(), args)
}

pub fn tr_with_args_in_locale(locale: Locale, key: &str, args: &[(&str, &str)]) -> String {
    let mut fluent_args = FluentArgs::new();
    for (name, value) in args {
        fluent_args.set(*name, *value);
    }

    try_translate_with_args(locale, key, Some(&fluent_args)).unwrap_or_else(|_| key.to_string())
}

pub fn try_translate(key: &str) -> Result<String, TranslationError> {
    try_translate_in_locale(current_locale(), key)
}

pub fn try_translate_in_locale(locale: Locale, key: &str) -> Result<String, TranslationError> {
    try_translate_with_args(locale, key, None)
}

pub fn try_translate_with_args(
    locale: Locale,
    key: &str,
    args: Option<&FluentArgs<'_>>,
) -> Result<String, TranslationError> {
    match translate_from_locale(locale, key, args) {
        Ok(value) => Ok(value),
        Err(error) if locale != Locale::En => {
            translate_from_locale(Locale::En, key, args).map_err(|_| {
                TranslationError::MissingMessage {
                    locale,
                    key: key.to_string(),
                    fallback_error: Some(Box::new(error)),
                }
            })
        }
        Err(error) => Err(error),
    }
}

fn translate_from_locale(
    locale: Locale,
    key: &str,
    args: Option<&FluentArgs<'_>>,
) -> Result<String, TranslationError> {
    let bundle = build_bundle(locale).map_err(|source| TranslationError::InvalidBundle {
        locale,
        source: Box::new(source),
    })?;
    let message = bundle
        .get_message(key)
        .ok_or_else(|| TranslationError::MissingMessage {
            locale,
            key: key.to_string(),
            fallback_error: None,
        })?;
    let pattern = message
        .value()
        .ok_or_else(|| TranslationError::MissingValue {
            locale,
            key: key.to_string(),
        })?;
    let mut errors = Vec::new();
    let value = bundle.format_pattern(pattern, args, &mut errors);

    if errors.is_empty() {
        Ok(value.into_owned())
    } else {
        Err(TranslationError::Format {
            locale,
            key: key.to_string(),
            errors: errors.into_iter().map(|error| error.to_string()).collect(),
        })
    }
}

fn build_bundle(locale: Locale) -> Result<FluentBundle<FluentResource>, BundleError> {
    let mut bundle = FluentBundle::new(vec![locale.language_identifier()]);
    bundle.set_use_isolating(false);

    for resource in resources_for_locale(locale) {
        let fluent_resource =
            FluentResource::try_new(resource.source.to_string()).map_err(|(_, errors)| {
                BundleError::Parse {
                    path: resource.path,
                    errors: errors
                        .into_iter()
                        .map(|error| format!("{error:?}"))
                        .collect(),
                }
            })?;
        bundle
            .add_resource(fluent_resource)
            .map_err(|errors| BundleError::AddResource {
                path: resource.path,
                errors: errors
                    .into_iter()
                    .map(|error| format!("{error:?}"))
                    .collect(),
            })?;
    }

    Ok(bundle)
}

#[derive(Debug, Error)]
pub enum TranslationError {
    #[error("invalid {locale} translation bundle")]
    InvalidBundle {
        locale: Locale,
        source: Box<BundleError>,
    },
    #[error("missing {locale} translation for key `{key}`")]
    MissingMessage {
        locale: Locale,
        key: String,
        fallback_error: Option<Box<TranslationError>>,
    },
    #[error("missing value for {locale} translation key `{key}`")]
    MissingValue { locale: Locale, key: String },
    #[error("failed to format {locale} translation key `{key}`: {errors:?}")]
    Format {
        locale: Locale,
        key: String,
        errors: Vec<String>,
    },
}

#[derive(Debug, Error)]
pub enum BundleError {
    #[error("failed to parse `{path}`: {errors:?}")]
    Parse {
        path: &'static str,
        errors: Vec<String>,
    },
    #[error("failed to add `{path}` to bundle: {errors:?}")]
    AddResource {
        path: &'static str,
        errors: Vec<String>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BundleResource {
    pub locale: Locale,
    pub path: &'static str,
    pub source: &'static str,
}

pub fn bundled_resources() -> &'static [BundleResource] {
    BUNDLED_RESOURCES
}

fn resources_for_locale(locale: Locale) -> impl Iterator<Item = &'static BundleResource> {
    BUNDLED_RESOURCES
        .iter()
        .filter(move |resource| resource.locale == locale)
}

const BUNDLED_RESOURCES: &[BundleResource] = &[
    BundleResource {
        locale: Locale::En,
        path: "app.ftl",
        source: include_str!("../bundles/en/app.ftl"),
    },
    BundleResource {
        locale: Locale::En,
        path: "common.ftl",
        source: include_str!("../bundles/en/common.ftl"),
    },
    BundleResource {
        locale: Locale::En,
        path: "settings.ftl",
        source: include_str!("../bundles/en/settings.ftl"),
    },
    BundleResource {
        locale: Locale::ZhCn,
        path: "app.ftl",
        source: include_str!("../bundles/zh-CN/app.ftl"),
    },
    BundleResource {
        locale: Locale::ZhCn,
        path: "common.ftl",
        source: include_str!("../bundles/zh-CN/common.ftl"),
    },
    BundleResource {
        locale: Locale::ZhCn,
        path: "settings.ftl",
        source: include_str!("../bundles/zh-CN/settings.ftl"),
    },
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum I18nCheckMode {
    Normal,
    Hard,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct I18nCheckOptions {
    pub check_parity: bool,
    pub mode: I18nCheckMode,
}

impl Default for I18nCheckOptions {
    fn default() -> Self {
        Self {
            check_parity: false,
            mode: I18nCheckMode::Normal,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct I18nCheckReport {
    pub locale_count: usize,
    pub resource_count: usize,
    pub message_count: usize,
}

pub fn check_bundles(options: I18nCheckOptions) -> Result<I18nCheckReport, I18nCheckError> {
    let mut resource_keys_by_locale =
        BTreeMap::<Locale, BTreeMap<&'static str, BTreeSet<String>>>::new();
    let mut message_count = 0;

    for resource in BUNDLED_RESOURCES {
        validate_resource(resource, options.mode)?;
        let keys = parse_message_keys(resource)?;
        message_count += keys.len();
        resource_keys_by_locale
            .entry(resource.locale)
            .or_default()
            .insert(resource.path, keys);
    }

    if options.check_parity || options.mode == I18nCheckMode::Hard {
        validate_key_parity(&resource_keys_by_locale)?;
    }

    Ok(I18nCheckReport {
        locale_count: resource_keys_by_locale.len(),
        resource_count: BUNDLED_RESOURCES.len(),
        message_count,
    })
}

fn validate_resource(resource: &BundleResource, mode: I18nCheckMode) -> Result<(), I18nCheckError> {
    let fluent_resource =
        FluentResource::try_new(resource.source.to_string()).map_err(|(_, errors)| {
            I18nCheckError::InvalidResource {
                locale: resource.locale,
                path: resource.path,
                errors: errors
                    .into_iter()
                    .map(|error| format!("{error:?}"))
                    .collect(),
            }
        })?;

    if mode == I18nCheckMode::Hard {
        let mut bundle = FluentBundle::new(vec![resource.locale.language_identifier()]);
        bundle.set_use_isolating(false);
        bundle
            .add_resource(fluent_resource)
            .map_err(|errors| I18nCheckError::InvalidResource {
                locale: resource.locale,
                path: resource.path,
                errors: errors
                    .into_iter()
                    .map(|error| format!("{error:?}"))
                    .collect(),
            })?;
    }

    Ok(())
}

fn validate_key_parity(
    resource_keys_by_locale: &BTreeMap<Locale, BTreeMap<&'static str, BTreeSet<String>>>,
) -> Result<(), I18nCheckError> {
    let en_resources = resource_keys_by_locale
        .get(&Locale::En)
        .ok_or(I18nCheckError::MissingLocale(Locale::En))?;

    for locale in [Locale::ZhCn] {
        let locale_resources = resource_keys_by_locale
            .get(&locale)
            .ok_or(I18nCheckError::MissingLocale(locale))?;

        let en_paths = en_resources.keys().copied().collect::<BTreeSet<_>>();
        let locale_paths = locale_resources.keys().copied().collect::<BTreeSet<_>>();

        if en_paths != locale_paths {
            return Err(I18nCheckError::ResourceParity {
                locale,
                missing_paths: en_paths.difference(&locale_paths).copied().collect(),
                extra_paths: locale_paths.difference(&en_paths).copied().collect(),
            });
        }

        for (path, en_keys) in en_resources {
            let locale_keys = locale_resources
                .get(path)
                .expect("resource path parity was checked above");

            if en_keys != locale_keys {
                return Err(I18nCheckError::KeyParity {
                    locale,
                    path,
                    missing_keys: en_keys.difference(locale_keys).cloned().collect(),
                    extra_keys: locale_keys.difference(en_keys).cloned().collect(),
                });
            }
        }
    }

    Ok(())
}

fn parse_message_keys(resource: &BundleResource) -> Result<BTreeSet<String>, I18nCheckError> {
    let mut keys = BTreeSet::new();

    for (line_index, line) in resource.source.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.len() != line.len() {
            continue;
        }

        let Some((raw_key, _)) = line.split_once('=') else {
            continue;
        };
        let key = raw_key.trim();

        if !is_message_key(key) {
            continue;
        }

        if !keys.insert(key.to_string()) {
            return Err(I18nCheckError::DuplicateKey {
                locale: resource.locale,
                path: resource.path,
                key: key.to_string(),
                line: line_index + 1,
            });
        }
    }

    if keys.is_empty() {
        return Err(I18nCheckError::NoMessages {
            locale: resource.locale,
            path: resource.path,
        });
    }

    Ok(keys)
}

fn is_message_key(key: &str) -> bool {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

#[derive(Debug, Error)]
pub enum I18nCheckError {
    #[error("invalid {locale} resource `{path}`: {errors:?}")]
    InvalidResource {
        locale: Locale,
        path: &'static str,
        errors: Vec<String>,
    },
    #[error("missing locale bundle `{0}`")]
    MissingLocale(Locale),
    #[error(
        "resource path parity failed for `{locale}`: missing {missing_paths:?}, extra {extra_paths:?}"
    )]
    ResourceParity {
        locale: Locale,
        missing_paths: Vec<&'static str>,
        extra_paths: Vec<&'static str>,
    },
    #[error(
        "key parity failed for `{locale}` resource `{path}`: missing {missing_keys:?}, extra {extra_keys:?}"
    )]
    KeyParity {
        locale: Locale,
        path: &'static str,
        missing_keys: Vec<String>,
        extra_keys: Vec<String>,
    },
    #[error("duplicate key `{key}` in {locale} resource `{path}` at line {line}")]
    DuplicateKey {
        locale: Locale,
        path: &'static str,
        key: String,
        line: usize,
    },
    #[error("no messages found in {locale} resource `{path}`")]
    NoMessages { locale: Locale, path: &'static str },
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
