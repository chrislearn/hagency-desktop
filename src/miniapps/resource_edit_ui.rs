//! Load canonical readback before enabling the existing native consent footer.
//! A monthly-maximum amendment has its own sheet in the confirmation family:
//! the owner can adjust the maximum with a slider, it stays open while the
//! change saves and then shows what changed, or why nothing did and which
//! amount would work, with that amount one tap away.
use super::*;
use super::sheet_ui::{Fact, Figure, SheetGlyph, SheetTone, SheetView};
use crate::host::hagency::resource_edit::{CeilingFacts, EditRefusal, ResourceEditConsent};
use ::hagency::native_owner::CeilingAmendmentReview;
use crate::miniapps::design::DEFAULT_SLIDER_MAX;

/// Native panel words for a monthly-maximum amendment: title, purpose,
/// consequence and confirmation. The sheet itself is `ceiling_view`.
pub(super) const AMENDMENT_WORDS: (&str, &str, &str, &str) = (
    "Change the monthly maximum",
    "",
    "",
    "Change the monthly maximum",
);

/// The English catalog key shown when the review could not be loaded.
fn failure_text(code: Option<&str>, amendment: bool) -> &'static str {
    if let Some(message)=code.and_then(crate::host::hagency::errors::message){return message;}
    match code {
        None if amendment => "Reading this pool…",
        None => "Reading current resource configuration…",
        Some("management_resource_edit_changed") => "This pool just changed, so nothing was saved. Close this window and refresh, then try again.",
        Some("management_resource_edit_not_available") => "This pool can’t be changed here right now. Close this window and refresh to see why.",
        Some("management_ceiling_amendment_not_available") => "This pool’s monthly maximum can’t change right now. Close this window and refresh to see why.",
        Some("management_ceiling_clear_refused") => "A pool already offered to a server can’t drop its monthly maximum. You can lower it instead.",
        Some("management_ceiling_unchanged") => "The monthly maximum already has this value. Nothing was changed.",
        Some("management_ceiling_below_commitments") => "The monthly maximum can’t be lower than what this month has used and given out.",
        Some("management_ceiling_usage_unsettled") => "Some Agent use is still being reported, so the monthly maximum can only go up for now. Keep this computer online, then try again.",
        Some(_) => "The current settings couldn’t be confirmed. Close this window and refresh, then try again.",
    }
}
/// Refusals the mini app also explains beside its maximum control. The
/// sheet replies with the code, so the page says the same thing after it closes.
fn ceiling_refusal(code: &str) -> bool {
    matches!(
        code,
        "management_resource_edit_changed"
            | "management_ceiling_amendment_not_available"
            | "management_ceiling_clear_refused"
            | "management_ceiling_unchanged"
            | "management_ceiling_below_commitments"
            | "management_ceiling_usage_unsettled"
    )
}
/// Refusals at review that the owner can fix in this window by choosing
/// another value: a review writes nothing, so it can simply run again.
fn ceiling_fixable(code: &str) -> bool {
    matches!(
        code,
        "management_ceiling_unchanged" | "management_ceiling_below_commitments" | "management_ceiling_usage_unsettled"
    )
}
fn amount(tokens: Option<u64>) -> String {
    tokens.map(crate::utils::format_token_count).unwrap_or_else(|| crate::i18n::tr("Unknown").into())
}

