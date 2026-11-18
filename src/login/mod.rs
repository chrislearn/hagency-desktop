use crate::i18n::tr;
use hagency_store::credential_vault::Error as CredentialError;
use makepad_widgets::ScriptVm;

pub mod homeserver;
pub mod login_screen;
pub mod login_status_modal;

pub fn script_mod(vm: &mut ScriptVm) {
    login_status_modal::script_mod(vm);
    login_screen::script_mod(vm);
}

pub(crate) fn browser_credential_error_message(
    error: &anyhow::Error,
    authorized: bool,
) -> Option<&'static str> {
    Some(match error.downcast_ref::<CredentialError>()? {
        CredentialError::WriteResourcesExhausted => tr(
            "Windows Credential Manager could not save a new sign-in credential. If the store is full, remove only entries you no longer use, then try again.",
        ),
        CredentialError::Locked | CredentialError::InteractionRequired => {
            tr("Unlock your system credential store or approve its access prompt, then try again.")
        }
        CredentialError::Denied => tr(
            "Your system denied access to the credential store. Check its access settings and try again.",
        ),
        CredentialError::DurabilityUnknown if authorized => tr(
            "Browser authorization completed, but Hagency could not confirm that the local session was saved.",
        ),
        CredentialError::DurabilityUnknown => {
            tr("Hagency could not verify local credential storage before opening the browser.")
        }
        _ => tr("Hagency could not access local sign-in data. Restart Hagency and try again."),
    })
}
