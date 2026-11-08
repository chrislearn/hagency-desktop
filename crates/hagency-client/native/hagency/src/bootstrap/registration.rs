//! Offline fleet registration command (G11): the trusted-local writer of the
//! `registrations` row the provisioning ingress requires before serve.
//!
//! Parity with the retained product's `POST /api/project-sides` →
//! `projectSideStore.upsertSide` (spec `task-rust-project-side-registration`),
//! but as a pre-serve CLI act — the port precedent for a trusted local store
//! write exposed to an operator (`hagency account …`, `accounts.rs`). The write
//! is `DomainRepository::register` (`hagency-store/src/domain.rs:730`), the sole
//! writer of the table; this module adds no second INSERT and no guard of its
//! own — the store's own contract is kept whole: shape validation
//! (`authority.rs:36-58`), the identical-content no-op, the stale-generation
//! refusal (`Error::Generation`), and the rotate-and-reconcile on advance.
use hagency_store::{DomainRepository, Repository, private};
use std::path::{Path, PathBuf};

#[derive(clap::Subcommand)]
pub enum Command {
    /// Write the fleet registration before first serve; refuses without an
    /// initialized private state. The record is the six-field JSON document:
    /// fleetId, generation, serverName, receptionRoomId, representativeMxid,
    /// approvalBotMxid.
    Register {
        /// Path to the registration JSON file (`-` reads stdin).
        #[arg(long)]
        file: PathBuf,
    },
    /// Bind the fleet reception room from a verified probe event, replacing
    /// the offline step (TS parity: `lib/fleet-protocol.js:132-153`).
    Probe(super::probe::BindArgs),
    /// Import the fleet configuration the owner downloaded from Palpo ("My
    /// HAgency access" → Download): the fleet registration row (reception
    /// unbound until Palpo's connection probe), the outbound transport
    /// credential and the App Service tokens. Run with the service stopped.
    Import {
        /// Path to the downloaded JSON file.
        #[arg(long)]
        file: PathBuf,
        /// The fleet's Matrix client API, e.g. https://matrix.example.org.
        #[arg(long)]
        homeserver: String,
        /// A reception room Palpo already verified for this fleet (its connection
        /// is proven, so Palpo sends no new probe); binds it at import.
        #[arg(long)]
        reception: Option<String>,
    },
}

pub fn run(state: &Path, command: Command) -> Result<(), hagency_store::Error> {
    // Require an initialized private state; never manufacture a replacement key
    // or import an ambient credential home. Token bytes never leave this scope.
    private::read_secret(&state.join("operator.token"))?;
    let _custody = Repository::open(state)?;
    let mut domain = DomainRepository::open(state)?;
    match command {
        Command::Register { file } => {
            let raw = if file.as_os_str() == "-" {
                let mut buf = String::new();
                std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)
                    .map_err(|_| hagency_store::Error::OutcomeUnknown)?;
                buf
            } else {
                std::fs::read_to_string(&file).map_err(|_| {
                    hagency_store::Error::Invalid(hagency_core::InvalidInput(
                        "registration file unreadable",
                    ))
                })?
            };
            let registration: hagency_core::authority::Registration = serde_json::from_str(&raw)
                .map_err(|_| {
                    hagency_store::Error::Invalid(hagency_core::InvalidInput(
                        "invalid registration document",
                    ))
                })?;
            // The store's own contract runs unmodified: validate, refuse a stale
            // generation, no-op on identical content, rotate and reconcile on an
            // advance. Nothing here softens or pre-empts it.
            domain.register(&registration)
        }
        Command::Import {
            file,
            homeserver,
            reception,
        } => {
            drop(domain);
            drop(_custody);
            let imported =
                super::palpo_import::run(state, &file, &homeserver, reception.as_deref()).map_err(
                    |error| {
                        eprintln!("Error: {error}");
                        hagency_store::Error::Invalid(hagency_core::InvalidInput(
                            "palpo import refused",
                        ))
                    },
                )?;
            println!(
                "{}",
                serde_json::json!({"imported": true, "fleetId": imported.fleet_id,
                    "serverName": imported.server_name, "representative": imported.representative,
                    "approvalBot": imported.approval_bot, "endpoint": imported.endpoint,
                    "reception": if imported.reception.is_empty() { "unbound until Palpo's Verify connection" } else { imported.reception.as_str() }})
            );
            Ok(())
        }
        Command::Probe(args) => super::probe::run(state, args).map_err(|_| {
            hagency_store::Error::Invalid(hagency_core::InvalidInput("probe refused"))
        }),
    }
}
