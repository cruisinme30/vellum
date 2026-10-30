// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Second factors: authenticator apps, passkeys and recovery codes
//! (ADR 0069, D290).
//!
//! - A password sign-in by someone with a factor returns a **ticket**
//!   (`vmt_…`, five minutes) instead of a session. The ticket is redeemed
//!   with a code, a recovery code or a passkey assertion; wrong answers
//!   count towards the account's sign-in lockout, and the ticket ends after
//!   [`MAX_TICKET_FAILURES`].
//! - When the workspace policy covers someone without a factor, the ticket
//!   is an **enrolment** ticket: it adds a factor, and only then opens the
//!   session.
//! - Re-authentication asks for a factor too, when the person has one.
//!
//! Recovery codes are kept as hashes; authenticator secrets sealed
//! ([`crate::seal`]).

use std::net::IpAddr;
use std::time::{Duration, Instant};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vellum_kernel::{Error, ErrorKind, Timestamp, UserId};

use crate::identity::{Identity, NewSession, after};
use crate::lockout::Key;
use crate::secret::{self, Kind};
use crate::store::{
    ChallengePurpose, MfaChallenge, Passkey, PendingTotp, SecondFactors,
    SessionMethod, TotpFactor, User, hash_key,
};
use crate::totp;
use crate::webauthn::{self, Assertion, Registration};

/// How long a sign-in ticket or passkey ceremony lasts.
pub const TICKET_LIFETIME: Duration = Duration::from_mins(5);
/// How long an authenticator app offered for setup waits for its first code.
const PENDING_TOTP_LIFETIME: Duration = Duration::from_mins(10);
/// Wrong answers a ticket survives.
pub const MAX_TICKET_FAILURES: u32 = 5;
/// Recovery codes per set.
pub const RECOVERY_CODES: usize = 10;
/// The most passkeys one person keeps.
const MAX_PASSKEYS: usize = 20;
/// The issuer authenticator apps show.
const ISSUER: &str = "Vellum";

/// What a password sign-in leads to.
#[derive(Debug, Clone)]
pub enum SignIn {
    /// Signed in.
    Session(NewSession),
    /// A second factor is needed: redeem the ticket with one.
    Verify(Ticket),
    /// The workspace requires a second factor and the person has none: add
    /// one with the ticket, which then signs in.
    Enrol(Ticket),
}

/// A sign-in ticket, as the browser gets it.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Ticket {
    /// The ticket, `vmt_…`. It works for five minutes.
    pub ticket: String,
    /// The factors that can redeem it: `totp`, `passkey`, `recovery`.
    pub methods: Vec<String>,
}

/// What proves the second factor.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Proof {
    /// A 6-digit app code or a recovery code.
    #[serde(default)]
    pub code: Option<String>,
    /// A passkey assertion for the ticket's challenge.
    #[serde(default)]
    pub passkey: Option<Assertion>,
}

/// A new authenticator app: what to scan or type into it.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TotpSetup {
    /// The secret in base32, to type in by hand.
    pub secret: String,
    /// The `otpauth://` link, which a phone opens in its app.
    pub uri: String,
    /// The link as a QR code: one string per row, `1` for a dark module.
    pub qr: Vec<String>,
}

/// A person's factors, as their settings page shows them.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FactorsView {
    /// Whether an authenticator app is set up.
    pub totp: bool,
    /// Registered passkeys.
    pub passkeys: Vec<PasskeyView>,
    /// Recovery codes not used yet.
    pub recovery_codes_left: usize,
    /// Whether the workspace requires a factor of this person.
    pub required: bool,
}

/// A passkey as listed.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PasskeyView {
    /// The credential ID.
    pub id: String,
    /// Its label.
    pub name: String,
    /// When it was added.
    pub created_at: Timestamp,
    /// When it last signed in.
    pub last_used_at: Option<Timestamp>,
}

/// A factor just added.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FactorAdded {
    /// The person's factors now.
    pub factors: FactorsView,
    /// New recovery codes, when this was the first factor. Shown once.
    pub recovery_codes: Option<Vec<String>>,
}

/// A `WebAuthn` ceremony to run in the browser.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PasskeyCeremony {
    /// The ticket that answers it.
    pub ticket: String,
    /// The options for `navigator.credentials.create()` or `.get()`, with
    /// binary fields in base64url.
    pub options: Value,
}

fn no_ticket() -> Error {
    Error::new(
        ErrorKind::Unauthenticated,
        "the sign-in step expired; sign in again",
    )
}

fn wrong_code() -> Error {
    Error::new(ErrorKind::Unauthenticated, "the code is wrong")
}

/// The answer re-authentication gives when a factor is missing.
pub const SECOND_FACTOR_REQUIRED: &str = "second factor required";

fn factor_policy_conflict() -> Error {
    Error::new(
        ErrorKind::Conflict,
        "your workspace requires two-step verification; add another method \
            first",
    )
}

fn unix_seconds() -> u64 {
    u64::try_from(Timestamp::now().unix_ms() / 1000).unwrap_or(0)
}