/// The monthly-maximum sheet's facts, copied from the native prompt so the
/// shipped sheet and its offline preview render through one function.
pub(super) struct CeilingState<'a> {
    pub account: &'a str,
    /// The amendment under review.
    pub review: Option<&'a CeilingAmendmentReview>,
    /// The same request was already saved: (previous, saved) maximum.
    pub recorded: Option<(u64, u64)>,
    pub facts: Option<&'a CeilingFacts>,
    /// The review could not be shown, and why.
    pub failure: Option<&'a str>,
    pub saving: bool,
    /// What saving returned: the saved amendment, or why it was refused.
    pub saved: Option<&'a Result<Value, String>>,
    pub more: bool,
    /// The value the request proposed, known before its review arrives.
    pub proposed: Option<u64>,
    /// The value the owner chose with the slider here, if they moved it.
    pub chosen: Option<u64>,
    /// After a refusal the owner chose to pick again: the slider shows.
    pub retrying: bool,
    /// The chosen value is being reviewed before it saves.
    pub reviewing: bool,
    /// What the owner typed isn't an amount.
    pub invalid: bool,
}
impl CeilingState<'_> {
    fn previous(&self) -> Option<u64> {
        self.review.map(|review| review.previous_tokens).or(self.facts.and_then(|facts| facts.current))
    }
    /// The value the sheet stands for now: the slider's, else the request's.
    fn value(&self) -> Option<u64> {
        self.chosen
            .or(self.review.map(|review| review.tokens))
            .or(self.facts.and_then(|facts| facts.proposed))
            .or(self.proposed)
    }
    /// The lowest maximum allowed now: what this month used and gave out,
    /// or the current maximum while use is still being reported.
    fn floor(&self) -> u64 {
        match self.facts {
            Some(facts) => facts.committed.or(facts.current).unwrap_or(1).max(1),
            None => 1,
        }
    }
    /// Stable 1B scale, bounded by an explicitly known account limit.
    fn top(&self) -> u64 {
        let known = self.facts.and_then(|facts| facts.account_limit).filter(|limit| *limit > 0);
        known.unwrap_or(DEFAULT_SLIDER_MAX).min(DEFAULT_SLIDER_MAX)
    }
    /// The slider is shown: a review to confirm, or choosing again after a refusal.
    fn choosing(&self) -> bool {
        self.saved.is_none() && !self.saving && self.recorded.is_none() && (self.review.is_some() || self.retrying || self.reviewing)
    }
}
/// The slider's (minimum, maximum, value) while the sheet shows it.
pub(super) fn ceiling_inputs(s: &CeilingState) -> Option<(u64, u64, u64)> {
    s.choosing().then(|| (s.floor(), s.top(), s.value().unwrap_or(0)))
}
/// Everything the monthly-maximum sheet shows, from native state only.
pub(super) fn ceiling_view(s: &CeilingState) -> SheetView {
    let tr = crate::i18n::tr;
    let fmt = crate::i18n::format;
    let facts = s.facts.cloned().unwrap_or_default();
    let name = s
        .review
        .and_then(|review| review.name.clone())
        .or_else(|| facts.name.clone())
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty());
    let previous = s.previous();
    let value = s.value();
    let per_request = s.review.and_then(|review| review.per_request_tokens).or(facts.per_request);
    let mut v = SheetView {
        glyph: SheetGlyph::Maximum,
        title: tr("Change the Monthly Maximum").into(),
        pool: name.clone().unwrap_or_default(),
        caption: tr("Monthly maximum").into(),
        more_label: tr("Technical Details").into(),
        more_open: s.more,
        cancel: tr("Cancel").into(),
        primary: match value {
            Some(n) if n > 0 => fmt("Change to {amount}", &[("amount", amount(Some(n)))]),
            _ => tr("Change the Monthly Maximum").into(),
        },
        quota_label: tr("Monthly maximum").into(),
        ..SheetView::default()
    };
    // For a server administrator: who and the account's limit. Internal
    // IDs, change numbers and revisions stay out of the window.
    let mut technical = vec![(tr("Your account"), s.account.to_owned())];
    if let Some(limit) = facts.account_limit {
        technical.push((tr("Account limit"), amount(Some(limit))));
    }
    v.technical = technical
        .into_iter()
        .filter(|(_, value)| !value.is_empty())
        .map(|(label, value)| format!("{label}\n{value}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    // What stays the same: the per-request limit and running Agents.
    let per_request_fact = |done: bool| {
        let fact = Fact::new(
            tr("Per-request limit"),
            match per_request {
                Some(limit) => fmt("{amount} (unchanged)", &[("amount", amount(Some(limit)))]),
                None => tr("Unchanged").into(),
            },
        );
        if done && value.zip(previous).is_some_and(|(new, old)| new > old) {
            fact.note(tr("To let each Agent get more at a time, raise it on the pool’s page."))
        } else {
            fact.note(tr("Change it separately on the pool’s page if you need to."))
        }
    };
    // The floor, in the mini app's words: what this month used and gave out.
    let floor_fact = || match facts.committed {
        Some(committed) => Some(
            Fact::new(tr("Used and given out this month"), amount(Some(committed)))
                .note(tr("The monthly maximum can’t go below it.")),
        ),
        None if s.facts.is_some() => Some(
            Fact::new(tr("Used and given out this month"), tr("Still being reported"))
                .note(tr("Until it’s known, the monthly maximum can only go up.")),
        ),
        None => None,
    };
    let saved = s.saved.map(|result| result.as_ref().map_err(String::as_str));
    match (saved, s.saving, s.failure.filter(|_| !s.retrying)) {
        // Saved: what changed, in the same words as the page's result.
        (Some(Ok(result)), _, _) => {
            let amended = &result["ceilingAmendment"];
            let before = amended["previousTokens"].as_u64().or(previous);
            let after = amended["tokens"].as_u64().or(value);
            v.tone = SheetTone::Done;
            v.title = tr("Monthly Maximum Saved").into();
            v.before = before.map(|n| Figure::new(amount(Some(n)), tr("Before")));
            v.after = Some(Figure::new(amount(after), tr("Now")));
            v.lead = fmt("Agents can now use up to {amount} in total this month.", &[("amount", amount(after))]);
            v.facts_label = tr("This pool").into();
            v.facts = vec![per_request_fact(true), Fact::new(tr("Running Agents"), tr("Not affected"))];
            v.primary = tr("Done").into();
            v.primary_visible = true;
            v.primary_enabled = true;
            v.primary_closes = true;
            v.cancel.clear();
        }
        // Refused when saving: the request was used, so nothing more here.
        (Some(Err(code)), _, _) => {
            refusal(&mut v, code, previous, value, &facts);
            v.facts_label = tr("This pool").into();
            v.facts = vec![Fact::new(tr("Monthly maximum now"), amount(previous))];
            v.primary = tr("OK").into();
            v.primary_visible = true;
            v.primary_enabled = true;
            v.primary_closes = true;
            v.cancel.clear();
        }
        // Refused at review: why, and the amount that works one tap away.
        (None, false, Some(code)) => {
            refusal(&mut v, code, previous, value, &facts);
            v.facts_label = tr("This pool").into();
            v.facts = vec![Fact::new(tr("Monthly maximum now"), amount(previous))];
            v.primary_visible = true;
            v.primary_enabled = true;
            match (code, facts.committed) {
                ("management_ceiling_below_commitments", Some(committed)) => {
                    v.primary = fmt("Change to {amount}", &[("amount", amount(Some(committed)))]);
                    v.primary_recovers = true;
                    v.cancel = tr("Close").into();
                }
                (code, _) if ceiling_fixable(code) && previous.is_some() => {
                    v.primary = tr("Choose Again").into();
                    v.primary_recovers = true;
                    v.cancel = tr("Close").into();
                }
                _ => {
                    v.primary = tr("OK").into();
                    v.primary_closes = true;
                    v.cancel.clear();
                }
            }
        }
        // Saving: the change being saved stays in view.
        (None, true, _) => {
            v.tone = SheetTone::Working;
            v.before = previous.map(|n| Figure::new(amount(Some(n)), tr("Now")));
            v.after = Some(Figure::new(amount(value), tr("Being saved")));
            v.status = sentences(&[tr("Saving your change…").into(), tr("Closing this window doesn’t cancel it.").into()]);
            v.cancel = tr("Close").into();
        }
        (None, false, None) => match (s.recorded, s.choosing()) {
            (Some((before, after)), _) => {
                v.tone = SheetTone::Done;
                v.title = tr("Already Saved").into();
                v.before = Some(Figure::new(amount(Some(before)), tr("Before")));
                v.after = Some(Figure::new(amount(Some(after)), tr("Now")));
                v.lead = tr("This change was saved earlier. Nothing new is saved.").into();
                v.primary = tr("Done").into();
                v.primary_visible = true;
                v.primary_enabled = true;
                v.cancel.clear();
            }
            // The change, with a slider to adjust it before saving.
            (None, true) => {
                v.tone = SheetTone::Proposal;
                v.choosing = true;
                v.quota_visible = true;
                v.before = previous.map(|n| Figure::new(amount(Some(n)), tr("Now")));
                v.after = Some(Figure::new(amount(value), tr("After you confirm")));
                let lowering = value.zip(previous).is_some_and(|(new, old)| new < old);
                v.lead = fmt(
                    if lowering {
                        "Once you confirm, Agents can use up to {amount} in total this month; running Agents aren’t affected. New requests beyond it can’t be approved."
                    } else {
                        "Once you confirm, Agents can use up to {amount} in total this month; running Agents aren’t affected."
                    },
                    &[("amount", amount(value))],
                );
                let floor = s.floor();
                let limit = facts.account_limit.filter(|limit| *limit > 0);
                let range = match limit {
                    Some(limit) => fmt("Choose {min} – {max}.", &[("min", amount(Some(floor))), ("max", amount(Some(limit)))]),
                    None => fmt("Slider range: {min} – 1 B. You can also type an amount.", &[("min", amount(Some(floor)))]),
                };
                // Opened from the pool's page with its value: say so, so this slider reads as a check.
                v.quota_hint = match (s.proposed, s.chosen) {
                    (Some(page), None) if page > 0 && !s.retrying => sentences(&[
                        fmt("You chose {amount} on the pool’s page; you can still adjust it here.", &[("amount", amount(Some(page)))]),
                        range,
                    ]),
                    _ => range,
                };
                v.problem = match value {
                    _ if s.invalid => tr("Enter an amount, for example 1.5.").into(),
                    None | Some(0) => tr("Choose a monthly maximum above 0.").into(),
                    Some(n) if facts.committed.is_some_and(|committed| n < committed) => fmt(
                        "It can’t be lower than the {committed} used and given out this month.",
                        &[("committed", amount(facts.committed))],
                    ),
                    Some(n) if s.facts.is_some() && facts.committed.is_none() && previous.is_some_and(|old| n < old) => {
                        tr("While use is still being reported, it can only go up.").into()
                    }
                    Some(n) if Some(n) == previous => tr("That’s the current monthly maximum. Choose another amount.").into(),
                    Some(n) if limit.is_some_and(|limit| n > limit) => fmt(
                        "It can’t be more than the account limit of {max}.",
                        &[("max", amount(limit))],
                    ),
                    Some(_) => String::new(),
                };
                // A value that can't be saved isn't described as if it would apply,
                // and the button doesn't name it: it keeps a plain name, disabled.
                if !v.problem.is_empty() {
                    v.lead.clear();
                    v.after = Some(Figure::new(amount(value), tr("You chose")));
                    v.primary = tr("Confirm Changes").into();
                }
                if s.reviewing {
                    v.status = tr("Checking this amount…").into();
                }
                v.facts_label = tr("This pool").into();
                v.facts = vec![per_request_fact(false)];
                if let Some(fact) = floor_fact() {
                    v.facts.push(fact);
                }
                v.primary_visible = true;
                v.primary_enabled = v.problem.is_empty() && !s.reviewing;
            }
            // Reading: the change asked for stays in view.
            (None, false) => {
                v.tone = SheetTone::Working;
                v.technical.clear();
                v.after = s.proposed.map(|n| Figure::new(amount(Some(n)), tr("You chose")));
                if v.after.is_none() {
                    v.caption.clear();
                }
                v.status = tr("Reading this pool…").into();
            }
        },
    }
    if let Some(calendar)=s.review.and_then(|review| review.calendar.as_ref()).or(facts.calendar.as_ref()) {
        use crate::host::hagency::resource_edit::calendar_words;
        for text in [&mut v.title,&mut v.caption,&mut v.quota_label,&mut v.primary,&mut v.lead,&mut v.problem,&mut v.status,&mut v.quota_hint] {*text=calendar_words(text);}
        for fact in &mut v.facts {fact.label=calendar_words(&fact.label);fact.note=calendar_words(&fact.note);}
        if let Some(figure)=&mut v.after {figure.tag=calendar_words(&figure.tag);}
        v.facts.push(super::grant_ui::calendar_fact(calendar,None,crate::host::hagency::grants::now_ms()));
    }
    v
}
/// A refused maximum: why, and which amount would work where it is known.
fn refusal(v: &mut SheetView, code: &str, previous: Option<u64>, proposed: Option<u64>, facts: &CeilingFacts) {
    let tr = crate::i18n::tr;
    let fmt = crate::i18n::format;
    v.tone = SheetTone::Failed;
    v.title = tr("Monthly Maximum Not Saved").into();
    v.status = tr("Nothing was changed.").into();
    match (code, facts.committed) {
        ("management_ceiling_below_commitments", Some(committed)) => {
            v.caption = tr("The monthly maximum must be at least").into();
            v.after = Some(Figure::new(amount(Some(committed)), tr("Used and given out this month")));
            v.lead = match proposed {
                Some(chosen) => fmt(
                    "You chose {chosen}, which is less than the {committed} used and given out this month.",
                    &[("chosen", amount(Some(chosen))), ("committed", amount(Some(committed)))],
                ),
                None => fmt("Choose {committed} or more.", &[("committed", amount(Some(committed)))]),
            };
        }
        ("management_ceiling_usage_unsettled", _) if previous.is_some() => {
            v.caption = tr("For now it can only go up from").into();
            v.after = Some(Figure::new(amount(previous), tr("Now")));
            v.lead = tr(failure_text(Some(code), true)).into();
        }
        ("management_ceiling_unchanged", _) if previous.is_some() => {
            v.after = Some(Figure::new(amount(previous), tr("Now")));
            v.lead = tr("The monthly maximum already has this value.").into();
        }
        _ => {
            v.caption.clear();
            v.lead = tr(failure_text(Some(code), true)).into();
        }
    }
}
fn sentences(parts: &[String]) -> String {
    let separator = if crate::i18n::language() == crate::i18n::Language::Chinese { "" } else { " " };
    parts.iter().filter(|p| !p.is_empty()).cloned().collect::<Vec<_>>().join(separator)
}

/// Offline review only: the monthly maximum sheet for a fixture, through the
/// same view as the real window. `state`: proposal (default), loading,
/// saving or saved; `error` a refusal (at review unless `state` is saved);
/// `chosen` the slider's value; `retrying` after 改成 or 重新选择;
/// `reviewing` while the chosen value is checked. Amounts are tokens.
#[cfg(any(test, feature = "onboarding-test-support"))]
pub(super) struct CeilingPreview {
    review: CeilingAmendmentReview,
    facts: CeilingFacts,
    state: String,
    failure: Option<String>,
    saved: Option<Result<Value, String>>,
    account: String,
    more: bool,
    chosen: Option<u64>,
    retrying: bool,
    reviewing: bool,
}
#[cfg(any(test, feature = "onboarding-test-support"))]
impl CeilingPreview {
    pub(super) fn from_fixture(fixture: &Value) -> Result<Self, String> {
        let tokens = |key: &str| fixture[key].as_u64();
        let review = CeilingAmendmentReview {
            resource_id: fixture["resource"].as_str().unwrap_or("resource_original").into(),
            name: fixture["label"].as_str().map(str::to_owned),
            revision: "0".repeat(64),
            sequence: 1,
            previous_tokens: tokens("current").ok_or("preview field current missing")?,
            tokens: tokens("proposed").ok_or("preview field proposed missing")?,
            per_request_tokens: tokens("perRequest"),
            calendar: serde_json::from_value(fixture["calendar"].clone()).ok(),
        };
        let facts = CeilingFacts {
            calendar: review.calendar.clone(),
            name: review.name.clone(),
            current: Some(review.previous_tokens),
            proposed: Some(review.tokens),
            committed: tokens("committed"),
            per_request: review.per_request_tokens,
            account_limit: tokens("accountLimit"),
        };
        let state = fixture["state"].as_str().unwrap_or("proposal").to_owned();
        let failure = fixture["error"].as_str().map(str::to_owned);
        let chosen = tokens("chosen");
        let saved = match (state.as_str(), &failure) {
            ("saved", None) => Some(Ok(json!({"resourceId": review.resource_id, "ceilingAmendment": {"sequence": 2,
                "previousTokens": review.previous_tokens, "tokens": chosen.unwrap_or(review.tokens)}}))),
            ("saved", Some(code)) => Some(Err(code.clone())),
            _ => None,
        };
        Ok(Self {
            review,
            facts,
            state,
            failure,
            saved,
            account: fixture["account"].as_str().unwrap_or_default().to_owned(),
            more: fixture["technical"].as_bool().unwrap_or(false),
            chosen,
            retrying: fixture["retrying"].as_bool().unwrap_or(false),
            reviewing: fixture["reviewing"].as_bool().unwrap_or(false),
        })
    }
    pub(super) fn state(&self) -> CeilingState<'_> {
        let loading = self.state == "loading";
        CeilingState {
            account: &self.account,
            // A refused or re-chosen value has no clean review.
            review: if self.failure.is_some() || loading || self.retrying { None } else { Some(&self.review) },
            recorded: None,
            facts: if loading { None } else { Some(&self.facts) },
            failure: if self.state == "saved" { None } else { self.failure.as_deref() },
            saving: self.state == "saving",
            saved: self.saved.as_ref(),
            more: self.more,
            proposed: Some(self.review.tokens),
            chosen: self.chosen,
            retrying: self.retrying,
            reviewing: self.reviewing,
            invalid: false,
        }
    }
}
#[cfg(any(test, feature = "onboarding-test-support"))]
pub(super) fn preview(fixture: &Value) -> Result<SheetView, String> {
    Ok(ceiling_view(&CeilingPreview::from_fixture(fixture)?.state()))
}

