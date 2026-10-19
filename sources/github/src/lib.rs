//! Explicitly scoped GitHub acquisition. The core owns all ingestion and durable cursor writes.

mod api;
mod wiki;

#[cfg(not(test))]
use memcastle::source::host::run_process;

// Native coverage runs cannot call the component host import. Execute only the checked-in
// offline stand-ins, using absolute paths so tests never change the process-wide PATH.
#[cfg(test)]
fn run_process(
    program: &str,
    args: &[String],
    stdin: Option<&[u8]>,
) -> Result<memcastle::source::host::ProcessOutput, String> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let executable = match program {
        "curl" | "git" => format!("{}/fixtures/bin/{program}", env!("CARGO_MANIFEST_DIR")),
        "sh" => "/bin/sh".into(),
        "rm" => "/bin/rm".into(),
        _ => return Err("unexpected test process".into()),
    };
    let mut command = Command::new(executable);
    command.args(args);
    if program == "sh" {
        command.env(
            "PATH",
            format!(
                "{}/fixtures/bin:{}",
                env!("CARGO_MANIFEST_DIR"),
                std::env::var("PATH").unwrap_or_default()
            ),
        );
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    if let Some(input) = stdin {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input)
            .map_err(|error| error.to_string())?;
    }
    let output = child
        .wait_with_output()
        .map_err(|error| error.to_string())?;
    Ok(memcastle::source::host::ProcessOutput {
        status: output.status.code().unwrap_or(-1),
        stdout: output.stdout,
        stderr: output.stderr,
    })
}

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

wit_bindgen::generate!({
    path: "wit",
    world: "source",
});

use exports::memcastle::source::adapter::Guest;
use memcastle::source::types::{
    Candidate, CanonicalDocument, Discovery, RawDocument, Segment, SourceError, SourceKind,
    SourceRef,
};

const MAX_REPOSITORIES: usize = 500;

struct GitHub;

fn option<'a>(source: &'a SourceRef, name: &str) -> Option<&'a str> {
    source
        .options
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

fn list(value: &str) -> Result<Vec<String>, String> {
    let mut entries = BTreeSet::new();
    for entry in value.split(',') {
        let entry = entry.trim().to_ascii_lowercase();
        if entry.is_empty() || entry.chars().any(char::is_control) {
            return Err("scope lists must contain nonempty, comma-separated entries".into());
        }
        entries.insert(entry);
    }
    Ok(entries.into_iter().collect())
}

fn pattern(value: &str) -> bool {
    let Some((owner, repo)) = value.split_once('/') else {
        return false;
    };
    !owner.is_empty()
        && !repo.is_empty()
        && !repo.contains('/')
        && owner
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_*?".contains(&byte))
        && repo
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-._*?".contains(&byte))
}

/// `*` and `?` do not match `/`, so owner/repo boundaries cannot be crossed accidentally.
fn glob(pattern: &str, text: &str) -> bool {
    let (mut p, mut t) = (0, 0);
    let (mut star, mut retry) = (None, 0);
    let pattern = pattern.as_bytes();
    let text = text.as_bytes();
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == text[t] || pattern[p] == b'?' && text[t] != b'/') {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == b'*' {
            star = Some(p);
            retry = t;
            p += 1;
        } else if let Some(at) = star {
            if text[retry] == b'/' {
                return false;
            }
            retry += 1;
            t = retry;
            p = at + 1;
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|byte| *byte == b'*')
}

fn matches(patterns: &[String], value: &str) -> bool {
    patterns.iter().any(|pattern| glob(pattern, value))
}

fn entries(source: &SourceRef, key: &str) -> Vec<String> {
    option(source, key)
        .map(|value| value.split(',').map(str::to_string).collect())
        .unwrap_or_default()
}

fn enabled(source: &SourceRef, key: &str, default: bool) -> bool {
    option(source, key).map_or(default, |value| value == "true")
}

fn timestamp(value: &str) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(value)
        .map(|at| at.with_timezone(&Utc))
        .map_err(|_| "GitHub returned an invalid timestamp".into())
}

fn since(value: &str) -> Result<DateTime<Utc>, String> {
    if let Ok(at) = timestamp(value) {
        return Ok(at);
    }
    let day = if value.len() == 7 {
        format!("{value}-01")
    } else {
        value.into()
    };
    chrono::NaiveDate::parse_from_str(&day, "%Y-%m-%d")
        .ok()
        .and_then(|date| date.and_hms_opt(0, 0, 0))
        .map(|date| date.and_utc())
        .ok_or_else(|| "since must be YYYY-MM, YYYY-MM-DD or an RFC 3339 instant".into())
}

#[derive(Default, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    api: BTreeMap<String, (String, String)>,
    wiki: BTreeMap<String, String>,
}