/// Lower-case, without spaces or dashes: how recovery codes compare.
fn normalise_recovery(code: &str) -> String {
    code.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

fn recovery_hash(code: &str) -> String {
    hash_key(&secret::hash(&normalise_recovery(code)))
}

/// Ten new recovery codes (`xxxxx-xxxxx`, 50 bits each) and their hashes.
fn recovery_codes() -> Result<(Vec<String>, Vec<String>), Error> {
    let mut codes = Vec::with_capacity(RECOVERY_CODES);
    for _ in 0..RECOVERY_CODES {
        let mut bytes = [0_u8; 7];
        getrandom::fill(&mut bytes).map_err(|_| {
            Error::new(ErrorKind::Internal, "random source failed")
        })?;
        let text = data_encoding::BASE32_NOPAD
            .encode(&bytes)
            .to_ascii_lowercase();
        let (a, rest) = text.split_at_checked(5).unwrap_or((&text, ""));
        let b = rest.get(..5).unwrap_or(rest);
        codes.push(format!("{a}-{b}"));
    }
    let hashes = codes.iter().map(|c| recovery_hash(c)).collect();
    Ok((codes, hashes))
}

/// The QR code of `text`, one string of `0`s and `1`s per row.
fn qr_rows(text: &str) -> Vec<String> {
    use qrcodegen::{QrCode, QrCodeEcc};
    QrCode::encode_text(text, QrCodeEcc::Medium).map_or_else(
        |_| Vec::new(),
        |qr| {
            let size = qr.size();
            (0..size)
                .map(|y| {
                    (0..size)
                        .map(|x| if qr.get_module(x, y) { '1' } else { '0' })
                        .collect()
                })
                .collect()
        },
    )
}

fn passkey_view(p: &Passkey) -> PasskeyView {
    PasskeyView {
        id: p.id.clone(),
        name: p.name.clone(),
        created_at: p.created_at,
        last_used_at: p.last_used_at,
    }
}

fn check_passkey_name(name: &str) -> Result<String, Error> {
    let name = name.trim();
    if name.is_empty()
        || name.chars().count() > 60
        || name.chars().any(char::is_control)
    {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "name the passkey in 1 to 60 characters",
        ));
    }
    Ok(name.to_owned())
}

/// What a ticket was checked against.
enum Checked {
    /// A matching app code, for this step.
    Totp(u64),
    /// A recovery code, by hash.
    Recovery(String),
    /// A passkey, by credential ID, with its new counter.
    Passkey(String, u32),
}

impl Identity {
    /// Whether the workspace requires `user` to use a second factor.
    pub(crate) fn factor_required(&self, user: &User) -> bool {
        // Break-glass admins always need one while single sign-on is
        // required: their password alone must not open the workspace.
        self.store().read(|s| {
            s.sign_in.second_factor.covers(user)
                || (s.sign_in.require_sso && user.admin && user.break_glass)
        })
    }

    /// The step after a correct password: a session, or a ticket for a
    /// second factor or for adding one.
    pub(crate) fn after_password(
        &self,
        user: &User,
        user_agent: Option<&str>,
    ) -> Result<SignIn, Error> {
        if user.factors.any() {
            let mut methods = Vec::new();
            if user.factors.totp.is_some() {
                methods.push("totp".to_owned());
            }
            if !user.factors.passkeys.is_empty() {
                methods.push("passkey".to_owned());
            }
            if !user.factors.recovery.is_empty() {
                methods.push("recovery".to_owned());
            }
            let ticket = self.new_challenge(
                user.id,
                ChallengePurpose::SignIn,
                user_agent,
                None,
            )?;
            return Ok(SignIn::Verify(Ticket { ticket, methods }));
        }
        if self.factor_required(user) {
            let ticket = self.new_challenge(
                user.id,
                ChallengePurpose::Enrol,
                user_agent,
                None,
            )?;
            return Ok(SignIn::Enrol(Ticket {
                ticket,
                methods: vec!["totp".to_owned(), "passkey".to_owned()],
            }));
        }
        self.open_session(
            user.id,
            SessionMethod::Password,
            self.config().session_lifetime,
            user_agent,
        )
        .map(SignIn::Session)
    }

    fn new_challenge(
        &self,
        user: UserId,
        purpose: ChallengePurpose,
        user_agent: Option<&str>,
        session: Option<String>,
    ) -> Result<String, Error> {
        let ticket = secret::generate(Kind::MfaTicket)?;
        let now = Timestamp::now();
        let record = MfaChallenge {
            user,
            purpose,
            webauthn: None,
            failures: 0,
            expires: after(now, TICKET_LIFETIME),
            user_agent: user_agent.map(|ua| ua.chars().take(256).collect()),
            session,
        };
        let key = hash_key(&secret::hash(&ticket));
        self.store().write(|s| {
            s.mfa_challenges.retain(|_, c| c.expires > now);
            let _ = s.mfa_challenges.insert(key, record);
        })?;
        Ok(ticket)
    }