pub(super) struct ResourceEditPrompt {
    account: String,
    /// The request asked for a monthly-maximum amendment.
    amendment: bool,
    expires: Instant,
    /// A review on its way. The consent always comes back with its result.
    receiver: Option<Receiver<(ResourceEditConsent, Result<(), EditRefusal>)>>,
    /// A consent whose review came back clean: it can be confirmed.
    consent: Option<ResourceEditConsent>,
    /// A monthly-maximum consent refused at review: it can be reviewed
    /// again with another value, since a review writes nothing.
    retry: Option<ResourceEditConsent>,
    failure: Option<String>,
    review: Option<CeilingAmendmentReview>,
    recorded: Option<(u64, u64)>,
    facts: Option<CeilingFacts>,
    /// A confirmed amendment that is saving, then what saving returned.
    saving: Option<Receiver<ServiceEvent>>,
    saved: Option<Result<Value, String>>,
    /// The mini app has its reply for this request.
    replied: bool,
    /// The sheet's single disclosure is open.
    more: bool,
    /// The maximum the request proposed, and the one chosen here.
    proposed: Option<u64>,
    chosen: Option<u64>,
    /// After a refusal the owner chose to pick again.
    retrying: bool,
    /// Save as soon as the review of the chosen value comes back clean.
    save_after_review: bool,
    /// The field holds something that isn't an amount.
    invalid: bool,
}
impl ResourceEditPrompt {
    pub(super) fn begin(consent: ResourceEditConsent) -> Self {
        let account = consent.account().to_owned();
        let amendment = consent.amendment_requested();
        let proposed = consent.proposed_tokens();
        let mut prompt = Self {
            account,
            amendment,
            expires: Instant::now() + Duration::from_secs(300),
            receiver: None,
            consent: None,
            retry: None,
            failure: None,
            review: None,
            recorded: None,
            facts: None,
            saving: None,
            saved: None,
            replied: false,
            more: false,
            proposed,
            chosen: None,
            retrying: false,
            save_after_review: false,
            invalid: false,
        };
        prompt.review_now(consent);
        prompt
    }
    fn review_now(&mut self, consent: ResourceEditConsent) {
        let (tx, receiver) = mpsc::sync_channel(1);
        crate::sliding_sync::spawn_async_task(async move {
            let result = consent.review().await;
            let _ = tx.try_send(result);
            SignalToUI::set_ui_signal();
        });
        self.receiver = Some(receiver);
    }
    pub(super) fn poll(&mut self) -> bool {
        let mut changed = false;
        if let Some(receiver) = &self.receiver {
            let result = match receiver.try_recv() {
                Ok(result) => Some(Some(result)),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => Some(None),
            };
            if let Some(result) = result {
                self.receiver = None;
                self.retrying = false;
                match result {
                    Some((consent, Ok(()))) => {
                        self.review = consent.ceiling_review().cloned();
                        self.recorded = consent.recorded_amendment();
                        if let Some(facts) = consent.facts() {
                            self.facts = Some(facts.clone());
                        }
                        self.failure = None;
                        self.consent = Some(consent);
                        if std::mem::take(&mut self.save_after_review) {
                            self.start_save();
                        }
                    }
                    Some((consent, Err(refusal))) => {
                        self.review = None;
                        self.failure = Some(refusal.code);
                        if refusal.facts.is_some() {
                            self.facts = refusal.facts;
                        }
                        self.save_after_review = false;
                        if self.amendment {
                            self.retry = Some(consent);
                        }
                    }
                    None => {
                        self.save_after_review = false;
                        self.failure = Some("management_unavailable".into());
                    }
                }
                changed = true;
            }
        }
        if let Some(receiver) = &self.saving {
            let result = match receiver.try_recv() {
                Ok(ServiceEvent::Complete(result)) => Some(result),
                Ok(_) | Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => Some(Err("management_outcome_unknown".into())),
            };
            if let Some(result) = result {
                self.saving = None;
                self.saved = Some(result);
                changed = true;
            }
        }
        changed
    }
    pub(super) fn account(&self) -> &str {
        &self.account
    }
    pub(super) fn expired(&self) -> bool {
        Instant::now() >= self.expires
            || self
                .consent
                .as_ref()
                .is_some_and(ResourceEditConsent::expired)
    }
    pub(super) fn ready(&self) -> bool {
        self.consent.is_some() && !self.expired()
    }
    pub(super) fn recovering(&self) -> bool {
        self.consent
            .as_ref()
            .is_some_and(ResourceEditConsent::recovering)
    }
    pub(super) fn amending(&self) -> bool {
        self.consent
            .as_ref()
            .is_some_and(ResourceEditConsent::amending)
    }
    /// A monthly-maximum request keeps its own window, even when its review
    /// could not be loaded and the window only says why.
    pub(super) fn amendment_requested(&self) -> bool {
        self.amendment
    }
    pub(super) fn details(&self) -> String {
        if let Some(consent) = &self.consent {
            return consent.details();
        }
        crate::i18n::tr(failure_text(self.failure.as_deref(), self.amendment)).into()
    }
    fn state(&self) -> CeilingState<'_> {
        CeilingState {
            account: &self.account,
            review: self.review.as_ref(),
            recorded: self.recorded,
            facts: self.facts.as_ref(),
            failure: self.failure.as_deref(),
            saving: self.saving.is_some(),
            saved: self.saved.as_ref(),
            more: self.more,
            proposed: self.proposed,
            chosen: self.chosen,
            retrying: self.retrying,
            reviewing: self.receiver.is_some() && self.save_after_review,
            invalid: self.invalid,
        }
    }
    /// The sheet for this monthly-maximum request, from native state only.
    pub(super) fn sheet(&self) -> SheetView {
        ceiling_view(&self.state())
    }
    /// The slider's (minimum, maximum, value) while the sheet shows it.
    pub(super) fn inputs(&self) -> Option<(u64, u64, u64)> {
        ceiling_inputs(&self.state())
    }
    pub(super) fn toggle_more(&mut self) {
        self.more = !self.more;
    }
    /// The owner moved the slider or typed into the field: an amount, or
    /// None for text that isn't one. Nothing is reviewed or saved until
    /// they tap Save. Returns whether what the sheet shows changed.
    pub(super) fn choose(&mut self, tokens: Option<u64>) -> bool {
        if self.saving.is_some() || self.saved.is_some() || self.save_after_review {
            return false;
        }
        let Some(tokens) = tokens else {
            return !std::mem::replace(&mut self.invalid, true);
        };
        let was_invalid = std::mem::replace(&mut self.invalid, false);
        if self.chosen == Some(tokens) {
            return was_invalid;
        }
        if self.chosen.is_none() && self.state().value() == Some(tokens) {
            return was_invalid;
        }
        self.chosen = Some(tokens);
        true
    }
    /// 改成 0.25 M or 重新选择 after a refusal: back to the slider, set to
    /// the amount that works where it is known. The refusal is kept for the
    /// reply until something saves.
    pub(super) fn rechoose(&mut self) {
        if self.saving.is_some() || self.saved.is_some() || self.receiver.is_some() {
            return;
        }
        if let (Some("management_ceiling_below_commitments"), Some(committed)) =
            (self.failure.as_deref(), self.facts.as_ref().and_then(|facts| facts.committed))
        {
            self.chosen = Some(committed);
        }
        self.retrying = true;
    }
    /// Confirm the maximum shown and keep the sheet: it saves, then shows
    /// the result. A value chosen here is reviewed again first, then saved
    /// once that review comes back clean.
    pub(super) fn save(&mut self) -> bool {
        if self.saving.is_some() || self.saved.is_some() || self.receiver.is_some() || self.invalid {
            return false;
        }
        let reviewed = self.review.as_ref().map(|review| review.tokens);
        if self.chosen.is_none() || self.chosen == reviewed {
            return self.start_save();
        }
        let Some(tokens) = self.chosen else {
            return false;
        };
        let Some(mut consent) = self.consent.take().or_else(|| self.retry.take()) else {
            return false;
        };
        if consent.set_ceiling_tokens(tokens).is_err() {
            self.retry = Some(consent);
            return false;
        }
        self.review = None;
        self.save_after_review = true;
        self.review_now(consent);
        true
    }
    fn start_save(&mut self) -> bool {
        if self.saving.is_some() || self.saved.is_some() || !self.ready() || !self.amending() {
            return false;
        }
        let Some(consent) = self.consent.take() else {
            return false;
        };
        let (tx, receiver) = mpsc::sync_channel(1);
        crate::sliding_sync::spawn_async_task(async move {
            let result = consent.accept().await;
            let _ = tx.try_send(ServiceEvent::Complete(result));
            SignalToUI::set_ui_signal();
        });
        // Saving has its own deadline inside the writer; the sheet stays.
        self.expires = Instant::now() + Duration::from_secs(600);
        self.saving = Some(receiver);
        true
    }
    pub(super) fn saving(&self) -> bool {
        self.saving.is_some()
    }
    /// The last review was refused; closing replies with why.
    pub(super) fn refused(&self) -> bool {
        self.failure.as_deref().is_some_and(ceiling_refusal)
    }
    /// The save's result for the mini app, once, as soon as it is known.
    pub(super) fn take_reply(&mut self) -> Option<Result<Value, String>> {
        if self.replied {
            return None;
        }
        let reply = self.saved.clone()?;
        self.replied = true;
        Some(reply)
    }
    /// Closing the sheet: a save still running keeps its reply path, and a
    /// refusal at review tells the mini app why instead of "cancelled".
    pub(super) fn close(&mut self) -> CeilingClose {
        if self.replied {
            return CeilingClose::Replied;
        }
        if let Some(receiver) = self.saving.take() {
            self.replied = true;
            return CeilingClose::Saving(receiver);
        }
        if let Some(reply) = self.take_reply() {
            return CeilingClose::Reply(reply);
        }
        match self.failure.as_deref() {
            Some(code) if ceiling_refusal(code) => {
                self.replied = true;
                CeilingClose::Reply(Err(code.to_owned()))
            }
            _ => CeilingClose::Cancelled,
        }
    }
    pub(super) async fn accept(self) -> Result<Value, String> {
        if self.expired() {
            return Err("provider_confirmation_expired".into());
        }
        self.consent
            .ok_or_else(|| "resource_edit_review_required".to_owned())?
            .accept()
            .await
    }
}
/// How a closing monthly-maximum sheet answers the mini app.
pub(super) enum CeilingClose {
    /// The result was already sent.
    Replied,
    /// Still saving: the result is sent when it arrives.
    Saving(Receiver<ServiceEvent>),
    /// Send this now.
    Reply(Result<Value, String>),
    /// Nothing was confirmed.
    Cancelled,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{Language, text_for};