fn cursor(value: &str) -> Result<Cursor, SourceError> {
    let value: Value = serde_json::from_str(value)
        .map_err(|_| SourceError::CursorInvalid("the cursor is not JSON".into()))?;
    if value.is_null() {
        return Ok(Cursor::default());
    }
    let parsed: Cursor = serde_json::from_value(value).map_err(|_| {
        SourceError::CursorInvalid("unrecognized GitHub cursor; run with --full".into())
    })?;
    if parsed.api.iter().any(|(repo, (at, id))| {
        !pattern(repo) || repo.contains(['*', '?']) || timestamp(at).is_err() || id.is_empty()
    }) || parsed
        .wiki
        .iter()
        .any(|(repo, key)| !pattern(repo) || key.len() < 42 || !key.contains(':'))
    {
        return Err(SourceError::CursorInvalid(
            "invalid GitHub cursor; run with --full".into(),
        ));
    }
    Ok(parsed)
}

#[derive(Clone, Serialize, Deserialize)]
struct Handle {
    kind: String,
    repo: String,
    number: Option<u64>,
    id: Option<u64>,
    wiki: Option<wiki::Revision>,
    updated: String,
}

struct Entry {
    key: (String, String),
    external_id: String,
    handle: Handle,
}

fn id(value: &Value) -> Result<u64, String> {
    value
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| "GitHub returned a record without a numeric ID".into())
}

fn number(value: &Value) -> Result<u64, String> {
    value
        .get("number")
        .and_then(Value::as_u64)
        .ok_or_else(|| "GitHub returned an issue or pull without a number".into())
}

fn add(
    entries: &mut Vec<Entry>,
    repo: &str,
    kind: &str,
    value: &Value,
    number: Option<u64>,
) -> Result<(), String> {
    let id = id(value)?;
    let updated = value
        .get("updated_at")
        .or_else(|| value.get("submitted_at"))
        .or_else(|| value.get("created_at"))
        .and_then(Value::as_str)
        .ok_or("GitHub returned a record without a timestamp")?;
    timestamp(updated)?;
    let external_id = format!("{repo}/{kind}/{id}");
    entries.push(Entry {
        key: (updated.into(), external_id.clone()),
        external_id,
        handle: Handle {
            kind: kind.into(),
            repo: repo.into(),
            number,
            id: Some(id),
            wiki: None,
            updated: updated.into(),
        },
    });
    Ok(())
}

/// Expand only owners explicitly named by an include pattern; exact names never enumerate an account.
fn repositories(source: &SourceRef) -> Result<Vec<(String, Value)>, String> {
    let include = entries(source, "include");
    let exclude = entries(source, "exclude");
    let topics = entries(source, "topics");
    let mut names = BTreeSet::new();
    for selector in &include {
        let (owner, _) = selector
            .split_once('/')
            .ok_or("invalid repository selector")?;
        if !selector.contains(['*', '?']) {
            names.insert(selector.clone());
        } else {
            let listings = if owner.contains(['*', '?']) {
                if api::token().is_none() {
                    return Err("owner wildcard needs a selected GitHub credential or GH_TOKEN/GITHUB_TOKEN to list accessible repositories".into());
                }
                api::pages("/user/repos?affiliation=owner,collaborator,organization_member")?
            } else {
                // `org/*` is a repository selector, not permission to ingest every accessible account.
                match api::pages(&format!("/orgs/{owner}/repos?type=all")) {
                    Ok(listings) => listings,
                    // A user, unlike an organization, has a different documented listing endpoint.
                    Err(error) if error.contains("unavailable") => {
                        let mut listings = api::pages(&format!("/users/{owner}/repos?type=owner"))?;
                        // Public user listings omit private repositories even when the token can read them.
                        if api::token().is_some() {
                            listings.extend(api::pages(
                                "/user/repos?affiliation=owner,collaborator,organization_member",
                            )?);
                        }
                        listings
                    }
                    Err(error) => return Err(error),
                }
            };
            for listing in listings {
                let name = listing
                    .get("full_name")
                    .and_then(Value::as_str)
                    .ok_or("GitHub repository listing has no full_name")?
                    .to_ascii_lowercase();
                if pattern(&name) && !name.contains(['*', '?']) && glob(selector, &name) {
                    names.insert(name);
                }
            }
        }
    }
    if names.len() > MAX_REPOSITORIES {
        return Err("more than 500 repositories match; narrow include/exclude patterns".into());
    }
    let mut selected = Vec::new();
    for name in names {
        if matches(&exclude, &name) {
            continue;
        }
        let metadata = api::request(&format!("/repos/{name}"))?;
        if !topics.is_empty()
            && !metadata
                .get("topics")
                .and_then(Value::as_array)
                .is_some_and(|found| {
                    found
                        .iter()
                        .filter_map(Value::as_str)
                        .any(|topic| topics.iter().any(|wanted| wanted == topic))
                })
        {
            continue;
        }
        selected.push((name, metadata));
    }
    if selected.is_empty() {
        return Err("no accessible repositories match include, exclude and topics; check the scope and token".into());
    }
    Ok(selected)
}

