use anyhow::{Context, Result, bail};
use clap::{Args, ValueEnum};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use warp_i18n::{Locale, bundled_resources};

#[derive(Debug, Args)]
pub struct I18nCoverageArgs {
    /// Source scope to scan for likely bare UI strings.
    #[arg(long, value_enum, default_value_t = I18nCoverageScopeArg::SettingsView)]
    scope: I18nCoverageScopeArg,

    /// Maximum number of candidate bare-string examples to print.
    #[arg(long, default_value_t = 40)]
    max_examples: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum I18nCoverageScopeArg {
    /// Scan app/src/settings_view.
    SettingsView,
    /// Scan all app/src Rust sources, excluding tests and integration helpers.
    App,
}

impl I18nCoverageScopeArg {
    fn scan_root(self) -> &'static str {
        match self {
            Self::SettingsView => "app/src/settings_view",
            Self::App => "app/src",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::SettingsView => "settings-view",
            Self::App => "app",
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
struct BareStringCandidate {
    path: PathBuf,
    line_number: usize,
    literal: String,
}

#[derive(Debug, Default)]
struct SourceAudit {
    localized_call_sites: usize,
    bare_string_candidates: Vec<BareStringCandidate>,
}

pub fn run(args: I18nCoverageArgs) -> Result<()> {
    let workspace_root = std::env::current_dir().context("failed to read current directory")?;
    let scan_root = workspace_root.join(args.scope.scan_root());
    if !scan_root.exists() {
        bail!(
            "scan root `{}` does not exist; run xtask from the workspace root",
            scan_root.display()
        );
    }

    let resource_counts = message_counts_by_locale();
    let source_audit = audit_source_tree(&scan_root, &workspace_root)?;
    let candidate_count = source_audit.bare_string_candidates.len();
    let denominator = source_audit.localized_call_sites + candidate_count;
    let coverage = if denominator == 0 {
        100.0
    } else {
        source_audit.localized_call_sites as f64 * 100.0 / denominator as f64
    };

    println!("i18n coverage audit ({})", args.scope.label());
    println!("  scan root: {}", args.scope.scan_root());
    println!("  resource messages by locale:");
    for (locale, count) in &resource_counts {
        println!("    {locale}: {count}");
    }
    println!(
        "  localized source call sites: {}",
        source_audit.localized_call_sites
    );
    println!("  candidate bare UI strings: {candidate_count}");
    println!(
        "  estimated source-call coverage: {:.1}% ({}/{})",
        coverage, source_audit.localized_call_sites, denominator
    );
    println!();
    println!("Notes:");
    println!("  - This is a heuristic source audit, not a product-level UI coverage guarantee.");
    println!(
        "  - Candidates intentionally exclude tests, URLs, logs, binding names, search terms, and i18n keys."
    );
    println!(
        "  - Review candidates before translating; dynamic/server/telemetry strings may still be false positives."
    );

    if candidate_count > 0 && args.max_examples > 0 {
        println!();
        println!(
            "Candidate examples (showing up to {} of {}):",
            args.max_examples.min(candidate_count),
            candidate_count
        );
        for candidate in source_audit
            .bare_string_candidates
            .iter()
            .take(args.max_examples)
        {
            let path = candidate
                .path
                .strip_prefix(&workspace_root)
                .unwrap_or(&candidate.path);
            println!(
                "  {}:{}: {:?}",
                path.display(),
                candidate.line_number,
                candidate.literal
            );
        }
    }

    Ok(())
}

fn message_counts_by_locale() -> BTreeMap<Locale, usize> {
    let mut counts = BTreeMap::new();

    for resource in bundled_resources() {
        *counts.entry(resource.locale).or_default() += message_keys(resource.source).len();
    }

    counts
}

fn message_keys(source: &str) -> BTreeSet<String> {
    source
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim_start();
            if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.len() != line.len() {
                return None;
            }

            let (raw_key, _) = line.split_once('=')?;
            let key = raw_key.trim();
            is_message_key(key).then(|| key.to_string())
        })
        .collect()
}

fn is_message_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
}

