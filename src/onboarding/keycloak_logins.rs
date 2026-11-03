//! Keycloak sign-in creation for invitations.
//!
//! An invited family member needs a Keycloak login before they can sign in and
//! accept their invitation. Realm self-registration stays off, so the API
//! creates the login itself when an administrator sends the invitation.
//!
//! [`KeycloakLoginAdapter`] wraps the configured mail adapter:
//!
//! - A person with no Keycloak login gets one under their reserved username and
//!   verified address. Keycloak then emails a single "choose your password" link
//!   (`execute-actions-email` with `UPDATE_PASSWORD`) whose redirect is the
//!   invitation's activation link. That is the only message the person receives.
//! - A person whose login already has a password gets the ordinary invitation
//!   message through the wrapped adapter. Their password is never reset.
//!
//! The adapter authenticates with a client-credentials service account. It never
//! sets, generates, reads, or logs a password, and no error carries the address,
//! the token, the secret, or a Keycloak response body. A login is matched by its
//! exact username and address only; a username held by a different address is
//! refused rather than reused.

use std::time::Duration;

use serde::Deserialize;

use crate::onboarding::delivery::{DeliveryError, DeliveryReceipt, InvitationMessage, MailAdapter};

/// The service-account settings for creating Keycloak logins.
#[derive(Clone, PartialEq, Eq)]
pub struct KeycloakLoginSettings {
    /// Keycloak base URL, for example `https://kc.example.test`. Never the
    /// realm URL.
    pub base_url: String,
    /// The realm that issues the application's tokens.
    pub realm: String,
    /// The confidential client whose service account may manage users.
    pub client_id: String,
    /// That client's secret.
    pub client_secret: String,
    /// The browser client the password link returns through. Its valid
    /// redirect URIs must include the activation URL.
    pub actions_client_id: String,
    /// How long Keycloak's password link stays valid.
    pub actions_lifespan: Duration,
    /// The bounded per-request timeout.
    pub timeout: Duration,
}

// The secret is a credential: never print it.
impl std::fmt::Debug for KeycloakLoginSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeycloakLoginSettings")
            .field("base_url", &self.base_url)
            .field("realm", &self.realm)
            .field("client_id", &self.client_id)
            .field("client_secret", &"<redacted>")
            .field("actions_client_id", &self.actions_client_id)
            .field("actions_lifespan", &self.actions_lifespan)
            .field("timeout", &self.timeout)
            .finish()
    }
}

impl KeycloakLoginSettings {
    /// The issuer this realm signs tokens with.
    pub fn issuer(&self) -> String {
        format!(
            "{}/realms/{}",
            self.base_url.trim().trim_end_matches('/'),
            self.realm.trim()
        )
    }

    fn base(&self) -> &str {
        self.base_url.trim().trim_end_matches('/')
    }

    fn users_url(&self) -> String {
        format!("{}/admin/realms/{}/users", self.base(), self.realm.trim())
    }

    fn token_url(&self) -> String {
        format!(
            "{}/realms/{}/protocol/openid-connect/token",
            self.base(),
            self.realm.trim()
        )
    }

    fn is_complete(&self) -> bool {
        [
            &self.base_url,
            &self.realm,
            &self.client_id,
            &self.client_secret,
            &self.actions_client_id,
        ]
        .iter()
        .all(|value| !value.trim().is_empty())
    }
}

/// What a login needs beyond the verified address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginProfile {
    /// The reserved application username, reused as the Keycloak username.
    pub username: String,
    /// The given name(s), when the verified name parts provide them.
    pub first_name: Option<String>,
    /// The family name, when the verified name parts provide it.
    pub last_name: Option<String>,
}

/// The Keycloak user fields this module reads.
#[derive(Debug, Clone, Deserialize)]
struct KeycloakUser {
    id: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    email: Option<String>,
    #[serde(default = "enabled_default")]
    enabled: bool,
}

fn enabled_default() -> bool {
    true
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
}

#[derive(Deserialize)]
struct Credential {
    #[serde(rename = "type", default)]
    kind: String,
}