fn discover_api(source: &SourceRef, repos: &[(String, Value)]) -> Result<Vec<Entry>, String> {
    let mut result = Vec::new();
    let labels = entries(source, "labels");
    let boundary = option(source, "since").map(since).transpose()?;
    for (repo, metadata) in repos {
        if enabled(source, "metadata", true) {
            add(&mut result, repo, "repository", metadata, None)?;
        }
        if !["issues", "pulls", "comments", "reviews"]
            .iter()
            .any(|key| enabled(source, key, matches!(*key, "issues" | "pulls")))
        {
            continue;
        }
        let issues = api::pages(&format!(
            "/repos/{repo}/issues?state=all&sort=updated&direction=asc"
        ))?;
        let mut selected = BTreeMap::new();
        for issue in issues {
            let pull = issue.get("pull_request").is_some();
            let kind = if pull { "pull" } else { "issue" };
            let category = if pull { "pulls" } else { "issues" };
            if !enabled(source, category, true) {
                continue;
            }
            if !labels.is_empty()
                && !issue
                    .get("labels")
                    .and_then(Value::as_array)
                    .is_some_and(|found| {
                        found
                            .iter()
                            .filter_map(|label| label.get("name").and_then(Value::as_str))
                            .any(|label| {
                                labels
                                    .iter()
                                    .any(|wanted| wanted == &label.to_ascii_lowercase())
                            })
                    })
            {
                continue;
            }
            let issue_number = number(&issue)?;
            selected.insert(issue_number, pull);
            if boundary.is_none_or(|since| {
                issue
                    .get("updated_at")
                    .and_then(Value::as_str)
                    .and_then(|at| timestamp(at).ok())
                    .is_some_and(|at| at >= since)
            }) {
                add(&mut result, repo, kind, &issue, Some(issue_number))?;
            }
        }
        if enabled(source, "comments", false) {
            for comment in api::pages(&format!("/repos/{repo}/issues/comments"))? {
                let Some(issue_number) = comment
                    .get("issue_url")
                    .and_then(Value::as_str)
                    .and_then(|url| url.rsplit('/').next())
                    .and_then(|n| n.parse::<u64>().ok())
                else {
                    continue;
                };
                if selected.contains_key(&issue_number) {
                    add(&mut result, repo, "comment", &comment, Some(issue_number))?;
                }
            }
        }
        if enabled(source, "reviews", false) {
            for comment in api::pages(&format!("/repos/{repo}/pulls/comments"))? {
                let Some(pull_number) = comment
                    .get("pull_request_url")
                    .and_then(Value::as_str)
                    .and_then(|url| url.rsplit('/').next())
                    .and_then(|n| n.parse::<u64>().ok())
                else {
                    continue;
                };
                if selected.get(&pull_number) == Some(&true) {
                    add(
                        &mut result,
                        repo,
                        "review-comment",
                        &comment,
                        Some(pull_number),
                    )?;
                }
            }
            for pull_number in selected
                .iter()
                .filter_map(|(number, pull)| pull.then_some(*number))
            {
                for review in api::pages(&format!("/repos/{repo}/pulls/{pull_number}/reviews"))? {
                    if review.get("submitted_at").is_some_and(Value::is_null) {
                        continue;
                    }
                    add(&mut result, repo, "review", &review, Some(pull_number))?;
                }
            }
        }
    }
    result.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(result)
}

fn digest(body: &str) -> String {
    format!("{:x}", Sha256::digest(body.as_bytes()))
}

fn normalized(raw: RawDocument) -> Result<CanonicalDocument, SourceError> {
    let metadata: Value = serde_json::from_str(&raw.metadata)
        .map_err(|_| SourceError::Failed("GitHub document metadata is invalid".into()))?;
    let kind = metadata
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| SourceError::Failed("GitHub document has no kind".into()))?;
    let repo = metadata
        .get("repo")
        .and_then(Value::as_str)
        .ok_or_else(|| SourceError::Failed("GitHub document has no repository".into()))?;
    let title = metadata
        .get("title")
        .and_then(Value::as_str)
        .map(str::to_string);
    let uri = metadata
        .get("url")
        .and_then(Value::as_str)
        .map(str::to_string);
    let mut tags = vec!["github".into(), kind.into(), format!("repo:{repo}")];
    if let Some(labels) = metadata.get("labels").and_then(Value::as_array) {
        for label in labels.iter().filter_map(Value::as_str) {
            tags.push(format!("label:{label}"));
        }
    }
    if let Some(topics) = metadata.get("topics").and_then(Value::as_array) {
        for topic in topics.iter().filter_map(Value::as_str) {
            tags.push(format!("topic:{topic}"));
        }
    }
    Ok(CanonicalDocument {
        title,
        room: Some(
            match kind {
                "wiki" => "wikis",
                "repository" => "repositories",
                "pull" | "review" | "review-comment" => "pulls",
                _ => "issues",
            }
            .into(),
        ),
        name: None,
        kind: SourceKind::Other,
        uri,
        tags,
        segments: vec![Segment { text: raw.body }],
    })
}