    /// The live challenge behind a ticket, if it has one of `purposes`.
    fn challenge(
        &self,
        ticket: &str,
        purposes: &[ChallengePurpose],
    ) -> Result<(String, MfaChallenge, User), Error> {
        if !secret::well_formed(ticket, Kind::MfaTicket) {
            return Err(no_ticket());
        }
        let key = hash_key(&secret::hash(ticket));
        let now = Timestamp::now();
        self.store()
            .read(|s| {
                let c = s.mfa_challenges.get(&key)?;
                let user = s.users.get(&c.user)?;
                (c.expires > now
                    && purposes.contains(&c.purpose)
                    && !user.disabled)
                    .then(|| (key.clone(), c.clone(), user.clone()))
            })
            .ok_or_else(no_ticket)
    }

    /// Checks a proof against a user's factors, without changing anything.
    fn check_proof(
        &self,
        user: &User,
        challenge: Option<&str>,
        proof: &Proof,
    ) -> Result<Option<Checked>, Error> {
        if let Some(assertion) = &proof.passkey {
            let Some(challenge) = challenge else {
                return Ok(None);
            };
            let Some(passkey) =
                user.factors.passkeys.iter().find(|p| p.id == assertion.id)
            else {
                return Ok(None);
            };
            return Ok(webauthn::verify_assertion(
                &self.relying_party(),
                challenge,
                &passkey.key,
                passkey.sign_count,
                assertion,
            )
            .ok()
            .map(|count| Checked::Passkey(passkey.id.clone(), count)));
        }
        let Some(code) = proof.code.as_deref() else {
            return Ok(None);
        };
        let digits: String =
            code.chars().filter(|c| !c.is_whitespace()).collect();
        if digits.len() == 6 && digits.bytes().all(|b| b.is_ascii_digit()) {
            let Some(app) = &user.factors.totp else {
                return Ok(None);
            };
            let secret =
                self.sealer()?.open(&totp_purpose(user.id), &app.sealed)?;
            return Ok(totp::verify(
                &secret,
                &digits,
                totp::step_at(unix_seconds()),
                app.last_step,
            )
            .map(Checked::Totp));
        }
        let hash = recovery_hash(code);
        Ok(user
            .factors
            .recovery
            .iter()
            .any(|h| h == &hash)
            .then_some(Checked::Recovery(hash)))
    }

    /// Records a used factor: the app's step, the spent recovery code, or
    /// the passkey's counter. Fails when another request used it first.
    fn spend(&self, user: UserId, checked: &Checked) -> Result<(), Error> {
        let now = Timestamp::now();
        self.store().write(|s| {
            let Some(u) = s.users.get_mut(&user) else {
                return Err(wrong_code());
            };
            match checked {
                Checked::Totp(step) => match &mut u.factors.totp {
                    Some(app)
                        if app.last_step.is_none_or(|last| *step > last) =>
                    {
                        app.last_step = Some(*step);
                    }
                    _ => return Err(wrong_code()),
                },
                Checked::Recovery(hash) => {
                    let before = u.factors.recovery.len();
                    u.factors.recovery.retain(|h| h != hash);
                    if u.factors.recovery.len() == before {
                        return Err(wrong_code());
                    }
                }
                Checked::Passkey(id, count) => {
                    let Some(p) =
                        u.factors.passkeys.iter_mut().find(|p| &p.id == id)
                    else {
                        return Err(wrong_code());
                    };
                    if (*count != 0 || p.sign_count != 0)
                        && *count <= p.sign_count
                    {
                        return Err(wrong_code());
                    }
                    p.sign_count = *count;
                    p.last_used_at = Some(now);
                }
            }
            Ok(())
        })?
    }

    /// Counts a wrong answer against the ticket and the account's lockout.
    fn fail_ticket(&self, key: &str, email: &str, ip: Option<IpAddr>) {
        let at = Instant::now();
        self.lockouts.fail(self.store(), Key::Email(email), at);
        if let Some(ip) = ip {
            self.lockouts.fail(self.store(), Key::Ip(ip), at);
        }
        let _ = self.store().write(|s| {
            let spent = s.mfa_challenges.get_mut(key).is_some_and(|c| {
                c.failures += 1;
                c.failures >= MAX_TICKET_FAILURES
            });
            if spent {
                let _ = s.mfa_challenges.remove(key);
            }
        });
        tracing::info!(
            event = "identity.second_factor_failed",
            "second factor refused"
        );
    }

    fn check_lockout(
        &self,
        email: &str,
        ip: Option<IpAddr>,
    ) -> Result<(), Error> {
        let now = Instant::now();
        if let Some(ip) = ip {
            self.lockouts
                .check(self.store(), Key::Ip(ip), now)
                .map_err(crate::identity::locked)?;
        }
        self.lockouts
            .check(self.store(), Key::Email(email), now)
            .map_err(crate::identity::locked)
    }