/// The login the invitation will use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnsuredLogin {
    /// The Keycloak user id, which is the OIDC subject.
    pub user_id: String,
    /// True when this call created the login.
    pub created: bool,
    /// True when the login already has a password.
    pub has_password: bool,
}

fn login_error(code: &'static str) -> DeliveryError {
    DeliveryError::Login(code)
}

/// Keycloak ids are UUIDs. Anything else is refused before it reaches a URL.
fn safe_user_id(id: &str) -> Result<&str, DeliveryError> {
    if !id.is_empty()
        && id.len() <= 64
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        Ok(id)
    } else {
        Err(login_error("login_provider_invalid_response"))
    }
}

/// A blocking Keycloak admin client limited to the calls invitations need.
pub struct KeycloakLogins {
    settings: KeycloakLoginSettings,
}

impl KeycloakLogins {
    pub fn new(settings: KeycloakLoginSettings) -> Self {
        Self { settings }
    }

    fn http(&self) -> Result<reqwest::blocking::Client, DeliveryError> {
        reqwest::blocking::Client::builder()
            .timeout(self.settings.timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| login_error("login_provider_unavailable"))
    }

    fn access_token(&self, http: &reqwest::blocking::Client) -> Result<String, DeliveryError> {
        let response = http
            .post(self.settings.token_url())
            .form(&[
                ("grant_type", "client_credentials"),
                ("client_id", self.settings.client_id.trim()),
                ("client_secret", self.settings.client_secret.trim()),
            ])
            .send()
            .map_err(|_| login_error("login_provider_unavailable"))?;
        if !response.status().is_success() {
            return Err(login_error("login_provider_unauthorized"));
        }
        read_json::<TokenResponse>(response)
            .map(|body| body.access_token)
            .map_err(|_| login_error("login_provider_invalid_response"))
    }

    fn find(
        &self,
        http: &reqwest::blocking::Client,
        token: &str,
        field: &str,
        value: &str,
    ) -> Result<Vec<KeycloakUser>, DeliveryError> {
        let response = http
            .get(self.settings.users_url())
            .bearer_auth(token)
            .query(&[(field, value), ("exact", "true")])
            .send()
            .map_err(|_| login_error("login_provider_unavailable"))?;
        if !response.status().is_success() {
            return Err(status_error(response.status()));
        }
        let users = read_json::<Vec<KeycloakUser>>(response)
            .map_err(|_| login_error("login_provider_invalid_response"))?;
        // `exact=true` is a server-side filter; keep only true matches so an
        // older server that ignores it can never widen the result.
        Ok(users
            .into_iter()
            .filter(|user| match field {
                "username" => user.username.eq_ignore_ascii_case(value),
                "email" => user
                    .email
                    .as_deref()
                    .is_some_and(|email| email.eq_ignore_ascii_case(value)),
                _ => false,
            })
            .collect())
    }