impl Guest for GitHub {
    fn identify(
        locator: Option<String>,
        options: Vec<(String, String)>,
    ) -> Result<SourceRef, SourceError> {
        if locator
            .as_deref()
            .is_some_and(|value| value.is_empty() || value.chars().any(char::is_control))
        {
            return Err(SourceError::InvalidInput(
                "GitHub locator must be a nonempty label".into(),
            ));
        }
        let mut validated = BTreeMap::new();
        for (key, value) in options {
            if validated.contains_key(&key) {
                return Err(SourceError::InvalidInput(format!(
                    "duplicate GitHub option {key}"
                )));
            }
            let value = match key.as_str() {
                "include" | "exclude" | "wiki_include" | "wiki_exclude" => {
                    let entries = list(&value).map_err(SourceError::InvalidInput)?;
                    if entries.iter().any(|value| !pattern(value)) {
                        return Err(SourceError::InvalidInput(format!(
                            "{key} needs owner/repo patterns (for example org/*)"
                        )));
                    }
                    entries.join(",")
                }
                "labels" | "topics" | "paths" => {
                    list(&value).map_err(SourceError::InvalidInput)?.join(",")
                }
                "issues" | "pulls" | "comments" | "reviews" | "metadata" | "wiki" => {
                    match value.trim() {
                        "true" | "false" => value.trim().into(),
                        _ => {
                            return Err(SourceError::InvalidInput(format!(
                                "{key} must be true or false"
                            )));
                        }
                    }
                }
                "since" => {
                    since(&value).map_err(SourceError::InvalidInput)?;
                    value.trim().into()
                }
                "account" => {
                    if value.trim().is_empty() || value.chars().any(char::is_control) {
                        return Err(SourceError::InvalidInput(
                            "account must be a nonempty label".into(),
                        ));
                    }
                    value.trim().into()
                }
                _ => {
                    return Err(SourceError::InvalidInput(format!(
                        "unknown GitHub option {key}"
                    )));
                }
            };
            validated.insert(key, value);
        }
        if !validated.contains_key("include") {
            return Err(SourceError::InvalidInput(
                "include needs at least one owner/repo pattern".into(),
            ));
        }
        let mut source = SourceRef {
            source: "github".into(),
            account: None,
            // A locator can label a separate account/server fixture; every request still uses fixed GitHub origins.
            locator: locator.unwrap_or_else(|| "github.com".into()),
            options: validated.clone().into_iter().collect(),
        };
        let wiki = enabled(&source, "wiki", true);
        if !["issues", "pulls", "comments", "reviews", "metadata"]
            .iter()
            .any(|key| {
                enabled(
                    &source,
                    key,
                    matches!(*key, "issues" | "pulls" | "metadata"),
                )
            })
            && (!wiki || option(&source, "wiki_include").is_none())
        {
            return Err(SourceError::InvalidInput(
                "enable at least one content kind or wiki_include".into(),
            ));
        }
        if option(&source, "wiki_exclude").is_some() && option(&source, "wiki_include").is_none() {
            return Err(SourceError::InvalidInput(
                "wiki_exclude needs wiki_include".into(),
            ));
        }
        if option(&source, "wiki").is_some_and(|value| value == "true")
            && option(&source, "wiki_include").is_none()
        {
            return Err(SourceError::InvalidInput(
                "wiki=true needs wiki_include".into(),
            ));
        }
        if option(&source, "paths").is_some() && option(&source, "wiki_include").is_none() {
            return Err(SourceError::InvalidInput("paths needs wiki_include".into()));
        }
        if option(&source, "labels").is_some()
            && !enabled(&source, "issues", true)
            && !enabled(&source, "pulls", true)
        {
            return Err(SourceError::InvalidInput(
                "labels needs issues or pulls enabled".into(),
            ));
        }
        if enabled(&source, "reviews", false) && !enabled(&source, "pulls", true) {
            return Err(SourceError::InvalidInput(
                "reviews=true needs pulls=true".into(),
            ));
        }
        if enabled(&source, "comments", false)
            && !enabled(&source, "issues", true)
            && !enabled(&source, "pulls", true)
        {
            return Err(SourceError::InvalidInput(
                "comments=true needs issues=true or pulls=true".into(),
            ));
        }
        let account = validated
            .get("account")
            .cloned()
            .unwrap_or_else(|| "github.com".into());
        // A selector changes the set of documents: fold its canonical values into identity, not just run options.
        let selectors: Vec<_> = validated
            .iter()
            .filter(|(key, _)| key.as_str() != "since")
            .collect();
        source.account = Some(format!(
            "{account}:{}",
            digest(&serde_json::to_string(&selectors).unwrap())
        ));
        Ok(source)
    }