    #[test]
    fn ceiling_amendment_panel_and_refusals_are_localized_and_distinct() {
        let (title, purpose, consequence, confirm) = AMENDMENT_WORDS;
        for english in [title, confirm] {
            assert_ne!(text_for(Language::Chinese, english), english, "{english}");
        }
        assert!(purpose.is_empty() && consequence.is_empty());
        assert_eq!(text_for(Language::Chinese, title), "修改每月上限");
        // One verb with the per-request window and the page: the button commits 改成 X.
        assert_eq!(text_for(Language::Chinese, confirm), "修改每月上限");
        assert_eq!(text_for(Language::Chinese, "Change to {amount}"), "改成 {amount}");
        let generic = failure_text(Some("management_unavailable"), true);
        for code in [
            "management_ceiling_amendment_not_available",
            "management_ceiling_clear_refused",
            "management_ceiling_unchanged",
            "management_ceiling_below_commitments",
            "management_ceiling_usage_unsettled",
        ] {
            let english = failure_text(Some(code), true);
            assert_ne!(english, generic, "{code} has its own reason");
            assert_ne!(text_for(Language::Chinese, english), english, "{code} is translated");
            assert!(ceiling_refusal(code), "{code} reaches the page as itself");
        }
        assert!(failure_text(Some("management_ceiling_below_commitments"), true).contains("used and given out"));
        assert_eq!(text_for(Language::Chinese, "Used and given out this month"), "本月已用和已分出");
        assert_eq!(
            text_for(Language::Chinese, failure_text(Some("management_ceiling_usage_unsettled"), true)),
            "部分 Agent 的用量还在上报，所以暂时只能调高、不能调低每月上限。请保持这台电脑在线，稍后再试。"
        );
        assert_eq!(text_for(Language::Chinese, failure_text(None, true)), "正在读取这个资源池…");
    }