    /// Redeems a sign-in ticket with a second factor and opens the session.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Unauthenticated`] for an unknown or spent ticket or a
    /// wrong proof, and [`ErrorKind::RateLimited`] while locked out.
    pub fn verify_sign_in(
        &self,
        ticket: &str,
        proof: &Proof,
        ip: Option<IpAddr>,
    ) -> Result<NewSession, Error> {
        let (key, challenge, user) =
            self.challenge(ticket, &[ChallengePurpose::SignIn])?;
        self.check_lockout(&user.email, ip)?;
        let Some(checked) =
            self.check_proof(&user, challenge.webauthn.as_deref(), proof)?
        else {
            self.fail_ticket(&key, &user.email, ip);
            return Err(wrong_code());
        };
        // One redemption per ticket, even when two arrive at once.
        let taken = self
            .store()
            .write(|s| s.mfa_challenges.remove(&key).is_some())?;
        if !taken {
            return Err(no_ticket());
        }
        self.spend(user.id, &checked)?;
        self.lockouts.succeed(self.store(), Key::Email(&user.email));
        tracing::info!(
            event = "identity.signed_in",
            user = %user.id,
            second_factor = true,
            "signed in"
        );
        self.open_session(
            user.id,
            SessionMethod::Password,
            self.config().session_lifetime,
            challenge.user_agent.as_deref(),
        )
    }

    /// The relying party passkeys are bound to: the public URL.
    pub(crate) fn relying_party(&self) -> webauthn::RelyingParty {
        let url = &self.config().public_url;
        webauthn::RelyingParty {
            id: url.host_str().unwrap_or("localhost").to_owned(),
            origin: url.origin().ascii_serialization(),
        }
    }

    fn webauthn_challenge(&self, key: &str) -> Result<String, Error> {
        let mut bytes = [0_u8; 32];
        getrandom::fill(&mut bytes).map_err(|_| {
            Error::new(ErrorKind::Internal, "random source failed")
        })?;
        let challenge = webauthn::encode(&bytes);
        let stored = challenge.clone();
        self.store().write(|s| {
            s.mfa_challenges
                .get_mut(key)
                .map(|c| c.webauthn = Some(stored))
                .ok_or_else(no_ticket)
        })??;
        Ok(challenge)
    }

    fn request_options(&self, user: &User, challenge: &str) -> Value {
        let allow: Vec<Value> = user
            .factors
            .passkeys
            .iter()
            .map(|p| json!({ "type": "public-key", "id": p.id }))
            .collect();
        json!({
            "challenge": challenge,
            "rpId": self.relying_party().id,
            "allowCredentials": allow,
            "userVerification": "preferred",
            "timeout": 300_000,
        })
    }

    fn creation_options(&self, user: &User, challenge: &str) -> Value {
        let exclude: Vec<Value> = user
            .factors
            .passkeys
            .iter()
            .map(|p| json!({ "type": "public-key", "id": p.id }))
            .collect();
        json!({
            "challenge": challenge,
            "rp": { "id": self.relying_party().id, "name": "Vellum" },
            "user": {
                "id": webauthn::encode(user.id.to_string().as_bytes()),
                "name": user.email,
                "displayName": user.name,
            },
            "pubKeyCredParams": [
                { "type": "public-key", "alg": webauthn::ES256 },
                { "type": "public-key", "alg": webauthn::RS256 },
            ],
            "excludeCredentials": exclude,
            "authenticatorSelection": {
                "residentKey": "preferred",
                "userVerification": "preferred",
            },
            "attestation": "none",
            "timeout": 300_000,
        })
    }

    /// The passkey options for a ticket: an assertion for a sign-in
    /// ticket, a registration for an enrolment ticket.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Unauthenticated`] for an unknown or spent ticket.
    pub fn ticket_passkey_options(&self, ticket: &str) -> Result<Value, Error> {
        let (key, challenge, user) = self.challenge(
            ticket,
            &[ChallengePurpose::SignIn, ChallengePurpose::Enrol],
        )?;
        let issued = self.webauthn_challenge(&key)?;
        Ok(if challenge.purpose == ChallengePurpose::SignIn {
            self.request_options(&user, &issued)
        } else {
            self.creation_options(&user, &issued)
        })
    }

    /// Offers an authenticator app to the person behind an enrolment
    /// ticket.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Unauthenticated`] for an unknown or spent ticket.
    pub fn ticket_totp_setup(&self, ticket: &str) -> Result<TotpSetup, Error> {
        let (_, _, user) =
            self.challenge(ticket, &[ChallengePurpose::Enrol])?;
        self.offer_totp(&user)
    }

