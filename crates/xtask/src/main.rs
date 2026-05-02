use anyhow::Result;
use clap::{Args, Parser, Subcommand, ValueEnum};
use warp_i18n::{I18nCheckMode, I18nCheckOptions, check_bundles};

#[derive(Debug, Parser)]
#[command(author, version, about = "Workspace automation tasks")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    CheckI18n(CheckI18nArgs),
}

#[derive(Debug, Args)]
struct CheckI18nArgs {
    #[arg(long)]
    check_parity: bool,
    #[arg(long, value_enum, default_value_t = CheckI18nModeArg::Normal)]
    mode: CheckI18nModeArg,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum CheckI18nModeArg {
    Normal,
    Hard,
}

impl From<CheckI18nModeArg> for I18nCheckMode {
    fn from(value: CheckI18nModeArg) -> Self {
        match value {
            CheckI18nModeArg::Normal => Self::Normal,
            CheckI18nModeArg::Hard => Self::Hard,
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::CheckI18n(args) => check_i18n(args),
    }
}

fn check_i18n(args: CheckI18nArgs) -> Result<()> {
    let mode = I18nCheckMode::from(args.mode);
    let report = check_bundles(I18nCheckOptions {
        check_parity: args.check_parity,
        mode,
    })?;

    println!(
        "i18n check passed: {} locales, {} resources, {} messages",
        report.locale_count, report.resource_count, report.message_count
    );

    Ok(())
}