    /// Creates the login. `Ok(None)` means Keycloak reported a conflict.
    fn create(
        &self,
        http: &reqwest::blocking::Client,
        token: &str,
        profile: &LoginProfile,
        email: &str,
    ) -> Result<Option<String>, DeliveryError> {
        let mut body = serde_json::json!({
            "username": profile.username,
            "email": email,
            // The administrator verified this address before sending, and the
            // password link proves the person controls it.
            "emailVerified": true,
            "enabled": true,
        });
        if let Some(first) = profile.first_name.as_deref() {
            body["firstName"] = serde_json::Value::from(first);
        }
        if let Some(last) = profile.last_name.as_deref() {
            body["lastName"] = serde_json::Value::from(last);
        }
        let response = http
            .post(self.settings.users_url())
            .bearer_auth(token)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body.to_string())
            .send()
            .map_err(|_| login_error("login_provider_unavailable"))?;
        let status = response.status();
        if status == reqwest::StatusCode::CONFLICT {
            return Ok(None);
        }
        if status != reqwest::StatusCode::CREATED {
            return Err(status_error(status));
        }
        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        let id = location.rsplit('/').next().unwrap_or_default();
        Ok(Some(safe_user_id(id)?.to_string()))
    }

    fn has_password(
        &self,
        http: &reqwest::blocking::Client,
        token: &str,
        user_id: &str,
    ) -> Result<bool, DeliveryError> {
        let url = format!(
            "{}/{}/credentials",
            self.settings.users_url(),
            safe_user_id(user_id)?
        );
        let response = http
            .get(url)
            .bearer_auth(token)
            .send()
            .map_err(|_| login_error("login_provider_unavailable"))?;
        if !response.status().is_success() {
            return Err(status_error(response.status()));
        }
        let credentials = read_json::<Vec<Credential>>(response)
            .map_err(|_| login_error("login_provider_invalid_response"))?;
        Ok(credentials.iter().any(|c| c.kind == "password"))
    }

    /// Asks Keycloak to email the "choose your password" link, returning to
    /// `redirect_uri` once the password is set.
    fn send_password_link(
        &self,
        http: &reqwest::blocking::Client,
        token: &str,
        user_id: &str,
        redirect_uri: &str,
    ) -> Result<(), DeliveryError> {
        let url = format!(
            "{}/{}/execute-actions-email",
            self.settings.users_url(),
            safe_user_id(user_id)?
        );
        let lifespan = self.settings.actions_lifespan.as_secs().max(60).to_string();
        let response = http
            .put(url)
            .bearer_auth(token)
            .query(&[
                ("client_id", self.settings.actions_client_id.trim()),
                ("redirect_uri", redirect_uri),
                ("lifespan", lifespan.as_str()),
            ])
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(r#"["UPDATE_PASSWORD"]"#)
            .send()
            .map_err(|_| login_error("login_provider_unavailable"))?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        // Keycloak answers 400 for a redirect URI the client does not allow and
        // 500 when its own SMTP could not send.
        Err(match status.as_u16() {
            400 => login_error("login_redirect_rejected"),
            500 => login_error("login_email_failed"),
            _ => status_error(status),
        })
    }

    /// Finds or creates the login for one invitation.
    ///
    /// Reuse requires an exact match: the reserved username with the same
    /// address, or else one login holding the address. A username already held
    /// by another address is refused. A disabled login is refused.
    pub fn ensure_login(
        &self,
        profile: &LoginProfile,
        email: &str,
    ) -> Result<EnsuredLogin, DeliveryError> {
        let http = self.http()?;
        let token = self.access_token(&http)?;
        self.ensure_login_with(&http, &token, profile, email, true)
    }

    fn ensure_login_with(
        &self,
        http: &reqwest::blocking::Client,
        token: &str,
        profile: &LoginProfile,
        email: &str,
        retry_conflict: bool,
    ) -> Result<EnsuredLogin, DeliveryError> {
        let by_username = self.find(http, token, "username", &profile.username)?;
        if let Some(user) = by_username.first() {
            let same_address = user
                .email
                .as_deref()
                .is_some_and(|held| held.eq_ignore_ascii_case(email));
            if !same_address {
                return Err(login_error("login_username_taken"));
            }
            return self.reuse(http, token, user);
        }
        let by_email = self.find(http, token, "email", email)?;
        match by_email.as_slice() {
            [user] => return self.reuse(http, token, user),
            [] => {}
            _ => return Err(login_error("login_ambiguous")),
        }
        match self.create(http, token, profile, email)? {
            Some(user_id) => Ok(EnsuredLogin {
                user_id,
                created: true,
                has_password: false,
            }),
            // A concurrent send created it first; look once more.
            None if retry_conflict => self.ensure_login_with(http, token, profile, email, false),
            None => Err(login_error("login_conflict")),
        }
    }

    fn reuse(
        &self,
        http: &reqwest::blocking::Client,
        token: &str,
        user: &KeycloakUser,
    ) -> Result<EnsuredLogin, DeliveryError> {
        if !user.enabled {
            return Err(login_error("login_disabled"));
        }
        let user_id = safe_user_id(&user.id)?.to_string();
        let has_password = self.has_password(http, token, &user_id)?;
        Ok(EnsuredLogin {
            user_id,
            created: false,
            has_password,
        })
    }
}