    fn default_wing(_source: SourceRef) -> String {
        "github".into()
    }
    fn default_room() -> String {
        "repositories".into()
    }

    fn discover(source: SourceRef, position: String, limit: u32) -> Result<Discovery, SourceError> {
        let mut state = cursor(&position)?;
        let repos = repositories(&source).map_err(SourceError::Failed)?;
        let api = discover_api(&source, &repos).map_err(SourceError::Failed)?;
        let wiki_include = if enabled(&source, "wiki", true) {
            entries(&source, "wiki_include")
        } else {
            Vec::new()
        };
        let wiki_exclude = entries(&source, "wiki_exclude");
        if !wiki_include.is_empty()
            && !repos
                .iter()
                .any(|(repo, _)| matches(&wiki_include, repo) && !matches(&wiki_exclude, repo))
        {
            return Err(SourceError::InvalidInput(
                "wiki_include matches no selected repository; check include, exclude and wiki_exclude"
                    .into(),
            ));
        }
        let paths = entries(&source, "paths");
        let boundary = option(&source, "since")
            .map(since)
            .transpose()
            .map_err(SourceError::InvalidInput)?;
        let mut found = Vec::new();
        let max = usize::try_from(limit).unwrap_or(usize::MAX);
        let mut more = false;
        for entry in api {
            if state
                .api
                .get(&entry.handle.repo)
                .is_some_and(|after| &entry.key <= after)
            {
                continue;
            }
            if found.len() == max {
                more = true;
                break;
            }
            state.api.insert(entry.handle.repo.clone(), entry.key);
            found.push(Candidate {
                external_id: entry.external_id,
                cursor_after: serde_json::to_string(&state).unwrap(),
                handle: serde_json::to_string(&entry.handle).unwrap(),
            });
        }
        if !more {
            for (repo, _) in repos {
                if !matches(&wiki_include, &repo) || matches(&wiki_exclude, &repo) {
                    continue;
                }
                let revisions = wiki::revisions(&repo).map_err(SourceError::Failed)?;
                let previous = state.wiki.get(&repo).cloned();
                if let Some(previous) = &previous
                    && !revisions
                        .iter()
                        .any(|r| format!("{}:{}", r.commit, r.path) == *previous)
                {
                    return Err(SourceError::CursorInvalid(format!(
                        "wiki history for {repo} changed; run with --full"
                    )));
                }
                let mut passed = previous.is_none();
                for revision in revisions {
                    let key = format!("{}:{}", revision.commit, revision.path);
                    if !passed {
                        if previous.as_deref() == Some(&key) {
                            passed = true;
                        }
                        continue;
                    }
                    if boundary.is_some_and(|since| {
                        timestamp(&revision.occurred_at).is_ok_and(|at| at < since)
                    }) || !paths.is_empty()
                        && !matches(&paths, &revision.path.to_ascii_lowercase())
                    {
                        continue;
                    }
                    if found.len() == max {
                        more = true;
                        break;
                    }
                    state.wiki.insert(repo.clone(), key);
                    let external_id = format!("{repo}/wiki/{}@{}", revision.path, revision.commit);
                    found.push(Candidate {
                        external_id,
                        cursor_after: serde_json::to_string(&state).unwrap(),
                        handle: serde_json::to_string(&Handle {
                            kind: "wiki".into(),
                            repo: repo.clone(),
                            id: None,
                            number: None,
                            updated: revision.occurred_at.clone(),
                            wiki: Some(revision),
                        })
                        .unwrap(),
                    });
                }
                if more {
                    break;
                }
            }
        }
        Ok(Discovery {
            candidates: found,
            exhausted: !more,
        })
    }