    fn fixture(extra: Value) -> Value {
        let mut base = json!({"panel": "maximum", "account": "@owner:test", "label": "Research",
            "current": 1_000_000u64, "proposed": 2_000_000u64, "perRequest": 200_000u64, "committed": 250_000u64,
            "accountLimit": 5_000_000u64});
        for (key, value) in extra.as_object().unwrap() {
            base[key] = value.clone();
        }
        base
    }

    #[test]
    fn owner_ceiling_slider_stays_at_one_billion_while_the_amount_changes() {
        for proposed in [20_000_000u64, 500_000_000, 1_000_000_000, 2_000_000_000] {
            let preview = CeilingPreview::from_fixture(&fixture(json!({
                "current":20_000_000u64, "proposed":proposed, "committed":5_000_000u64, "accountLimit":null
            }))).unwrap();
            assert_eq!(ceiling_inputs(&preview.state()), Some((5_000_000, 1_000_000_000, proposed)));
            let view = ceiling_view(&preview.state());
            assert!(view.quota_hint.contains("1 B"));
            if proposed != 20_000_000 { assert!(view.problem.is_empty(), "{}", view.problem); }
        }
    }

    #[test]
    fn the_maximum_sheet_shows_before_and_after_and_what_stays_the_same() {
        let view = preview(&fixture(json!({}))).unwrap();
        assert_eq!(view.tone, SheetTone::Proposal);
        assert_eq!(view.title, "Change the Monthly Maximum");
        assert_eq!(view.pool, "Research");
        assert_eq!(view.before, Some(Figure::new("1 M", "Now")));
        assert_eq!(view.after, Some(Figure::new("2 M", "After you confirm")));
        assert_eq!(view.lead, "Once you confirm, Agents can use up to 2 M in total this month; running Agents aren’t affected.");
        let rows: Vec<(&str, &str)> = view.facts.iter().map(|f| (f.label.as_str(), f.value.as_str())).collect();
        assert_eq!(rows, [
            ("Per-request limit", "0.2 M (unchanged)"),
            ("Used and given out this month", "0.25 M"),
        ]);
        assert_eq!(view.facts[1].note, "The monthly maximum can’t go below it.");
        // The slider adjusts it here, between the floor and the account limit.
        assert!(view.choosing && view.quota_visible && view.problem.is_empty());
        // Opened with the page's value: said as such, so this slider reads as a check.
        assert_eq!((view.quota_label.as_str(), view.quota_hint.as_str()), ("Monthly maximum", "You chose 2 M on the pool’s page; you can still adjust it here. Choose 0.25 M – 5 M."));
        let preview = CeilingPreview::from_fixture(&fixture(json!({}))).unwrap();
        assert_eq!(ceiling_inputs(&preview.state()), Some((250_000, 5_000_000, 2_000_000)));
        // The account is labelled and behind the disclosure, never a bare ID.
        for shown in [&view.title, &view.pool, &view.lead, &view.status] {
            assert!(!shown.contains("@owner") && !shown.contains("resource_original"), "{shown}");
        }
        assert!(view.technical.starts_with("Your account\n@owner:test"));
        assert_eq!((view.primary.as_str(), view.cancel.as_str()), ("Change to 2 M", "Cancel"));
        assert!(view.primary_enabled && !view.primary_closes);
    }