fn audit_source_tree(scan_root: &Path, workspace_root: &Path) -> Result<SourceAudit> {
    let mut audit = SourceAudit::default();
    let mut files = Vec::new();
    collect_rust_files(scan_root, &mut files)?;
    files.sort();

    for path in files {
        audit_source_file(&path, workspace_root, &mut audit)?;
    }

    Ok(audit)
}

fn collect_rust_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("failed to read {}", dir.display()))? {
        let entry =
            entry.with_context(|| format!("failed to read entry under {}", dir.display()))?;
        let path = entry.path();
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();

        if path.is_dir() {
            if should_skip_dir(&file_name) {
                continue;
            }
            collect_rust_files(&path, files)?;
        } else if path.extension().is_some_and(|ext| ext == "rs") && !should_skip_file(&file_name) {
            files.push(path);
        }
    }

    Ok(())
}

fn should_skip_dir(name: &str) -> bool {
    matches!(
        name,
        "integration_testing" | "tests" | "test_util" | "target"
    )
}

fn should_skip_file(name: &str) -> bool {
    name.ends_with("_test.rs") || name.ends_with("_tests.rs") || name == "mod_test.rs"
}

fn audit_source_file(path: &Path, workspace_root: &Path, audit: &mut SourceAudit) -> Result<()> {
    let source =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut in_ignored_function = false;
    let mut ignored_brace_depth: Option<usize> = None;
    let mut ignored_block_opened = false;
    let mut brace_depth = 0usize;

    for (line_index, line) in source.lines().enumerate() {
        let trimmed = line.trim_start();

        if trimmed.starts_with("fn search_terms(") || trimmed.starts_with("fn ui_name(") {
            in_ignored_function = true;
        }
        if ignored_brace_depth.is_none() && starts_ignored_block(trimmed) {
            ignored_brace_depth = Some(brace_depth);
            ignored_block_opened = false;
        }

        audit.localized_call_sites += localized_call_site_count(line);

        if !in_ignored_function && ignored_brace_depth.is_none() {
            for literal in string_literals_in_line(line) {
                if is_candidate_ui_string(line, &literal) {
                    audit.bare_string_candidates.push(BareStringCandidate {
                        path: path
                            .strip_prefix(workspace_root)
                            .unwrap_or(path)
                            .to_path_buf(),
                        line_number: line_index + 1,
                        literal,
                    });
                }
            }
        }

        if in_ignored_function && trimmed == "}" {
            in_ignored_function = false;
        }

        brace_depth = update_brace_depth(line, brace_depth);
        if ignored_brace_depth.is_some() && line_contains_code_char(line, '{') {
            ignored_block_opened = true;
        }
        if ignored_block_opened && ignored_brace_depth.is_some_and(|depth| brace_depth <= depth) {
            ignored_brace_depth = None;
            ignored_block_opened = false;
        }
    }

    Ok(())
}

fn starts_ignored_block(trimmed: &str) -> bool {
    trimmed.starts_with("pub mod flags")
        || trimmed.starts_with("impl Display for SettingsSection")
        || trimmed.starts_with("impl FromStr for SettingsSection")
        || trimmed.starts_with("pub fn init_actions_from_parent_view")
        || trimmed.starts_with("fn telemetry_event(")
        || (trimmed.starts_with("impl From<&") && trimmed.contains("LoginGatedFeature"))
        || (trimmed.starts_with("impl TryFrom<&") && trimmed.contains("TelemetryEvent"))
        || trimmed.starts_with("impl TelemetryEventDesc for")
}

fn update_brace_depth(line: &str, depth: usize) -> usize {
    let mut depth = depth;
    let mut in_string = false;
    let mut escaped = false;
    let mut chars = line.chars().peekable();

    while let Some(ch) = chars.next() {
        if !in_string && ch == '/' && chars.peek().is_some_and(|next| *next == '/') {
            break;
        }

        if escaped {
            escaped = false;
            continue;
        }

        match ch {
            '\\' if in_string => escaped = true,
            '"' => in_string = !in_string,
            '{' if !in_string => depth += 1,
            '}' if !in_string => depth = depth.saturating_sub(1),
            _ => {}
        }
    }

    depth
}