    fn read(_source: SourceRef, candidate: Candidate) -> Result<Option<RawDocument>, SourceError> {
        let handle: Handle = serde_json::from_str(&candidate.handle)
            .map_err(|_| SourceError::Failed("invalid GitHub candidate".into()))?;
        let (body, title, url, labels, topics, status) = if let Some(revision) = &handle.wiki {
            let body = wiki::page(revision).map_err(SourceError::Failed)?;
            let page = revision
                .path
                .strip_suffix(".md")
                .unwrap_or(&revision.path)
                .replace(' ', "-");
            (
                body,
                Some(revision.path.clone()),
                format!("https://github.com/{}/wiki/{page}", revision.repo),
                vec![],
                vec![],
                None,
            )
        } else {
            let path = match handle.kind.as_str() {
                "repository" => format!("/repos/{}", handle.repo),
                "issue" | "pull" => format!(
                    "/repos/{}/{}/{}",
                    handle.repo,
                    if handle.kind == "pull" {
                        "pulls"
                    } else {
                        "issues"
                    },
                    handle.number.unwrap_or_default()
                ),
                "comment" => format!(
                    "/repos/{}/issues/comments/{}",
                    handle.repo,
                    handle.id.unwrap_or_default()
                ),
                "review-comment" => format!(
                    "/repos/{}/pulls/comments/{}",
                    handle.repo,
                    handle.id.unwrap_or_default()
                ),
                "review" => format!(
                    "/repos/{}/pulls/{}/reviews/{}",
                    handle.repo,
                    handle.number.unwrap_or_default(),
                    handle.id.unwrap_or_default()
                ),
                _ => return Err(SourceError::Failed("unknown GitHub candidate kind".into())),
            };
            let record = api::request(&path).map_err(SourceError::Failed)?;
            let title = record
                .get("title")
                .or_else(|| record.get("name"))
                .and_then(Value::as_str)
                .map(str::to_string);
            let url = record
                .get("html_url")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let labels = record
                .get("labels")
                .and_then(Value::as_array)
                .map(|labels| {
                    labels
                        .iter()
                        .filter_map(|label| label.get("name").and_then(Value::as_str))
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            let topics = record
                .get("topics")
                .and_then(Value::as_array)
                .map(|topics| {
                    topics
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            let status = record
                .get("state")
                .and_then(Value::as_str)
                .map(str::to_string);
            let actor = record
                .pointer("/user/login")
                .and_then(Value::as_str)
                .map(str::to_string);
            let mut body = record
                .get("body")
                .or_else(|| record.get("description"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if let Some(title) = &title {
                body = format!("# {title}\n\n{body}");
            }
            // An approval with no written comment still has meaning; keep its state and author in its text.
            if handle.kind == "review" {
                body = format!(
                    "Review: {} by {}\n\n{body}",
                    status.as_deref().unwrap_or("submitted"),
                    actor.as_deref().unwrap_or("unknown")
                );
            }
            (body, title, url, labels, topics, status)
        };
        let metadata = json!({"kind":handle.kind,"repo":handle.repo,"title":title,
            "url":url,"labels":labels,"topics":topics,"state":status,"number":handle.number,
            "occurred_at":handle.updated,"commit":handle.wiki.as_ref().map(|r| &r.commit),
            "path":handle.wiki.as_ref().map(|r| &r.path)});
        // A changed label or URL is a changed document even when the Markdown body is the same.
        let revision = digest(&format!("{body}\0{metadata}"));
        Ok(Some(RawDocument {
            external_id: candidate.external_id,
            revision,
            body,
            metadata: metadata.to_string(),
            occurred_at: Some(handle.updated),
        }))
    }

    fn normalize(raw: RawDocument) -> Result<CanonicalDocument, SourceError> {
        normalized(raw)
    }
}

export!(GitHub);

#[cfg(test)]
mod tests {
    use super::*;

    fn source(options: &[(&str, &str)]) -> SourceRef {
        GitHub::identify(
            None,
            std::iter::once(("include", "acme/alpha"))
                .chain(options.iter().copied())
                .map(|(key, value)| (key.into(), value.into()))
                .collect(),
        )
        .unwrap()
    }

    fn discover_all(source: &SourceRef) -> Discovery {
        GitHub::discover(source.clone(), "null".into(), 100).unwrap()
    }

    #[test]
    fn rejects_invalid_options_and_unsafe_scopes() {
        for options in [
            vec![],
            vec![("include", "acme/alpha"), ("include", "acme/beta")],
            vec![("include", "acme/")],
            vec![("include", "acme/alpha,")],
            vec![("include", "acme/alpha"), ("unknown", "yes")],
            vec![("include", "acme/alpha"), ("issues", "yes")],
            vec![("include", "acme/alpha"), ("since", "tomorrow")],
            vec![("include", "acme/alpha"), ("wiki_exclude", "acme/alpha")],
            vec![("include", "acme/alpha"), ("wiki", "yes")],
            vec![("include", "acme/alpha"), ("wiki", "true")],
            vec![("include", "acme/alpha"), ("paths", "Home.md")],
            vec![
                ("include", "acme/alpha"),
                ("issues", "false"),
                ("pulls", "false"),
                ("labels", "wanted"),
            ],
            vec![
                ("include", "acme/alpha"),
                ("pulls", "false"),
                ("reviews", "true"),
            ],
            vec![
                ("include", "acme/alpha"),
                ("issues", "false"),
                ("pulls", "false"),
                ("comments", "true"),
            ],
        ] {
            assert!(matches!(
                GitHub::identify(
                    None,
                    options
                        .into_iter()
                        .map(|(k, v)| (k.into(), v.into()))
                        .collect()
                ),
                Err(SourceError::InvalidInput(_))
            ));
        }
        assert!(matches!(
            GitHub::identify(
                Some("\n".into()),
                vec![("include".into(), "acme/alpha".into())]
            ),
            Err(SourceError::InvalidInput(_))
        ));
        assert_eq!(GitHub::default_room(), "repositories");
        assert_eq!(GitHub::default_wing(source(&[])), "github");
        assert_eq!(since("2026-01").unwrap(), since("2026-01-01").unwrap());
        assert!(glob("acme/alph?", "acme/alpha"));
        assert!(!glob("acme/*", "acme/nested/repo"));
    }

    #[test]
    fn selectors_change_identity_but_since_does_not() {
        let base = source(&[]);
        assert_eq!(base.account, source(&[("since", "2026-01")]).account);
        assert_ne!(base.account, source(&[("labels", "wanted")]).account);
        assert_eq!(
            source(&[("labels", "WANTED, docs")]).account,
            source(&[("labels", "docs,wanted")]).account
        );
        assert!(
            source(&[
                ("issues", "false"),
                ("pulls", "false"),
                ("metadata", "false"),
                ("wiki_include", "acme/alpha")
            ])
            .account
            .is_some()
        );
    }

    #[test]
    fn cursor_rejects_malformed_and_legacy_positions() {
        assert!(cursor("null").unwrap().api.is_empty());
        for value in [
            "{",
            "{}",
            r#"{"other":1}"#,
            r#"{"api":{"bad":["2026-01-01T00:00:00Z","1"]},"wiki":{}}"#,
            r#"{"api":{},"wiki":{"acme/alpha":"bad"}}"#,
        ] {
            assert!(
                matches!(cursor(value), Err(SourceError::CursorInvalid(_))),
                "{value}"
            );
        }
    }

    #[test]
    fn discovery_paginates_without_repeating_candidates() {
        let source = source(&[]);
        let zero = GitHub::discover(source.clone(), "null".into(), 0).unwrap();
        assert!(zero.candidates.is_empty());
        assert!(!zero.exhausted);
        let mut position = "null".to_string();
        let mut ids = Vec::new();
        loop {
            let page = GitHub::discover(source.clone(), position.clone(), 1).unwrap();
            for candidate in &page.candidates {
                ids.push(candidate.external_id.clone());
                position = candidate.cursor_after.clone();
            }
            if page.exhausted {
                break;
            }
        }
        assert_eq!(
            ids,
            [
                "acme/alpha/issue/10",
                "acme/alpha/repository/1",
                "acme/alpha/pull/20"
            ]
        );
        assert!(discover_all(&source).exhausted);
        assert!(matches!(
            GitHub::discover(source, "{}".into(), 1),
            Err(SourceError::CursorInvalid(_))
        ));
    }

    #[test]
    fn filters_repositories_and_documents() {
        let selected = source(&[
            ("labels", "wanted"),
            ("topics", "docs"),
            ("comments", "true"),
            ("reviews", "true"),
        ]);
        let ids: Vec<_> = discover_all(&selected)
            .candidates
            .into_iter()
            .map(|c| c.external_id)
            .collect();
        assert!(ids.contains(&"acme/alpha/comment/31".into()));
        assert!(ids.contains(&"acme/alpha/review-comment/41".into()));
        assert!(ids.contains(&"acme/alpha/review/51".into()));
        assert_eq!(
            discover_all(&source(&[("issues", "false"), ("comments", "true")]))
                .candidates
                .len(),
            2
        );
        assert_eq!(
            discover_all(&source(&[("since", "2026-01-02"), ("metadata", "false")]))
                .candidates
                .len(),
            1
        );
        assert_eq!(
            discover_all(&source(&[("metadata", "false"), ("issues", "false")]))
                .candidates
                .len(),
            1
        );
        assert!(matches!(
            GitHub::discover(source(&[("topics", "missing")]), "null".into(), 10),
            Err(SourceError::Failed(_))
        ));
        assert!(matches!(
            GitHub::discover(source(&[("exclude", "acme/*")]), "null".into(), 10),
            Err(SourceError::Failed(_))
        ));
    }

    #[test]
    fn wiki_history_filters_and_resumes() {
        let wiki_source = source(&[("wiki_include", "acme/alpha"), ("paths", "home.md")]);
        let without_wiki = source(&[
            ("wiki_include", "acme/alpha"),
            ("wiki_exclude", "acme/alpha"),
            ("paths", "Home.md"),
            ("wiki", "false"),
        ]);
        assert_eq!(discover_all(&without_wiki).candidates.len(), 3);
        assert_ne!(wiki_source.account, without_wiki.account);
        let first = discover_all(&wiki_source);
        assert_eq!(first.candidates.len(), 5);
        let wiki = &first.candidates[3..];
        assert!(wiki.iter().all(|c| c.external_id.contains("Home.md")));
        let raw = GitHub::read(wiki_source.clone(), wiki[0].clone())
            .unwrap()
            .unwrap();
        assert_eq!(raw.body, "# Home v1\n");
        assert_eq!(
            GitHub::normalize(raw).unwrap().room.as_deref(),
            Some("wikis")
        );
        let last = wiki.last().unwrap().cursor_after.clone();
        assert!(
            GitHub::discover(wiki_source.clone(), last, 10)
                .unwrap()
                .candidates
                .is_empty()
        );
        let mut stale: Value = serde_json::from_str(&wiki[0].cursor_after).unwrap();
        stale["wiki"]["acme/alpha"] = json!(format!("{}:Missing.md", "a".repeat(40)));
        assert!(matches!(
            GitHub::discover(wiki_source.clone(), stale.to_string(), 10),
            Err(SourceError::CursorInvalid(_))
        ));
        assert!(matches!(
            GitHub::discover(source(&[("wiki_include", "other/*")]), "null".into(), 10),
            Err(SourceError::InvalidInput(_))
        ));
        assert!(matches!(
            GitHub::discover(
                source(&[
                    ("wiki_include", "acme/alpha"),
                    ("wiki_exclude", "acme/alpha")
                ]),
                "null".into(),
                10
            ),
            Err(SourceError::InvalidInput(_))
        ));
    }

    #[test]
    fn reads_each_api_kind_and_normalizes_metadata() {
        let source = source(&[("comments", "true"), ("reviews", "true")]);
        let discovered = discover_all(&source);
        assert_eq!(discovered.candidates.len(), 6);
        for candidate in discovered.candidates {
            let raw = GitHub::read(source.clone(), candidate.clone())
                .unwrap()
                .unwrap();
            assert_eq!(raw.external_id, candidate.external_id);
            assert_eq!(raw.revision.len(), 64);
            let canonical = GitHub::normalize(raw).unwrap();
            assert!(canonical.tags.contains(&"github".into()));
            assert_eq!(canonical.segments.len(), 1);
        }
        let review = discover_all(&source)
            .candidates
            .into_iter()
            .find(|c| c.external_id.contains("/review/"))
            .unwrap();
        assert!(
            GitHub::read(source.clone(), review)
                .unwrap()
                .unwrap()
                .body
                .starts_with("Review: submitted by unknown")
        );
        let invalid = Candidate {
            external_id: "invalid".into(),
            cursor_after: "null".into(),
            handle: "{".into(),
        };
        assert!(matches!(
            GitHub::read(source.clone(), invalid),
            Err(SourceError::Failed(_))
        ));
        let unknown = Candidate { external_id: "unknown".into(), cursor_after: "null".into(), handle: json!({"kind":"mystery","repo":"acme/alpha","number":null,"id":null,"wiki":null,"updated":"2026-01-01T00:00:00Z"}).to_string() };
        assert!(matches!(
            GitHub::read(source, unknown),
            Err(SourceError::Failed(_))
        ));
    }

    #[test]
    fn normalization_rejects_missing_metadata_and_maps_rooms() {
        let raw = |metadata: &str| RawDocument {
            external_id: "id".into(),
            revision: "revision".into(),
            body: "body".into(),
            metadata: metadata.into(),
            occurred_at: None,
        };
        for value in ["{", "{}", r#"{"kind":"issue"}"#] {
            assert!(matches!(
                GitHub::normalize(raw(value)),
                Err(SourceError::Failed(_))
            ));
        }
        for (kind, room) in [
            ("repository", "repositories"),
            ("issue", "issues"),
            ("pull", "pulls"),
            ("review", "pulls"),
            ("comment", "issues"),
            ("wiki", "wikis"),
        ] {
            let doc = GitHub::normalize(raw(&json!({"kind":kind,"repo":"acme/alpha","labels":["wanted"],"topics":["docs"],"title":"Title","url":"https://github.com"}).to_string())).unwrap();
            assert_eq!(doc.room.as_deref(), Some(room));
            assert!(doc.tags.contains(&"label:wanted".into()));
            assert!(doc.tags.contains(&"topic:docs".into()));
            assert_eq!(doc.title.as_deref(), Some("Title"));
        }
    }

    #[test]
    fn an_org_glob_does_not_cross_repository_or_owner_boundaries() {
        assert!(glob("org/*", "org/repo"));
        assert!(glob("org/repo-*", "org/repo-one"));
        assert!(!glob("org/*", "other/repo"));
        assert!(!glob("org/*", "org/repo/subpath"));
    }

    #[test]
    fn independent_scope_patterns_have_independent_source_identifiers() {
        let options = |include: &str| vec![("include".into(), include.into())];
        let first = GitHub::identify(None, options("org/a,org/b")).unwrap();
        let same = GitHub::identify(None, options("org/b, org/a")).unwrap();
        let other = GitHub::identify(None, options("org/a")).unwrap();
        assert_eq!(first.account, same.account);
        assert_ne!(first.account, other.account);
    }
}