    #[test]
    fn a_saved_or_refused_maximum_looks_different_and_says_what_works() {
        let saved = preview(&fixture(json!({"state": "saved"}))).unwrap();
        assert_eq!(saved.tone, SheetTone::Done);
        assert_eq!(saved.title, "Monthly Maximum Saved");
        assert_eq!((saved.before.clone().unwrap().value, saved.after.clone().unwrap().value), ("1 M".into(), "2 M".into()));
        assert!(saved.primary_closes && saved.cancel.is_empty() && saved.primary == "Done");
        // The same section name as every other state and the per-request window.
        assert_eq!(saved.facts_label, "This pool");
        let refused = preview(&fixture(json!({"error": "management_ceiling_below_commitments", "proposed": 200_000u64}))).unwrap();
        assert_eq!(refused.tone, SheetTone::Failed);
        assert_eq!(refused.title, "Monthly Maximum Not Saved");
        assert_eq!(refused.after, Some(Figure::new("0.25 M", "Used and given out this month")));
        assert_eq!(refused.lead, "You chose 0.2 M, which is less than the 0.25 M used and given out this month.");
        assert_eq!(refused.status, "Nothing was changed.");
        // The amount that works is one tap away, in this window.
        assert_eq!((refused.primary.as_str(), refused.cancel.as_str()), ("Change to 0.25 M", "Close"));
        assert!(refused.primary_recovers && refused.primary_enabled && !refused.primary_closes);
        // A refusal when saving used the request up: OK only.
        let late = preview(&fixture(json!({"state": "saved", "error": "management_ceiling_below_commitments", "proposed": 200_000u64}))).unwrap();
        assert_eq!((late.primary.as_str(), late.cancel.as_str()), ("OK", ""));
        assert!(late.primary_closes && !late.primary_recovers);
        let saving = preview(&fixture(json!({"state": "saving"}))).unwrap();
        assert_eq!(saving.tone, SheetTone::Working);
        assert!(!saving.primary_visible && saving.cancel == "Close");
        assert!(saving.status.contains("doesn’t cancel"), "{}", saving.status);
        // One saving line in both windows.
        assert!(saving.status.starts_with("Saving your change…"), "{}", saving.status);
    }

