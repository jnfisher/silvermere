//! The `NEW` dialogue: username, password twice, optional e-mail, gender.
//!
//! The order and texts are MBBSEmu's (`MenuRoutines.cs` Signup*Display/Input). The
//! changes: no 5-9 character password rule (1-128 instead), a blank e-mail is
//! accepted and the check is loose, and the account is created at the end in one step
//! (MBBSEmu also wrote a `BBSUSR.DAT` mirror record, which has no counterpart here).
//! The gender question stays because the game reads the account's sex.

use crate::account::{
    PASSWORD_MAX, PasswordError, Sex, USERNAME_MAX, UsernameError, email_acceptable,
    validate_password, validate_username,
};
use crate::login::{AuthMsg, EchoMode, LoginDialogue, Output, Reply, Request, SECURE_ECHO, State};
use crate::password::Secret;

/// Where the signup dialogue is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignupState {
    /// At the username prompt.
    Username,
    /// Waiting to hear whether `username` is free.
    CheckingUsername {
        /// The name typed.
        username: String,
    },
    /// At the password prompt.
    Password {
        /// The chosen name.
        username: String,
    },
    /// At the confirmation prompt.
    Confirm {
        /// The chosen name.
        username: String,
        /// The first password.
        password: Secret,
    },
    /// At the e-mail prompt.
    Email {
        /// The chosen name.
        username: String,
        /// The password.
        password: Secret,
    },
    /// At the gender prompt.
    Gender {
        /// The chosen name.
        username: String,
        /// The password.
        password: Secret,
        /// Blank or an address.
        email: Option<String>,
    },
    /// Waiting for the account to be created.
    Creating,
}

impl SignupState {
    /// `NEW` was typed at the username prompt.
    pub(crate) fn start(dlg: &mut LoginDialogue, out: &mut Vec<Output>) {
        ask_username(dlg, out);
    }

    pub(crate) fn on_line(self, dlg: &mut LoginDialogue, line: &str, out: &mut Vec<Output>) {
        match self {
            Self::Username => match validate_username(line) {
                Ok(()) => {
                    out.push(Output::Request(Request::UsernameFree {
                        username: line.to_string(),
                    }));
                    set(
                        dlg,
                        Self::CheckingUsername {
                            username: line.to_string(),
                        },
                    );
                }
                Err(UsernameError::TooLong) => {
                    debug_assert!(line.len() > USERNAME_MAX);
                    out.push(Output::Text(AuthMsg::UsernameTooLong));
                    ask_username(dlg, out);
                }
                Err(_) => {
                    out.push(Output::Text(AuthMsg::SignupUsernameInvalid));
                    ask_username(dlg, out);
                }
            },
            Self::Password { username } => {
                out.push(Output::SetEcho(EchoMode::On));
                match validate_password(line) {
                    Ok(()) => ask_confirm(dlg, out, username, Secret::new(line)),
                    Err(e) => {
                        out.push(Output::Text(match e {
                            PasswordError::Length => AuthMsg::SignupPasswordInvalid {
                                max: PASSWORD_MAX as u32,
                            },
                            PasswordError::Weak => AuthMsg::SignupPasswordWeak,
                        }));
                        ask_password(dlg, out, username);
                    }
                }
            }
            Self::Confirm { username, password } => {
                out.push(Output::SetEcho(EchoMode::On));
                if line == password.expose() {
                    ask_email(dlg, out, username, password);
                } else {
                    out.push(Output::Text(AuthMsg::SignupPasswordMismatch));
                    ask_password(dlg, out, username);
                }
            }
            Self::Email { username, password } => {
                if email_acceptable(line) {
                    let email = (!line.is_empty()).then(|| line.to_string());
                    ask_gender(dlg, out, username, password, email);
                } else {
                    out.push(Output::Text(AuthMsg::SignupEmailInvalid));
                    ask_email(dlg, out, username, password);
                }
            }
            Self::Gender {
                username,
                password,
                email,
            } => match Sex::from_answer(line) {
                Some(sex) => {
                    out.push(Output::Request(Request::Create {
                        username,
                        password,
                        email,
                        sex,
                    }));
                    set(dlg, Self::Creating);
                }
                None => {
                    out.push(Output::Text(AuthMsg::SignupGenderInvalid));
                    ask_gender(dlg, out, username, password, email);
                }
            },
            // Waiting for a reply: the line is ignored.
            waiting @ (Self::CheckingUsername { .. } | Self::Creating) => set(dlg, waiting),
        }
    }

    pub(crate) fn on_reply(self, dlg: &mut LoginDialogue, reply: Reply, out: &mut Vec<Output>) {
        match (self, reply) {
            (Self::CheckingUsername { username }, Reply::UsernameFree(free)) => {
                if free {
                    ask_password(dlg, out, username);
                } else {
                    out.push(Output::Text(AuthMsg::SignupUsernameTaken));
                    ask_username(dlg, out);
                }
            }
            (Self::Creating, Reply::Created(created)) => match created {
                Some(account) => dlg.finish(*account),
                None => {
                    // Taken between the username check and now.
                    out.push(Output::Text(AuthMsg::SignupUsernameTaken));
                    ask_username(dlg, out);
                }
            },
            (state, reply) => {
                debug_assert!(false, "unexpected reply {reply:?} in {state:?}");
                set(dlg, state);
            }
        }
    }
}

fn set(dlg: &mut LoginDialogue, state: SignupState) {
    dlg.set_state(State::Signup(state));
}

fn ask_username(dlg: &mut LoginDialogue, out: &mut Vec<Output>) {
    out.push(Output::Text(AuthMsg::SignupUsernamePrompt));
    set(dlg, SignupState::Username);
}

fn ask_password(dlg: &mut LoginDialogue, out: &mut Vec<Output>, username: String) {
    out.push(Output::SetEcho(SECURE_ECHO));
    out.push(Output::Text(AuthMsg::SignupPasswordPrompt));
    set(dlg, SignupState::Password { username });
}

fn ask_confirm(dlg: &mut LoginDialogue, out: &mut Vec<Output>, username: String, password: Secret) {
    out.push(Output::SetEcho(SECURE_ECHO));
    out.push(Output::Text(AuthMsg::SignupConfirmPrompt));
    set(dlg, SignupState::Confirm { username, password });
}

fn ask_email(dlg: &mut LoginDialogue, out: &mut Vec<Output>, username: String, password: Secret) {
    out.push(Output::Text(AuthMsg::SignupEmailPrompt));
    set(dlg, SignupState::Email { username, password });
}

fn ask_gender(
    dlg: &mut LoginDialogue,
    out: &mut Vec<Output>,
    username: String,
    password: Secret,
    email: Option<String>,
) {
    out.push(Output::Text(AuthMsg::SignupGenderPrompt));
    set(
        dlg,
        SignupState::Gender {
            username,
            password,
            email,
        },
    );
}
