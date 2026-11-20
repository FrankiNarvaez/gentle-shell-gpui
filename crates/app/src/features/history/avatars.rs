//! Commit authors' pictures for the Git pane, found as Tern finds them
//! (`gentle_core::git::avatar`): a GitHub noreply address, else GitHub's API
//! for the repository's commits by that address, else Gravatar. Each one is
//! downloaded once and kept as a 64 px PNG in `<data dir>/avatars/`, named by
//! the address's SHA-256; an address with no picture is remembered for a
//! week. A network failure stops looking up for the rest of the session, and
//! GitHub's rate limit stops only the API.

use std::collections::{HashMap, VecDeque};
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use gentle_core::git::avatar::{
    SIZE, github_commits_url, github_noreply_url, gravatar_url, normalize_email,
    parse_commits_avatar,
};
use gpui_kit::*;

const USER_AGENT: &str = concat!("gentle-shell-gpui/", env!("CARGO_PKG_VERSION"));
/// Lookups running at once.
const PARALLEL: usize = 4;
/// How long "no picture" holds before the address is asked about again.
const MISSING_FOR: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// A picture larger than this isn't one.
const MAX_BYTES: u64 = 2 * 1024 * 1024;

enum Known {
    Pending,
    Ready(PathBuf),
    Missing,
}

/// The pictures known this session; views observe it to redraw when one
/// arrives.
#[derive(Default)]
pub(super) struct Avatars {
    known: HashMap<String, Known>,
    queue: VecDeque<(String, Option<(String, String)>)>,
    running: usize,
    /// The network failed: nothing more is asked this session.
    offline: bool,
    /// GitHub's API refused (rate limit): only Gravatar is asked after.
    no_github_api: bool,
}

impl Global for Avatars {}

/// The picture of `email`, once it is on disk.
pub(super) fn picture(email: &str, cx: &App) -> Option<PathBuf> {
    match cx
        .try_global::<Avatars>()?
        .known
        .get(&normalize_email(email))?
    {
        Known::Ready(path) => Some(path.clone()),
        Known::Pending | Known::Missing => None,
    }
}

/// Look up the pictures of `emails` not asked about yet; `github` is the
/// repository whose commits name the accounts.
pub(super) fn request<'a>(
    emails: impl IntoIterator<Item = &'a str>,
    github: Option<(String, String)>,
    cx: &mut App,
) {
    if crate::shared::data_dir::app().is_none() {
        return;
    }
    let avatars = cx.default_global::<Avatars>();
    for email in emails {
        let email = normalize_email(email);
        if email.is_empty() || avatars.known.contains_key(&email) {
            continue;
        }
        avatars.known.insert(email.clone(), Known::Pending);
        avatars.queue.push_back((email, github.clone()));
    }
    pump(cx);
}

/// What one lookup found.
enum Found {
    Picture(PathBuf),
    Nothing,
    /// The network is down: no answer either way.
    Offline,
}

struct Lookup {
    found: Found,
    /// GitHub's API refused to answer (its rate limit).
    github_refused: bool,
}

/// Start lookups until `PARALLEL` run.
fn pump(cx: &mut App) {
    let Some(dir) = crate::shared::data_dir::app().map(|d| d.join("avatars")) else {
        return;
    };
    loop {
        let avatars = cx.global_mut::<Avatars>();
        if avatars.offline || avatars.running >= PARALLEL {
            return;
        }
        let Some((email, github)) = avatars.queue.pop_front() else {
            return;
        };
        avatars.running += 1;
        let github = github.filter(|_| !avatars.no_github_api);
        let dir = dir.clone();
        cx.spawn(async move |cx| {
            let job_email = email.clone();
            let lookup = cx
                .background_executor()
                .spawn(async move { resolve(&dir, &job_email, github.as_ref()) })
                .await;
            cx.update(|cx| {
                cx.update_global::<Avatars, _>(|avatars, _| {
                    avatars.running -= 1;
                    avatars.no_github_api |= lookup.github_refused;
                    let known = match lookup.found {
                        Found::Picture(path) => Known::Ready(path),
                        Found::Nothing => Known::Missing,
                        Found::Offline => {
                            avatars.offline = true;
                            Known::Missing
                        }
                    };
                    avatars.known.insert(email, known);
                });
                pump(cx);
            });
        })
        .detach();
    }
}