    #[test]
    fn the_owner_adjusts_the_maximum_in_the_window_and_fixes_a_refusal_there() {
        // Moved on the slider: the hero follows, and Save stays one tap.
        let moved = preview(&fixture(json!({"chosen": 1_500_000u64}))).unwrap();
        assert_eq!(moved.after, Some(Figure::new("1.5 M", "After you confirm")));
        assert!(moved.primary_enabled && moved.problem.is_empty());
        // Below the floor, equal to now, or above the account limit: said beside it.
        for (chosen, problem) in [
            (200_000u64, "It can’t be lower than the 0.25 M used and given out this month."),
            (1_000_000, "That’s the current monthly maximum. Choose another amount."),
            (6_000_000, "It can’t be more than the account limit of 5 M."),
        ] {
            let view = preview(&fixture(json!({"chosen": chosen}))).unwrap();
            assert_eq!(view.problem, problem, "{chosen}");
            assert!(!view.primary_enabled, "{chosen}");
            assert_eq!(view.primary, "Confirm Changes", "{chosen}");
            // A value that can't be saved isn't described as if it would apply.
            assert!(view.lead.is_empty(), "{chosen}: {}", view.lead);
            assert_eq!(view.after.clone().map(|f| f.tag), Some("You chose".to_owned()), "{chosen}");
        }
        // Lowering says what it means for new requests.
        let lower = preview(&fixture(json!({"chosen": 500_000u64}))).unwrap();
        assert!(lower.lead.ends_with("New requests beyond it can’t be approved."), "{}", lower.lead);
        // After 改成 0.25 M: back to the slider at the floor, ready to save.
        let fixed = preview(&fixture(json!({"error": "management_ceiling_below_commitments", "proposed": 200_000u64,
            "retrying": true, "chosen": 250_000u64}))).unwrap();
        assert_eq!(fixed.tone, SheetTone::Proposal);
        assert_eq!((fixed.before.clone().unwrap().value, fixed.after.clone().unwrap().value), ("1 M".into(), "0.25 M".into()));
        assert!(fixed.choosing && fixed.primary_enabled && fixed.primary == "Change to 0.25 M");
        // While that value is checked, Save waits.
        let checking = preview(&fixture(json!({"chosen": 1_500_000u64, "reviewing": true, "retrying": true,
            "error": "management_ceiling_below_commitments"}))).unwrap();
        assert!(!checking.primary_enabled && checking.status == "Checking this amount…");
        // Saved: the value chosen here is what the window and the page report.
        let saved = preview(&fixture(json!({"state": "saved", "chosen": 1_500_000u64}))).unwrap();
        assert_eq!(saved.after, Some(Figure::new("1.5 M", "Now")));
        // Use still being reported: it can only go up.
        let unsettled = preview(&fixture(json!({"committed": null, "chosen": 800_000u64}))).unwrap();
        assert_eq!(unsettled.problem, "While use is still being reported, it can only go up.");
        // Text that isn't an amount can't be saved as the last good value.
        let preview = CeilingPreview::from_fixture(&fixture(json!({}))).unwrap();
        let typed = CeilingState { invalid: true, ..preview.state() };
        let view = ceiling_view(&typed);
        assert_eq!(view.problem, "Enter an amount, for example 1.5.");
        assert!(!view.primary_enabled);
    }
}
