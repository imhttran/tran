//! Initial-admin bootstrap command.
//!
//! This is the only way to create the first admin. It is an explicit operator
//! action, writes the user, the admin role assignment, and the audit event in one
//! transaction, and refuses to run when an admin already exists. It is not
//! reachable from any HTTP route and is not a configuration switch.
//!
//! Identity is keyed on `(issuer, subject)`. Every state-changing command takes
//! `--issuer` so it can never resolve a same-subject account under another
//! issuer.

use rag_foundation::identity::Role;
use rag_foundation::identity_store::IdentityStore;
use rag_foundation::identity_store_pg::PgIdentityClient;

fn usage() -> String {
    [
        "usage:",
        "  admin bootstrap --subject <sub> --issuer <issuer> [--corpus <corpus>] [--actor <actor>]",
        "  admin grant     --subject <sub> --issuer <issuer> --role viewer|editor|admin [--actor <actor>]",
        "  admin revoke    --subject <sub> --issuer <issuer> --role viewer|editor|admin [--actor <actor>]",
        "  admin grant-corpus  --subject <sub> --issuer <issuer> --corpus <corpus> [--actor <actor>]",
        "  admin revoke-corpus --subject <sub> --issuer <issuer> --corpus <corpus> [--actor <actor>]",
        "  admin provision      --subject <sub> --issuer <issuer> [--actor <actor>]",
        "  admin deactivate     --subject <sub> --issuer <issuer> [--actor <actor>]",
    ]
    .join("\n")
}

fn flag(args: &[String], name: &str) -> Option<String> {
    let mut iter = args.iter();
    while let Some(value) = iter.next() {
        if value == name {
            return iter.next().cloned();
        }
    }
    None
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = args.first() else {
        return Err(usage());
    };
    let database_url =
        std::env::var("DATABASE_URL").map_err(|_| "DATABASE_URL must be set".to_string())?;
    let subject = flag(&args, "--subject").ok_or_else(|| "--subject is required".to_string())?;
    let issuer = flag(&args, "--issuer").ok_or_else(|| "--issuer is required".to_string())?;
    let actor = flag(&args, "--actor").unwrap_or_else(|| "cli-operator".to_string());

    let client = PgIdentityClient::connect(&database_url).map_err(|err| err.to_string())?;
    let mut store = IdentityStore::new(client);

    match command.as_str() {
        "bootstrap" => {
            store
                .bootstrap_initial_admin(&subject, &issuer, &actor)
                .map_err(|err| err.to_string())?;
            if let Some(corpus) = flag(&args, "--corpus") {
                store
                    .client()
                    .grant_corpus(&subject, &issuer, &corpus, &actor)
                    .map_err(|err| err.to_string())?;
                println!("initial admin created for {subject} with corpus {corpus}");
            } else {
                println!("initial admin created for {subject}");
            }
        }
        "grant" | "revoke" => {
            let role_name =
                flag(&args, "--role").ok_or_else(|| "--role is required".to_string())?;
            let role = Role::parse(&role_name)
                .ok_or_else(|| "role must be viewer, editor, or admin".to_string())?;
            if command == "grant" {
                store
                    .grant_role(&subject, &issuer, role, &actor)
                    .map_err(|err| err.to_string())?;
            } else {
                store
                    .revoke_role(&subject, &issuer, role, &actor)
                    .map_err(|err| err.to_string())?;
            }
            println!("{command} {role_name} for {subject}");
        }
        "grant-corpus" | "revoke-corpus" => {
            let corpus =
                flag(&args, "--corpus").ok_or_else(|| "--corpus is required".to_string())?;
            if command == "grant-corpus" {
                store
                    .client()
                    .grant_corpus(&subject, &issuer, &corpus, &actor)
                    .map_err(|err| err.to_string())?;
            } else {
                store
                    .client()
                    .revoke_corpus(&subject, &issuer, &corpus, &actor)
                    .map_err(|err| err.to_string())?;
            }
            println!("{command} {corpus} for {subject}");
        }
        "provision" => {
            let user_id = store
                .client()
                .provision_user(&subject, &issuer, &actor)
                .map_err(|err| err.to_string())?;
            println!("provisioned {subject} as user {user_id}");
        }
        "deactivate" => {
            store
                .client()
                .deactivate_user(&subject, &issuer, &actor)
                .map_err(|err| err.to_string())?;
            println!("deactivated {subject}");
        }
        other => return Err(format!("unknown command {other}\n{}", usage())),
    }
    Ok(())
}