/// Decodes a JSON body. Any failure is one safe code; the body is never echoed.
fn read_json<T: serde::de::DeserializeOwned>(
    response: reqwest::blocking::Response,
) -> Result<T, ()> {
    let text = response.text().map_err(|_| ())?;
    serde_json::from_str(&text).map_err(|_| ())
}

fn status_error(status: reqwest::StatusCode) -> DeliveryError {
    match status.as_u16() {
        401 | 403 => login_error("login_provider_unauthorized"),
        _ => login_error("login_provider_failed"),
    }
}

/// Wraps a mail adapter so every invitation also has a Keycloak login.
pub struct KeycloakLoginAdapter {
    logins: KeycloakLogins,
    configured: bool,
    inner: Box<dyn MailAdapter>,
}

impl KeycloakLoginAdapter {
    /// `inner` delivers the ordinary invitation to a person who already has a
    /// password. It may be unconfigured; that person's send then fails closed.
    pub fn new(settings: KeycloakLoginSettings, inner: Box<dyn MailAdapter>) -> Self {
        let configured = settings.is_complete();
        Self {
            logins: KeycloakLogins::new(settings),
            configured,
            inner,
        }
    }
}

impl MailAdapter for KeycloakLoginAdapter {
    fn name(&self) -> &'static str {
        "keycloak"
    }

    fn is_configured(&self) -> bool {
        self.configured
    }

    fn provisions_logins(&self) -> bool {
        true
    }

    fn deliver(
        &self,
        to: &str,
        subject: &str,
        body: &str,
    ) -> Result<DeliveryReceipt, DeliveryError> {
        self.inner.deliver(to, subject, body)
    }

    fn deliver_invitation(
        &self,
        message: &InvitationMessage<'_>,
    ) -> Result<DeliveryReceipt, DeliveryError> {
        if !self.configured {
            return Err(DeliveryError::NotConfigured);
        }
        let profile = message
            .login
            .ok_or(login_error("login_username_unavailable"))?;
        let login = self.logins.ensure_login(profile, message.to)?;
        if login.has_password {
            // An existing password is never reset. The person signs in as
            // usual and opens the ordinary invitation link.
            if !self.inner.is_configured() {
                return Err(DeliveryError::NotConfigured);
            }
            let receipt = self
                .inner
                .deliver(message.to, message.subject, message.body)?;
            return Ok(DeliveryReceipt {
                provider: format!("keycloak+{}", receipt.provider),
                ..receipt
            });
        }
        let http = self.logins.http()?;
        let token = self.logins.access_token(&http)?;
        self.logins
            .send_password_link(&http, &token, &login.user_id, message.link)?;
        // Keycloak answers only after its own SMTP accepted the message. That is
        // server acceptance, not a claim the message reached an inbox.
        Ok(DeliveryReceipt {
            provider: "keycloak".to_string(),
            accepted: true,
            message_id: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::onboarding::delivery::FakeAdapter;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    #[derive(Debug, Clone)]
    struct Seen {
        method: String,
        target: String,
        body: String,
    }

    type Route = Box<dyn Fn(&str, &str) -> (u16, Vec<(String, String)>, String) + Send>;

    /// A scripted Keycloak stand-in. Each connection carries one request; the
    /// router answers by method and path. Requests are recorded in order.
    fn fake_keycloak(route: Route) -> (String, Arc<Mutex<Vec<Seen>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake keycloak");
        let base = format!("http://{}", listener.local_addr().expect("address"));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut raw = Vec::new();
                let mut buffer = [0u8; 4096];
                let header_end = loop {
                    let read = stream.read(&mut buffer).unwrap_or(0);
                    if read == 0 {
                        break None;
                    }
                    raw.extend_from_slice(&buffer[..read]);
                    if let Some(pos) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                        break Some(pos + 4);
                    }
                };
                let Some(header_end) = header_end else {
                    continue;
                };
                let head = String::from_utf8_lossy(&raw[..header_end]).to_string();
                let length = head
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())?
                    })
                    .unwrap_or(0);
                while raw.len() < header_end + length {
                    let read = stream.read(&mut buffer).unwrap_or(0);
                    if read == 0 {
                        break;
                    }
                    raw.extend_from_slice(&buffer[..read]);
                }
                let body = String::from_utf8_lossy(&raw[header_end..]).to_string();
                let mut first = head.lines().next().unwrap_or_default().split(' ');
                let method = first.next().unwrap_or_default().to_string();
                let target = first.next().unwrap_or_default().to_string();
                let (status, headers, reply) = route(&method, &target);
                log.lock().unwrap().push(Seen {
                    method,
                    target,
                    body,
                });
                let mut response = format!(
                    "HTTP/1.1 {status} X\r\ncontent-length: {}\r\ncontent-type: application/json\r\nconnection: close\r\n",
                    reply.len()
                );
                for (name, value) in headers {
                    response.push_str(&format!("{name}: {value}\r\n"));
                }
                response.push_str("\r\n");
                response.push_str(&reply);
                let _ = stream.write_all(response.as_bytes());
            }
        });
        (base, seen)
    }

    fn settings(base: &str) -> KeycloakLoginSettings {
        KeycloakLoginSettings {
            base_url: base.to_string(),
            realm: "tran".to_string(),
            client_id: "tran-invitations".to_string(),
            client_secret: "synthetic-secret".to_string(),
            actions_client_id: "tran-web".to_string(),
            actions_lifespan: Duration::from_secs(3600),
            timeout: Duration::from_secs(5),
        }
    }

    fn profile() -> LoginProfile {
        LoginProfile {
            username: "nvan".to_string(),
            first_name: Some("Van An".to_string()),
            last_name: Some("Nguyen".to_string()),
        }
    }

    const USER_ID: &str = "0b7c1d2e-3f40-4a5b-8c6d-7e8f9a0b1c2d";
    const LINK: &str = "https://archive.example.test/activate/abc123";

    fn message<'a>(login: &'a LoginProfile) -> InvitationMessage<'a> {
        InvitationMessage {
            to: "an@example.test",
            subject: "Activate your family archive account",
            body: "Open https://archive.example.test/activate/abc123",
            link: LINK,
            login: Some(login),
        }
    }

    fn token_reply() -> (u16, Vec<(String, String)>, String) {
        (
            200,
            vec![],
            r#"{"access_token":"synthetic-access"}"#.to_string(),
        )
    }

    /// Routes for a realm where the given users already exist.
    fn realm(
        by_username: &'static str,
        by_email: &'static str,
        credentials: &'static str,
        actions_status: u16,
    ) -> Route {
        Box::new(move |method, target| {
            let path = target.split('?').next().unwrap_or_default();
            match (method, path) {
                ("POST", "/realms/tran/protocol/openid-connect/token") => token_reply(),
                ("GET", "/admin/realms/tran/users") if target.contains("username=") => {
                    (200, vec![], by_username.to_string())
                }
                ("GET", "/admin/realms/tran/users") if target.contains("email=") => {
                    (200, vec![], by_email.to_string())
                }
                ("POST", "/admin/realms/tran/users") => (
                    201,
                    vec![(
                        "location".to_string(),
                        format!("http://kc/admin/realms/tran/users/{USER_ID}"),
                    )],
                    String::new(),
                ),
                ("GET", p) if p.ends_with("/credentials") => (200, vec![], credentials.to_string()),
                ("PUT", p) if p.ends_with("/execute-actions-email") => {
                    (actions_status, vec![], String::new())
                }
                _ => (404, vec![], String::new()),
            }
        })
    }

    fn calls(seen: &Arc<Mutex<Vec<Seen>>>) -> Vec<String> {
        seen.lock()
            .unwrap()
            .iter()
            .map(|s| format!("{} {}", s.method, s.target.split('?').next().unwrap()))
            .collect()
    }

    #[test]
    fn a_new_person_gets_a_login_and_one_password_email() {
        let (base, seen) = fake_keycloak(realm("[]", "[]", "[]", 204));
        let inner = FakeAdapter::accepting();
        let adapter = KeycloakLoginAdapter::new(settings(&base), Box::new(inner));
        let login = profile();
        let receipt = adapter
            .deliver_invitation(&message(&login))
            .expect("delivered");
        assert_eq!(receipt.provider, "keycloak");
        assert!(receipt.accepted);

        let seen = seen.lock().unwrap().clone();
        let create = seen
            .iter()
            .find(|s| s.method == "POST" && s.target == "/admin/realms/tran/users")
            .expect("user created");
        let created: serde_json::Value = serde_json::from_str(&create.body).unwrap();
        assert_eq!(created["username"], "nvan");
        assert_eq!(created["email"], "an@example.test");
        assert_eq!(created["emailVerified"], true);
        assert_eq!(created["firstName"], "Van An");
        assert_eq!(created["lastName"], "Nguyen");
        assert!(
            created.get("credentials").is_none(),
            "no password is ever set"
        );

        let actions = seen
            .iter()
            .find(|s| s.target.contains("/execute-actions-email"))
            .expect("password email requested");
        assert!(actions.target.contains(&format!("/users/{USER_ID}/")));
        assert!(actions.target.contains("client_id=tran-web"));
        assert!(actions
            .target
            .contains("redirect_uri=https%3A%2F%2Farchive.example.test%2Factivate%2Fabc123"));
        assert_eq!(actions.body, r#"["UPDATE_PASSWORD"]"#);
    }

    #[test]
    fn the_wrapped_adapter_sends_nothing_for_a_new_login() {
        let (base, _seen) = fake_keycloak(realm("[]", "[]", "[]", 204));
        let inner = Arc::new(FakeAdapter::accepting());
        struct Shared(Arc<FakeAdapter>);
        impl MailAdapter for Shared {
            fn name(&self) -> &'static str {
                "fake"
            }
            fn deliver(&self, t: &str, s: &str, b: &str) -> Result<DeliveryReceipt, DeliveryError> {
                self.0.deliver(t, s, b)
            }
        }
        let adapter =
            KeycloakLoginAdapter::new(settings(&base), Box::new(Shared(Arc::clone(&inner))));
        let login = profile();
        adapter
            .deliver_invitation(&message(&login))
            .expect("delivered");
        assert!(inner.messages().is_empty(), "one email only: Keycloak's");
    }

    #[test]
    fn an_existing_login_with_a_password_gets_the_ordinary_invitation() {
        let existing = r#"[{"id":"0b7c1d2e-3f40-4a5b-8c6d-7e8f9a0b1c2d","username":"nvan","email":"AN@example.test","enabled":true}]"#;
        let (base, seen) = fake_keycloak(realm(existing, "[]", r#"[{"type":"password"}]"#, 204));
        let inner = Arc::new(FakeAdapter::accepting());
        struct Shared(Arc<FakeAdapter>);
        impl MailAdapter for Shared {
            fn name(&self) -> &'static str {
                "fake"
            }
            fn deliver(&self, t: &str, s: &str, b: &str) -> Result<DeliveryReceipt, DeliveryError> {
                self.0.deliver(t, s, b)
            }
        }
        let adapter =
            KeycloakLoginAdapter::new(settings(&base), Box::new(Shared(Arc::clone(&inner))));
        let login = profile();
        let receipt = adapter
            .deliver_invitation(&message(&login))
            .expect("delivered");
        assert_eq!(receipt.provider, "keycloak+fake");
        assert_eq!(inner.messages().len(), 1);
        let calls = calls(&seen);
        assert!(
            !calls.iter().any(|c| c.starts_with("POST /admin")),
            "no second login"
        );
        assert!(
            !calls.iter().any(|c| c.contains("execute-actions-email")),
            "an existing password is never reset"
        );
    }

    #[test]
    fn an_existing_login_with_a_password_fails_closed_without_mail() {
        let existing = r#"[{"id":"0b7c1d2e-3f40-4a5b-8c6d-7e8f9a0b1c2d","username":"nvan","email":"an@example.test"}]"#;
        let (base, _seen) = fake_keycloak(realm(existing, "[]", r#"[{"type":"password"}]"#, 204));
        let adapter = KeycloakLoginAdapter::new(
            settings(&base),
            Box::new(crate::onboarding::delivery::UnconfiguredAdapter),
        );
        let login = profile();
        assert_eq!(
            adapter.deliver_invitation(&message(&login)),
            Err(DeliveryError::NotConfigured)
        );
    }

    #[test]
    fn a_login_created_earlier_without_a_password_is_reused_on_resend() {
        let existing = r#"[{"id":"0b7c1d2e-3f40-4a5b-8c6d-7e8f9a0b1c2d","username":"nvan","email":"an@example.test"}]"#;
        let (base, seen) = fake_keycloak(realm(existing, "[]", "[]", 204));
        let adapter =
            KeycloakLoginAdapter::new(settings(&base), Box::new(FakeAdapter::accepting()));
        let login = profile();
        adapter
            .deliver_invitation(&message(&login))
            .expect("delivered");
        let calls = calls(&seen);
        assert!(!calls.iter().any(|c| c == "POST /admin/realms/tran/users"));
        assert!(calls.iter().any(|c| c.contains("execute-actions-email")));
    }

    #[test]
    fn a_login_found_by_address_is_reused() {
        let by_email = r#"[{"id":"0b7c1d2e-3f40-4a5b-8c6d-7e8f9a0b1c2d","username":"an.nguyen","email":"an@example.test"}]"#;
        let (base, seen) = fake_keycloak(realm("[]", by_email, "[]", 204));
        let adapter =
            KeycloakLoginAdapter::new(settings(&base), Box::new(FakeAdapter::accepting()));
        let login = profile();
        adapter
            .deliver_invitation(&message(&login))
            .expect("delivered");
        assert!(!calls(&seen)
            .iter()
            .any(|c| c == "POST /admin/realms/tran/users"));
    }

    #[test]
    fn a_username_held_by_another_address_is_refused() {
        let other = r#"[{"id":"0b7c1d2e-3f40-4a5b-8c6d-7e8f9a0b1c2d","username":"nvan","email":"someone@example.test"}]"#;
        let (base, seen) = fake_keycloak(realm(other, "[]", "[]", 204));
        let adapter =
            KeycloakLoginAdapter::new(settings(&base), Box::new(FakeAdapter::accepting()));
        let login = profile();
        assert_eq!(
            adapter.deliver_invitation(&message(&login)),
            Err(DeliveryError::Login("login_username_taken"))
        );
        let calls = calls(&seen);
        assert!(!calls
            .iter()
            .any(|c| c.starts_with("POST /admin") || c.starts_with("PUT")));
    }

    #[test]
    fn two_logins_holding_the_address_are_refused() {
        let two = r#"[{"id":"a","username":"x","email":"an@example.test"},{"id":"b","username":"y","email":"an@example.test"}]"#;
        let (base, _seen) = fake_keycloak(realm("[]", two, "[]", 204));
        let adapter =
            KeycloakLoginAdapter::new(settings(&base), Box::new(FakeAdapter::accepting()));
        let login = profile();
        assert_eq!(
            adapter.deliver_invitation(&message(&login)),
            Err(DeliveryError::Login("login_ambiguous"))
        );
    }

    #[test]
    fn a_disabled_login_is_refused() {
        let disabled = r#"[{"id":"0b7c1d2e-3f40-4a5b-8c6d-7e8f9a0b1c2d","username":"nvan","email":"an@example.test","enabled":false}]"#;
        let (base, _seen) = fake_keycloak(realm(disabled, "[]", "[]", 204));
        let adapter =
            KeycloakLoginAdapter::new(settings(&base), Box::new(FakeAdapter::accepting()));
        let login = profile();
        assert_eq!(
            adapter.deliver_invitation(&message(&login)),
            Err(DeliveryError::Login("login_disabled"))
        );
    }

    #[test]
    fn keycloak_errors_map_to_safe_codes() {
        for (status, code) in [
            (400, "login_redirect_rejected"),
            (500, "login_email_failed"),
            (403, "login_provider_unauthorized"),
        ] {
            let (base, _seen) = fake_keycloak(realm("[]", "[]", "[]", status));
            let adapter =
                KeycloakLoginAdapter::new(settings(&base), Box::new(FakeAdapter::accepting()));
            let login = profile();
            let err = adapter.deliver_invitation(&message(&login)).unwrap_err();
            assert_eq!(err, DeliveryError::Login(code));
            let text = err.to_string();
            assert!(!text.contains("example.test") && !text.contains("synthetic"));
        }
    }

    #[test]
    fn a_refused_service_account_creates_nothing() {
        let (base, seen) = fake_keycloak(Box::new(|_, _| (401, vec![], String::new())));
        let adapter =
            KeycloakLoginAdapter::new(settings(&base), Box::new(FakeAdapter::accepting()));
        let login = profile();
        assert_eq!(
            adapter.deliver_invitation(&message(&login)),
            Err(DeliveryError::Login("login_provider_unauthorized"))
        );
        assert_eq!(calls(&seen).len(), 1, "only the token request was made");
    }

    #[test]
    fn an_unreachable_keycloak_fails_closed() {
        let adapter = KeycloakLoginAdapter::new(
            settings("http://127.0.0.1:9"),
            Box::new(FakeAdapter::accepting()),
        );
        let login = profile();
        assert_eq!(
            adapter.deliver_invitation(&message(&login)),
            Err(DeliveryError::Login("login_provider_unavailable"))
        );
    }

    #[test]
    fn a_missing_login_profile_is_refused() {
        let adapter = KeycloakLoginAdapter::new(
            settings("http://127.0.0.1:9"),
            Box::new(FakeAdapter::accepting()),
        );
        let empty = LoginProfile {
            username: String::new(),
            first_name: None,
            last_name: None,
        };
        let mut msg = message(&empty);
        msg.login = None;
        assert_eq!(
            adapter.deliver_invitation(&msg),
            Err(DeliveryError::Login("login_username_unavailable"))
        );
    }

    #[test]
    fn incomplete_settings_are_not_configured() {
        let mut incomplete = settings("http://127.0.0.1:9");
        incomplete.client_secret = " ".to_string();
        let adapter = KeycloakLoginAdapter::new(incomplete, Box::new(FakeAdapter::accepting()));
        assert!(!adapter.is_configured());
        let login = profile();
        assert_eq!(
            adapter.deliver_invitation(&message(&login)),
            Err(DeliveryError::NotConfigured)
        );
    }

    #[test]
    fn a_non_uuid_location_is_refused() {
        let (base, _seen) = fake_keycloak(Box::new(|method, target| {
            let path = target.split('?').next().unwrap_or_default().to_string();
            match (method, path.as_str()) {
                ("POST", "/realms/tran/protocol/openid-connect/token") => token_reply(),
                ("GET", _) => (200, vec![], "[]".to_string()),
                ("POST", _) => (
                    201,
                    vec![(
                        "location".to_string(),
                        "http://kc/users/../../x?y".to_string(),
                    )],
                    String::new(),
                ),
                _ => (204, vec![], String::new()),
            }
        }));
        let adapter =
            KeycloakLoginAdapter::new(settings(&base), Box::new(FakeAdapter::accepting()));
        let login = profile();
        assert_eq!(
            adapter.deliver_invitation(&message(&login)),
            Err(DeliveryError::Login("login_provider_invalid_response"))
        );
    }

    #[test]
    fn debug_output_redacts_the_secret() {
        let text = format!("{:?}", settings("http://kc"));
        assert!(!text.contains("synthetic-secret"));
        assert!(text.contains("<redacted>"));
    }

    #[test]
    fn issuer_matches_the_realm_url() {
        let mut s = settings("https://kc.example.test/");
        s.realm = "tran".into();
        assert_eq!(s.issuer(), "https://kc.example.test/realms/tran");
    }
}
