#![forbid(unsafe_code)]

use crate::common::resolve::CurrentUser;
use crate::common::Error;
use crate::log::dev_info;
use crate::system::interface::UserId;
use crate::system::kernel::kernel_check;
use crate::system::timestamp::RecordScope;
use crate::system::User;
use crate::system::{time::Duration, timestamp::SessionRecordFile, Process};
use cli::help;
#[cfg(test)]
pub(crate) use cli::SudoAction;
#[cfg(not(test))]
use cli::SudoAction;
use std::path::PathBuf;

mod cli;
pub(crate) use cli::{SudoListOptions, SudoRunOptions, SudoValidateOptions};

pub(crate) mod diagnostic;
mod env;
mod pam;
mod pipeline;

#[cfg_attr(not(feature = "dev"), allow(dead_code))]
fn unstable_warning() {
    let check_var = std::env::var("SUDO_RS_IS_UNSTABLE").unwrap_or_else(|_| "".to_string());

    if check_var != "I accept that my system may break unexpectedly" {
        eprintln_ignore_io_error!(
            "WARNING!
Sudo-rs is compiled with development logs on, which means it is less secure and could potentially
break your system. We recommend that you do not run this on any production environment.
To turn off this warning and use sudo-rs you need to set the environment variable
SUDO_RS_IS_UNSTABLE to the value `I accept that my system may break unexpectedly`."
        );

        std::process::exit(1);
    }
}

const VERSION: &str = if let Some(version_override) = std::option_env!("SUDO_RS_VERSION") {
    version_override
} else {
    std::env!("CARGO_PKG_VERSION")
};

pub(crate) fn candidate_sudoers_file() -> PathBuf {
    if cfg!(unprivileged_for_testing_only) {
        return PathBuf::from("./sudoers");
    }

    let mut path = if cfg!(target_os = "freebsd") {
        option_env!("LOCALBASE").unwrap_or("/usr/local").into()
    } else {
        PathBuf::from("/")
    };
    path.push("etc/sudoers-rs");
    if !path.exists() {
        path.set_file_name("sudoers");
    };

    dev_info!("Running with {} file", path.display());
    path
}

fn sudo_process() -> Result<(), Error> {
    crate::log::SudoLogger::new("sudo: ").into_global_logger();

    dev_info!("development logs are enabled");

    self_check()?;
    kernel_check()?;

    // parse cli options
    match SudoAction::from_env() {
        Ok(action) => match action {
            SudoAction::Help(_) => {
                eprintln_ignore_io_error!("{}", help::long_help_message());
                std::process::exit(0);
            }
            SudoAction::Version(_) => {
                eprintln_ignore_io_error!("sudo-rs {VERSION}");
                std::process::exit(0);
            }
            SudoAction::RemoveTimestamp(_) => {
                let user = CurrentUser::resolve()?;
                let mut record_file =
                    SessionRecordFile::open_for_user(&user, Duration::seconds(0))?;
                record_file.reset()?;
                Ok(())
            }
            SudoAction::ResetTimestamp(_) => {
                if let Some(scope) = RecordScope::for_process(&Process::new()) {
                    let user = CurrentUser::resolve()?;
                    let mut record_file =
                        SessionRecordFile::open_for_user(&user, Duration::seconds(0))?;
                    record_file.disable(scope, None)?;
                }
                Ok(())
            }
            SudoAction::Validate(options) => pipeline::run_validate(options),
            SudoAction::Run(options) => {
                // special case for when no command is given
                if options.positional_args.is_empty() && !options.shell && !options.login {
                    eprintln_ignore_io_error!("{}", help::USAGE_MSG);
                    std::process::exit(1);
                } else {
                    #[cfg(feature = "dev")]
                    unstable_warning();

                    pipeline::run(options)
                }
            }
            SudoAction::List(options) => pipeline::run_list(options),
            SudoAction::Edit(_) => {
                eprintln_ignore_io_error!("error: `--edit` flag has not yet been implemented");
                std::process::exit(1);
            }
        },
        Err(e) => {
            eprintln_ignore_io_error!("{e}\n{}", help::USAGE_MSG);
            std::process::exit(1);
        }
    }
}

#[cfg(all(not(debug_assertions), unprivileged_for_testing_only))]
compile_error!(
    "unprivileged-for-testing-only feature must not be enabled in production as it is insecure"
);

fn self_check() -> Result<(), Error> {
    #[allow(clippy::collapsible_else_if)]
    if cfg!(unprivileged_for_testing_only) {
        // Make sure we unprivileged-for-testing-only is not enabled in production
        // by forbidding setuid when it is enabled.
        if User::effective_uid() == UserId::ROOT || User::effective_uid() != User::real_uid() {
            Err(Error::SelfCheckMustBeUnprivileged)
        } else {
            Ok(())
        }
    } else {
        if User::effective_uid() == UserId::ROOT {
            Ok(())
        } else {
            Err(Error::SelfCheck)
        }
    }
}

pub fn main() {
    match sudo_process() {
        Ok(()) => (),
        Err(error) => {
            if !error.is_silent() {
                diagnostic::diagnostic!("{error}");
            }
            std::process::exit(1);
        }
    }
}