fn line_contains_code_char(line: &str, target: char) -> bool {
    let mut in_string = false;
    let mut escaped = false;
    let mut chars = line.chars().peekable();

    while let Some(ch) = chars.next() {
        if !in_string && ch == '/' && chars.peek().is_some_and(|next| *next == '/') {
            break;
        }

        if escaped {
            escaped = false;
            continue;
        }

        match ch {
            '\\' if in_string => escaped = true,
            '"' => in_string = !in_string,
            _ if !in_string && ch == target => return true,
            _ => {}
        }
    }

    false
}

fn localized_call_site_count(line: &str) -> usize {
    count_occurrences(line, "warp_i18n::tr(")
        + count_occurrences(line, "warp_i18n::tr_with_args(")
        + count_occurrences(line, "warp_i18n::t!(")
        + count_occurrences(line, ".localized_label(")
        + count_occurrences(line, ".localized_label_in_locale(")
}

fn count_occurrences(haystack: &str, needle: &str) -> usize {
    haystack.match_indices(needle).count()
}

fn string_literals_in_line(line: &str) -> Vec<String> {
    let mut literals = Vec::new();
    let mut chars = line.char_indices().peekable();

    while let Some((_, ch)) = chars.next() {
        if ch == '/' && chars.peek().is_some_and(|(_, next)| *next == '/') {
            break;
        }

        if ch != '"' {
            continue;
        }

        let mut literal = String::new();
        let mut escaped = false;
        for (_, ch) in chars.by_ref() {
            if escaped {
                literal.push(ch);
                escaped = false;
                continue;
            }

            match ch {
                '\\' => escaped = true,
                '"' => {
                    literals.push(literal);
                    break;
                }
                _ => literal.push(ch),
            }
        }
    }

    literals
}

fn is_candidate_ui_string(line: &str, literal: &str) -> bool {
    let literal = literal.trim();

    if literal.len() < 2 || literal.len() > 180 {
        return false;
    }
    if !literal.chars().any(|ch| ch.is_ascii_alphabetic()) {
        return false;
    }
    if is_ignored_context(line) || is_ignored_literal(literal) {
        return false;
    }

    let starts_like_label = literal
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_uppercase());
    let has_label_separator = literal.contains(' ')
        || literal.contains("...")
        || literal.contains('&')
        || literal.contains('>')
        || literal.contains(':');

    starts_like_label || has_label_separator
}

fn is_ignored_context(line: &str) -> bool {
    const IGNORED_CONTEXTS: &[&str] = &[
        "warp_i18n::",
        "log::",
        "tracing::",
        "debug!(",
        "info!(",
        "warn!(",
        "error!(",
        "panic!(",
        "assert",
        "debug_assert",
        ".expect(",
        "expect(",
        "id!(",
        "FeatureFlag::",
        "ContextFlag::",
        "TelemetryEvent::",
        "ToggleSettingActionPair::",
        "SettingActionPairDescriptions::",
        "SettingActionPairContexts::",
        "FixedBinding::",
        "LoginGatedFeature",
        "ui_name()",
        "storage_key:",
        "toml_path:",
        "description:",
        "onboarding_name:",
        "PageType::new_",
        "parse::<SettingsSection>",
        "ctx.open_url",
        "OpenUrl(",
        "url_source",
        "action:",
        "value:",
    ];

    let trimmed = line.trim_start();
    if trimmed.starts_with("//")
        || trimmed.starts_with("///")
        || trimmed.starts_with("//!")
        || trimmed.starts_with('*')
    {
        return true;
    }

    IGNORED_CONTEXTS
        .iter()
        .any(|pattern| line.contains(pattern))
}