    /// Adds a factor with an enrolment ticket and opens the session.
    /// Returns the session and the new recovery codes.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Unauthenticated`] for an unknown ticket or a wrong
    /// code, and the errors of adding a passkey.
    pub fn enrol_with_ticket(
        &self,
        ticket: &str,
        code: Option<&str>,
        passkey: Option<(&str, &Registration)>,
        ip: Option<IpAddr>,
    ) -> Result<(NewSession, Vec<String>), Error> {
        let (key, challenge, user) =
            self.challenge(ticket, &[ChallengePurpose::Enrol])?;
        self.check_lockout(&user.email, ip)?;
        let added = match (code, passkey) {
            (Some(code), None) => self.confirm_totp(user.id, code),
            (None, Some((name, registration))) => {
                let issued =
                    challenge.webauthn.as_deref().ok_or_else(no_ticket)?;
                self.add_passkey(user.id, issued, name, registration)
            }
            _ => Err(Error::new(
                ErrorKind::InvalidInput,
                "send either a code or a passkey",
            )),
        };
        let added = match added {
            Ok(added) => added,
            Err(e) => {
                if e.kind() == ErrorKind::Unauthenticated {
                    self.fail_ticket(&key, &user.email, ip);
                }
                return Err(e);
            }
        };
        let _ = self.store().write(|s| s.mfa_challenges.remove(&key))?;
        self.lockouts.succeed(self.store(), Key::Email(&user.email));
        let session = self.open_session(
            user.id,
            SessionMethod::Password,
            self.config().session_lifetime,
            challenge.user_agent.as_deref(),
        )?;
        Ok((session, added.recovery_codes.unwrap_or_default()))
    }