/// The picture of `email` from the disk, else from the network.
fn resolve(dir: &Path, email: &str, github: Option<&(String, String)>) -> Lookup {
    use sha2::{Digest as _, Sha256};
    let hash: String = Sha256::digest(email.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let picture = dir.join(format!("{hash}.png"));
    let missing = dir.join(format!("{hash}.none"));
    let done = |found| Lookup {
        found,
        github_refused: false,
    };
    if picture.is_file() {
        return done(Found::Picture(picture));
    }
    let fresh = std::fs::metadata(&missing)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|at| SystemTime::now().duration_since(at).ok())
        .is_some_and(|age| age < MISSING_FOR);
    if fresh {
        return done(Found::Nothing);
    }

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(15)))
        .build()
        .into();
    let mut github_refused = false;
    let mut urls: Vec<String> = Vec::new();
    if let Some(url) = github_noreply_url(email) {
        urls.push(url);
    } else if let Some((owner, repo)) = github {
        match get(&agent, &github_commits_url(owner, repo, email), true) {
            Answer::Body(json) => {
                if let Some(url) = parse_commits_avatar(&String::from_utf8_lossy(&json)) {
                    urls.push(url);
                }
            }
            Answer::Refused => github_refused = true,
            Answer::NotFound => {}
            Answer::Offline => {
                return Lookup {
                    found: Found::Offline,
                    github_refused,
                };
            }
        }
    }
    urls.push(gravatar_url(&hash));
    for url in urls {
        match get(&agent, &url, false) {
            Answer::Body(bytes) => {
                if let Some(path) = save(dir, &picture, &bytes) {
                    return Lookup {
                        found: Found::Picture(path),
                        github_refused,
                    };
                }
            }
            Answer::Offline => {
                return Lookup {
                    found: Found::Offline,
                    github_refused,
                };
            }
            Answer::Refused | Answer::NotFound => {}
        }
    }
    let _ = std::fs::create_dir_all(dir).and_then(|()| std::fs::write(&missing, b""));
    Lookup {
        found: Found::Nothing,
        github_refused,
    }
}

enum Answer {
    Body(Vec<u8>),
    /// 404 and other "no".
    NotFound,
    /// 403 or 429: a rate limit.
    Refused,
    Offline,
}

fn get(agent: &ureq::Agent, url: &str, github_api: bool) -> Answer {
    let mut request = agent.get(url).header("User-Agent", USER_AGENT);
    if github_api {
        request = request
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
    }
    let Ok(response) = request.call() else {
        return Answer::Offline;
    };
    match response.status().as_u16() {
        200 => {
            let mut bytes = Vec::new();
            match response
                .into_body()
                .into_reader()
                .take(MAX_BYTES)
                .read_to_end(&mut bytes)
            {
                Ok(_) => Answer::Body(bytes),
                Err(_) => Answer::Offline,
            }
        }
        403 | 429 => Answer::Refused,
        _ => Answer::NotFound,
    }
}

/// Decode `bytes`, scale them to `SIZE` and write a PNG at `path` (through a
/// temporary file, so a half-written one is never read).
fn save(dir: &Path, path: &Path, bytes: &[u8]) -> Option<PathBuf> {
    let image = image::load_from_memory(bytes).ok()?;
    let image = image.resize_to_fill(SIZE, SIZE, image::imageops::FilterType::Lanczos3);
    std::fs::create_dir_all(dir).ok()?;
    let tmp = path.with_extension("png.tmp");
    image.save_with_format(&tmp, image::ImageFormat::Png).ok()?;
    std::fs::rename(&tmp, path).ok()?;
    Some(path.to_path_buf())
}