fn is_ignored_literal(literal: &str) -> bool {
    if literal.contains("://")
        || literal.starts_with("mailto:")
        || literal.starts_with("bundled/")
        || literal.contains(".svg")
        || literal.contains(".png")
        || literal.contains(".toml")
        || literal.contains("::")
        || literal.contains("${")
        || literal.contains("\\n")
        || literal.contains("{}")
        || (literal.starts_with('{') && literal.ends_with('}'))
        || literal.starts_with('/')
        || literal.starts_with('.')
        || literal.starts_with('%')
        || literal.starts_with('#')
        || literal.starts_with('[')
        || is_internal_identifier_literal(literal)
    {
        return true;
    }

    if literal.starts_with("settings-")
        || literal.starts_with("command-palette-")
        || literal.starts_with("workspace:")
        || literal.starts_with("app:")
        || literal.starts_with("input:")
        || literal.starts_with("pane_group:")
        || literal.starts_with("Could not find current ")
        || literal.starts_with("Failed to ")
        || literal.starts_with("Could not ")
        || literal.starts_with("Successfully updated ")
        || literal.starts_with("Unable to ")
        || literal.starts_with("Ignoring ")
        || literal.starts_with("Unrecognized ")
        || literal.starts_with("Received an unexpected ")
        || literal.starts_with("This server is not an installation ")
        || literal.starts_with("Install server update is only supported ")
    {
        return true;
    }

    if matches!(
        literal,
        "ESC"
            | "JSON"
            | "Gallery Id: None"
            | "Gallery Id: {uuid}"
            | "File-Based MCP Id: {uuid}"
            | "Gallery MCP Id: {uuid}"
            | "Templatable MCP Id: {template_uuid}"
            | "Templatable MCP Installation Id: {uuid}"
            | "Could not find cloud template"
            | "Accept Autosuggestion"
            | "Open Completions Menu"
            | "Tab"
            | "Left Option key is Meta"
            | "Right Option key is Meta"
            | "Left Alt key is Meta"
            | "Right Alt key is Meta"
    ) {
        return true;
    }

    !literal.contains(' ')
        && literal
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-')
}

fn is_internal_identifier_literal(literal: &str) -> bool {
    !literal.contains(' ')
        && literal
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':'))
        && (literal.contains('_')
            || literal.contains("Flag.")
            || literal.ends_with("Enabled")
            || literal.ends_with("Open")
            || literal.ends_with("View"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_string_literals_without_line_comments() {
        assert_eq!(
            string_literals_in_line(r#"Text::new("Visible label") // "comment""#),
            vec!["Visible label".to_string()]
        );
        assert_eq!(
            string_literals_in_line(r#"let s = "escaped \" quote";"#),
            vec!["escaped \" quote".to_string()]
        );
    }

    #[test]
    fn filters_non_ui_literals() {
        assert!(!is_candidate_ui_string(
            r#"warp_i18n::tr("settings-account-log-out")"#,
            "settings-account-log-out"
        ));
        assert!(!is_candidate_ui_string(
            r#"ctx.open_url("https://docs.warp.dev")"#,
            "https://docs.warp.dev"
        ));
        assert!(is_candidate_ui_string(
            r#"Text::new("Check for updates")"#,
            "Check for updates"
        ));
        assert!(!is_candidate_ui_string(
            r#"pub const COPY_ON_SELECT_CONTEXT_FLAG: &str = "Copy_On_Select";"#,
            "Copy_On_Select"
        ));
        assert!(!is_candidate_ui_string(
            r#"FixedBinding::empty("ShowConversationHistory", action, context)"#,
            "ShowConversationHistory"
        ));
        assert!(!is_candidate_ui_string(
            r#"action: "ToggleCopyOnSelect".to_string(),"#,
            "ToggleCopyOnSelect"
        ));
        assert!(!is_candidate_ui_string(
            r#"value: format!("{mode:?}"),"#,
            "{mode:?}"
        ));
        assert!(!is_candidate_ui_string(
            r#"debug!("Refreshing GitHub auth URL")"#,
            "Refreshing GitHub auth URL"
        ));
    }

    #[test]
    fn tracks_brace_depth_outside_strings_and_comments() {
        assert_eq!(update_brace_depth(r#"fn f() { let s = "{"; }"#, 0), 0);
        assert_eq!(update_brace_depth(r#"if ok { // }"#, 0), 1);
        assert!(line_contains_code_char(r#"fn f() { let s = "x";"#, '{'));
        assert!(!line_contains_code_char(r#"let s = "{";"#, '{'));
    }

    #[test]
    fn counts_locale_messages() {
        let counts = message_counts_by_locale();
        assert!(counts.get(&Locale::En).copied().unwrap_or_default() > 0);
        assert_eq!(counts.get(&Locale::En), counts.get(&Locale::ZhCn));
    }
}