    fn user_record(&self, user: UserId) -> Result<User, Error> {
        self.store()
            .read(|s| s.users.get(&user).cloned())
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "user not found"))
    }

    /// A person's factors.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::NotFound`] for an unknown user.
    pub fn factors(&self, user: UserId) -> Result<FactorsView, Error> {
        let user = self.user_record(user)?;
        Ok(self.factors_view(&user))
    }

    fn factors_view(&self, user: &User) -> FactorsView {
        FactorsView {
            totp: user.factors.totp.is_some(),
            passkeys: user.factors.passkeys.iter().map(passkey_view).collect(),
            recovery_codes_left: user.factors.recovery.len(),
            required: self.factor_required(user),
        }
    }

    fn offer_totp(&self, user: &User) -> Result<TotpSetup, Error> {
        if user.factors.totp.is_some() {
            return Err(Error::new(
                ErrorKind::Conflict,
                "an authenticator app is already set up; remove it first",
            ));
        }
        let secret = totp::generate_secret()?;
        let sealed = self.sealer()?.seal(&totp_purpose(user.id), &secret)?;
        let pending = PendingTotp {
            sealed,
            expires: after(Timestamp::now(), PENDING_TOTP_LIFETIME),
        };
        self.store().write(|s| {
            if let Some(u) = s.users.get_mut(&user.id) {
                u.factors.pending_totp = Some(pending);
            }
        })?;
        let uri = totp::uri(&secret, ISSUER, &user.email);
        Ok(TotpSetup {
            secret: totp::encode_secret(&secret),
            qr: qr_rows(&uri),
            uri,
        })
    }

    /// Offers a new authenticator app: a secret to scan, confirmed by its
    /// first code ([`Identity::confirm_totp`]).
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Conflict`] when an app is already set up.
    pub fn start_totp(&self, user: UserId) -> Result<TotpSetup, Error> {
        let user = self.user_record(user)?;
        self.offer_totp(&user)
    }

    /// Confirms the offered app with a code from it. The first factor
    /// also gets a set of recovery codes.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Unauthenticated`] for a wrong code, and
    /// [`ErrorKind::Conflict`] when no app was offered or the offer lapsed.
    pub fn confirm_totp(
        &self,
        user: UserId,
        code: &str,
    ) -> Result<FactorAdded, Error> {
        let record = self.user_record(user)?;
        let pending = record
            .factors
            .pending_totp
            .clone()
            .filter(|p| p.expires > Timestamp::now())
            .ok_or_else(|| {
                Error::new(
                    ErrorKind::Conflict,
                    "start adding the authenticator app again",
                )
            })?;
        let secret =
            self.sealer()?.open(&totp_purpose(user), &pending.sealed)?;
        let step =
            totp::verify(&secret, code, totp::step_at(unix_seconds()), None)
                .ok_or_else(wrong_code)?;
        let fresh = if record.factors.any() {
            None
        } else {
            Some(recovery_codes()?)
        };
        let hashes = fresh.as_ref().map(|(_, h)| h.clone());
        let updated = self.store().write(|s| {
            let u = s.users.get_mut(&user)?;
            u.factors.pending_totp = None;
            u.factors.totp = Some(TotpFactor {
                sealed: pending.sealed.clone(),
                last_step: Some(step),
                added_at: Timestamp::now(),
            });
            if let Some(hashes) = hashes {
                u.factors.recovery = hashes;
            }
            Some(u.clone())
        })?;
        let updated = updated
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "user not found"))?;
        tracing::info!(
            event = "identity.second_factor_added",
            user = %user,
            kind = "totp",
            "authenticator app added"
        );
        Ok(FactorAdded {
            factors: self.factors_view(&updated),
            recovery_codes: fresh.map(|(codes, _)| codes),
        })
    }

    fn ensure_another_factor(
        &self,
        user: &User,
        removing_totp: bool,
        passkey: Option<&str>,
    ) -> Result<(), Error> {
        let totp_left = user.factors.totp.is_some() && !removing_totp;
        let passkeys_left = user
            .factors
            .passkeys
            .iter()
            .any(|p| passkey.is_none_or(|id| p.id != id));
        if self.factor_required(user) && !totp_left && !passkeys_left {
            return Err(factor_policy_conflict());
        }
        Ok(())
    }

    /// Removes the authenticator app. Removing the last factor also
    /// removes the recovery codes.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Conflict`] when the workspace requires a factor and
    /// this is the last one.
    pub fn remove_totp(&self, user: UserId) -> Result<FactorsView, Error> {
        let record = self.user_record(user)?;
        self.ensure_another_factor(&record, true, None)?;
        let updated = self.store().write(|s| {
            let u = s.users.get_mut(&user)?;
            u.factors.totp = None;
            u.factors.pending_totp = None;
            if !u.factors.any() {
                u.factors.recovery.clear();
            }
            Some(u.clone())
        })?;
        let updated = updated
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "user not found"))?;
        tracing::info!(
            event = "identity.second_factor_removed",
            user = %user,
            kind = "totp",
            "authenticator app removed"
        );
        Ok(self.factors_view(&updated))
    }

    /// Starts adding a passkey from the settings page.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Conflict`] at the passkey limit.
    pub fn start_passkey(
        &self,
        user: UserId,
    ) -> Result<PasskeyCeremony, Error> {
        let record = self.user_record(user)?;
        if record.factors.passkeys.len() >= MAX_PASSKEYS {
            return Err(Error::new(
                ErrorKind::Conflict,
                format!("you can keep at most {MAX_PASSKEYS} passkeys"),
            ));
        }
        let ticket =
            self.new_challenge(user, ChallengePurpose::Register, None, None)?;
        let issued =
            self.webauthn_challenge(&hash_key(&secret::hash(&ticket)))?;
        Ok(PasskeyCeremony {
            ticket,
            options: self.creation_options(&record, &issued),
        })
    }

    /// Finishes adding a passkey started with [`Identity::start_passkey`].
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Unauthenticated`] for a ceremony that does not verify
    /// or a ticket of another person.
    pub fn finish_passkey(
        &self,
        user: UserId,
        ticket: &str,
        name: &str,
        registration: &Registration,
    ) -> Result<FactorAdded, Error> {
        let (key, challenge, owner) =
            self.challenge(ticket, &[ChallengePurpose::Register])?;
        if owner.id != user {
            return Err(no_ticket());
        }
        let issued = challenge.webauthn.as_deref().ok_or_else(no_ticket)?;
        let added = self.add_passkey(user, issued, name, registration)?;
        let _ = self.store().write(|s| s.mfa_challenges.remove(&key))?;
        Ok(added)
    }

    fn add_passkey(
        &self,
        user: UserId,
        challenge: &str,
        name: &str,
        registration: &Registration,
    ) -> Result<FactorAdded, Error> {
        let name = check_passkey_name(name)?;
        let registered = webauthn::verify_registration(
            &self.relying_party(),
            challenge,
            registration,
        )?;
        let first = !self.user_record(user)?.factors.any();
        let fresh = if first { Some(recovery_codes()?) } else { None };
        let hashes = fresh.as_ref().map(|(_, h)| h.clone());
        let updated = self.store().write(|s| {
            let taken = s.users.values().any(|u| {
                u.factors.passkeys.iter().any(|p| p.id == registered.id)
            });
            if taken {
                return Err(Error::new(
                    ErrorKind::Conflict,
                    "this passkey is already registered",
                ));
            }
            let u = s.users.get_mut(&user).ok_or_else(|| {
                Error::new(ErrorKind::NotFound, "user not found")
            })?;
            if u.factors.passkeys.len() >= MAX_PASSKEYS {
                return Err(Error::new(
                    ErrorKind::Conflict,
                    format!("you can keep at most {MAX_PASSKEYS} passkeys"),
                ));
            }
            u.factors.passkeys.push(Passkey {
                id: registered.id.clone(),
                name: name.clone(),
                key: registered.key.clone(),
                sign_count: registered.sign_count,
                created_at: Timestamp::now(),
                last_used_at: None,
            });
            if let Some(hashes) = hashes {
                u.factors.recovery = hashes;
            }
            Ok(u.clone())
        })??;
        tracing::info!(
            event = "identity.second_factor_added",
            user = %user,
            kind = "passkey",
            "passkey added"
        );
        Ok(FactorAdded {
            factors: self.factors_view(&updated),
            recovery_codes: fresh.map(|(codes, _)| codes),
        })
    }

    /// Removes a passkey.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::NotFound`] for an unknown passkey, and
    /// [`ErrorKind::Conflict`] when the workspace requires a factor and
    /// this is the last one.
    pub fn remove_passkey(
        &self,
        user: UserId,
        id: &str,
    ) -> Result<FactorsView, Error> {
        let record = self.user_record(user)?;
        if !record.factors.passkeys.iter().any(|p| p.id == id) {
            return Err(Error::new(ErrorKind::NotFound, "passkey not found"));
        }
        self.ensure_another_factor(&record, false, Some(id))?;
        let updated = self.store().write(|s| {
            let u = s.users.get_mut(&user)?;
            u.factors.passkeys.retain(|p| p.id != id);
            if !u.factors.any() {
                u.factors.recovery.clear();
            }
            Some(u.clone())
        })?;
        let updated = updated
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "user not found"))?;
        tracing::info!(
            event = "identity.second_factor_removed",
            user = %user,
            kind = "passkey",
            "passkey removed"
        );
        Ok(self.factors_view(&updated))
    }

    /// Replaces the recovery codes with a new set.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Conflict`] without a factor to recover.
    pub fn new_recovery_codes(
        &self,
        user: UserId,
    ) -> Result<Vec<String>, Error> {
        let record = self.user_record(user)?;
        if !record.factors.any() {
            return Err(Error::new(
                ErrorKind::Conflict,
                "add an authenticator app or passkey first",
            ));
        }
        let (codes, hashes) = recovery_codes()?;
        self.store().write(|s| {
            if let Some(u) = s.users.get_mut(&user) {
                u.factors.recovery = hashes;
            }
        })?;
        tracing::info!(
            event = "identity.recovery_codes_replaced",
            user = %user,
            "recovery codes replaced"
        );
        Ok(codes)
    }

    /// Removes every factor of a person, for an administrator helping
    /// someone who lost them all. Their sessions end too.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::NotFound`] for an unknown user.
    pub fn reset_factors(
        &self,
        user: UserId,
    ) -> Result<crate::identity::UserView, Error> {
        let view = self.store().write(|s| {
            let u = s.users.get_mut(&user)?;
            u.factors = SecondFactors::default();
            let view = crate::identity::UserView::from(&*u);
            s.sessions.retain(|_, x| x.user != user);
            s.mfa_challenges.retain(|_, c| c.user != user);
            Some(view)
        })?;
        tracing::warn!(
            event = "identity.second_factors_reset",
            user = %user,
            "second factors reset by an administrator"
        );
        view.ok_or_else(|| Error::new(ErrorKind::NotFound, "user not found"))
    }

    /// The passkey options for confirming the session behind `cookie`.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Unauthenticated`] for an unknown session, and
    /// [`ErrorKind::Conflict`] when its owner has no passkey.
    pub fn reauth_passkey_options(
        &self,
        cookie: &str,
    ) -> Result<PasskeyCeremony, Error> {
        let hash = secret::hash(cookie);
        let (session, user) = self
            .store()
            .read(|s| {
                let session = s.session_by_hash(&hash)?;
                Some((session.id.clone(), s.users.get(&session.user)?.clone()))
            })
            .ok_or_else(|| {
                Error::new(ErrorKind::Unauthenticated, "sign in to continue")
            })?;
        if user.factors.passkeys.is_empty() {
            return Err(Error::new(ErrorKind::Conflict, "you have no passkey"));
        }
        let ticket = self.new_challenge(
            user.id,
            ChallengePurpose::Reauth,
            None,
            Some(session),
        )?;
        let issued =
            self.webauthn_challenge(&hash_key(&secret::hash(&ticket)))?;
        Ok(PasskeyCeremony {
            ticket,
            options: self.request_options(&user, &issued),
        })
    }

    /// Checks the second factor of a re-authentication by `user` for the
    /// session `session_id`. People without a factor pass.
    pub(crate) fn reauth_factor(
        &self,
        user: &User,
        session_id: &str,
        proof: &Proof,
        reauth_ticket: Option<&str>,
        ip: Option<IpAddr>,
    ) -> Result<(), Error> {
        if !user.factors.any() {
            return Ok(());
        }
        if proof.code.is_none() && proof.passkey.is_none() {
            return Err(Error::new(
                ErrorKind::Unauthenticated,
                SECOND_FACTOR_REQUIRED,
            ));
        }
        let (challenge, ticket_key) = if proof.passkey.is_some() {
            let ticket = reauth_ticket.ok_or_else(no_ticket)?;
            let (key, c, owner) =
                self.challenge(ticket, &[ChallengePurpose::Reauth])?;
            if owner.id != user.id || c.session.as_deref() != Some(session_id) {
                return Err(no_ticket());
            }
            (c.webauthn, Some(key))
        } else {
            (None, None)
        };
        let Some(checked) =
            self.check_proof(user, challenge.as_deref(), proof)?
        else {
            let at = Instant::now();
            self.lockouts
                .fail(self.store(), Key::Email(&user.email), at);
            if let Some(ip) = ip {
                self.lockouts.fail(self.store(), Key::Ip(ip), at);
            }
            if let Some(key) = &ticket_key {
                let _ = self.store().write(|s| s.mfa_challenges.remove(key));
            }
            return Err(wrong_code());
        };
        if let Some(key) = &ticket_key {
            let _ = self.store().write(|s| s.mfa_challenges.remove(key))?;
        }
        self.spend(user.id, &checked)
    }
}

/// The associated data a person's authenticator secret is sealed with.
fn totp_purpose(user: UserId) -> String {
    format!("totp:{user}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_codes_are_distinct_and_compare_loosely() {
        let (codes, hashes) = recovery_codes().unwrap();
        assert_eq!(codes.len(), RECOVERY_CODES);
        let unique: std::collections::BTreeSet<_> = hashes.iter().collect();
        assert_eq!(unique.len(), RECOVERY_CODES);
        let code = &codes[0];
        assert_eq!(code.len(), 11);
        assert_eq!(
            recovery_hash(&code.to_uppercase().replace('-', " ")),
            hashes[0]
        );
    }

    fn identity() -> Identity {
        let url = url::Url::parse("https://vellum.northstar.dev").unwrap();
        Identity::new(crate::IdentityConfig::new(url), crate::Store::memory())
    }

    fn challenge_of(options: &Value) -> String {
        options["challenge"].as_str().unwrap().to_owned()
    }

    #[test]
    fn a_passkey_signs_in_once_per_assertion() {
        let id = identity();
        let priya = id
            .setup(
                "priya@northstar.dev",
                "Priya",
                "correct horse battery staple",
            )
            .unwrap();
        let rp = id.relying_party();
        let mut key = crate::webauthn::tests::SoftKey::new();
        let ceremony = id.start_passkey(priya.id).unwrap();
        assert_eq!(ceremony.options["rp"]["id"], "vellum.northstar.dev");
        let registration = key.register(&rp, &challenge_of(&ceremony.options));
        let added = id
            .finish_passkey(priya.id, &ceremony.ticket, "Laptop", &registration)
            .unwrap();
        assert_eq!(added.factors.passkeys.len(), 1);
        assert_eq!(added.recovery_codes.unwrap().len(), RECOVERY_CODES);
        // The ceremony was spent, and the same key cannot be added twice.
        assert!(
            id.finish_passkey(
                priya.id,
                &ceremony.ticket,
                "Laptop",
                &registration
            )
            .is_err()
        );

        let SignIn::Verify(step) = id
            .sign_in_password(
                "priya@northstar.dev",
                "correct horse battery staple",
                None,
                None,
            )
            .unwrap()
        else {
            panic!("expected a second step");
        };
        assert_eq!(step.methods, ["passkey", "recovery"]);
        let options = id.ticket_passkey_options(&step.ticket).unwrap();
        assert_eq!(
            options["allowCredentials"][0]["id"],
            added.factors.passkeys[0].id
        );
        let assertion = key.assert(&rp.id, &rp.origin, &challenge_of(&options));
        let proof = Proof {
            code: None,
            passkey: Some(assertion),
        };
        let session = id.verify_sign_in(&step.ticket, &proof, None).unwrap();
        assert!(id.session(&session.secret).is_some());

        // Replaying the assertion on a new ticket fails: wrong challenge.
        let SignIn::Verify(next) = id
            .sign_in_password(
                "priya@northstar.dev",
                "correct horse battery staple",
                None,
                None,
            )
            .unwrap()
        else {
            panic!("expected a second step");
        };
        let _ = id.ticket_passkey_options(&next.ticket).unwrap();
        assert!(id.verify_sign_in(&next.ticket, &proof, None).is_err());
    }

    #[test]
    fn a_passkey_ceremony_belongs_to_its_person() {
        let id = identity();
        let priya = id
            .setup(
                "priya@northstar.dev",
                "Priya",
                "correct horse battery staple",
            )
            .unwrap();
        let sam = id
            .create_user(
                "sam@northstar.dev",
                "Sam",
                Some("correct horse battery staple"),
                false,
            )
            .unwrap();
        let rp = id.relying_party();
        let key = crate::webauthn::tests::SoftKey::new();
        let ceremony = id.start_passkey(priya.id).unwrap();
        let registration = key.register(&rp, &challenge_of(&ceremony.options));
        // Sam cannot finish Priya's ceremony.
        assert!(
            id.finish_passkey(
                sam.id,
                &ceremony.ticket,
                "Stolen",
                &registration
            )
            .is_err()
        );
        assert!(id.factors(sam.id).unwrap().passkeys.is_empty());
        // A passkey ceremony is not a sign-in ticket.
        let proof = Proof {
            code: Some("123456".into()),
            passkey: None,
        };
        assert!(id.verify_sign_in(&ceremony.ticket, &proof, None).is_err());
    }

    #[test]
    fn the_qr_code_is_square() {
        let rows =
            qr_rows("otpauth://totp/Vellum:priya%40northstar.dev?secret=ABC");
        assert!(rows.len() >= 21);
        assert!(rows.iter().all(|r| r.len() == rows.len()));
    }
}
