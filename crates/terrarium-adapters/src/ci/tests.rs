//! The CI adapter on fictional runs from `tests/fixtures/ci/`, and the real client against a
//! server on the loopback interface: no test reaches the network.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use super::*;

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/ci")
        .join(format!("{name}.json"));
    std::fs::read_to_string(path).unwrap()
}

fn verdict_of(name: &str) -> Option<Verdict> {
    let runs: Runs = serde_json::from_str(&fixture(name)).unwrap();
    verdict(&runs.workflow_runs)
}

const A: &str = "a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1";

#[test]
fn the_newest_decided_commit_gives_the_verdict() {
    let pass = verdict_of("pass").unwrap();
    assert!(pass.passed);
    assert_eq!((pass.sha.as_str(), pass.workflows), (A, 2));
    assert!(pass.failing.is_empty());

    let fail = verdict_of("fail").unwrap();
    assert!(!fail.passed);
    assert_eq!(fail.failing, [".github/workflows/ci.yml"]);

    // A failure is final while another workflow still runs.
    let early = verdict_of("fail-while-running").unwrap();
    assert!(!early.passed);

    // The newest commit still runs: the one before it decides, its failure included.
    let older = verdict_of("pending-then-older").unwrap();
    assert_eq!((older.passed, older.sha.as_str()), (false, A));
    assert_eq!(older.failing, [".github/workflows/lint.yml"]);

    // A re-run that passed replaces the attempt that failed.
    let rerun = verdict_of("rerun").unwrap();
    assert!(rerun.passed);

    // Nothing but cancelled and skipped, or no run at all: nothing to go by.
    assert_eq!(verdict_of("silent"), None);
    assert_eq!(verdict_of("empty"), None);
}

#[test]
fn a_certificate_not_trusted_is_told_apart_from_no_answer() {
    let refused = || rustls::Error::InvalidCertificate(rustls::CertificateError::UnknownIssuer);
    assert_eq!(
        client::unreached(&ureq::Error::Rustls(refused())),
        CiError::Certificate
    );
    // As the TLS stream reports it, inside an I/O error.
    let io = std::io::Error::new(std::io::ErrorKind::InvalidData, refused());
    assert_eq!(
        client::unreached(&ureq::Error::Io(io)),
        CiError::Certificate
    );
    for other in [
        ureq::Error::ConnectionFailed,
        ureq::Error::HostNotFound,
        ureq::Error::Rustls(rustls::Error::DecryptError),
        ureq::Error::Io(std::io::Error::from(std::io::ErrorKind::TimedOut)),
    ] {
        assert_eq!(client::unreached(&other), CiError::Unreachable, "{other}");
    }
    assert!(CiError::Certificate.is_transient());
}

#[test]
fn a_failing_workflow_s_name_is_cut_to_a_path_s_length() {
    let run = |id: u64, name: &str| Run {
        id,
        workflow_id: id,
        head_sha: A.to_owned(),
        status: Some("completed".into()),
        conclusion: Some("failure".into()),
        run_attempt: None,
        path: None,
        name: Some(name.to_owned()),
    };
    // Each "é" is two bytes, so the cut falls inside one and backs off to its start.
    let long = format!("x{}", "é".repeat(2_000));
    let failing = verdict(&[run(1, &long), run(2, "lint")]).unwrap().failing;
    assert_eq!(failing.len(), 2);
    let cut = failing.iter().find(|name| name.starts_with('x')).unwrap();
    assert_eq!(cut.len(), 1_023);
    assert!(long.starts_with(cut.as_str()));
    assert!(failing.contains(&"lint".to_owned()));
}

#[test]
fn a_verdict_changes_its_hash_only_with_what_decided_it() {
    let pass = verdict_of("pass").unwrap();
    assert_eq!(verdict_of("pass").unwrap().hash, pass.hash);
    assert_ne!(verdict_of("fail").unwrap().hash, pass.hash);
    assert_ne!(verdict_of("rerun").unwrap().hash, pass.hash);
}

#[test]
fn a_repository_is_read_from_its_remote_on_github_only() {
    let slug = |url: &str| Slug::from_remote_url(url).map(|slug| slug.to_string());
    let moss = Some("example-garden/moss-bed".to_owned());
    for url in [
        "https://github.com/example-garden/moss-bed.git",
        "https://github.com/example-garden/moss-bed",
        "https://github.com/example-garden/moss-bed/",
        "https://someone@github.com/example-garden/moss-bed.git",
        "git@github.com:example-garden/moss-bed.git",
        "github.com:example-garden/moss-bed",
        "ssh://git@github.com/example-garden/moss-bed.git",
        "ssh://git@ssh.github.com:443/example-garden/moss-bed.git",
        "  https://GitHub.com/example-garden/moss-bed.git  ",
    ] {
        assert_eq!(slug(url), moss, "{url}");
    }
    for url in [
        "https://gitlab.com/example-garden/moss-bed.git",
        "https://github.example.com/example-garden/moss-bed.git",
        "git@gitlab.com:example-garden/moss-bed.git",
        "/srv/git/moss-bed.git",
        "./moss-bed",
        "https://github.com/example-garden",
        "https://github.com/example-garden/moss-bed/tree/main",
        "https://github.com/example-garden/..",
        "file:///srv/git/moss-bed",
    ] {
        assert_eq!(slug(url), None, "{url}");
    }
}

#[test]
fn a_typed_repository_takes_githubs_characters_only() {
    let parse = |text: &str| Slug::parse(text).map(|slug| slug.to_string());
    assert_eq!(
        parse(" example-garden/moss.bed_2 "),
        Some("example-garden/moss.bed_2".into())
    );
    assert_eq!(
        parse("example-garden/moss-bed.git"),
        Some("example-garden/moss-bed".into())
    );
    for bad in [
        "",
        "moss-bed",
        "example-garden/",
        "/moss-bed",
        "example garden/moss",
        "a/b/c",
        "example-garden/..",
        "example-garden/.",
        "ex_ample/moss",
        "ex@mple/moss",
        "o/m#x",
    ] {
        assert_eq!(parse(bad), None, "{bad:?}");
    }
    let stored: Slug = serde_json::from_str(r#""example-garden/moss-bed""#).unwrap();
    assert_eq!(
        serde_json::to_string(&stored).unwrap(),
        r#""example-garden/moss-bed""#
    );
    assert!(serde_json::from_str::<Slug>(r#""no slash""#).is_err());
}

#[test]
fn every_name_in_the_runs_url_is_encoded() {
    let slug = Slug::parse("example-garden/moss.bed").unwrap();
    assert_eq!(
        runs_path(&slug, "feat/a b#c&d=e+f%g?h"),
        "/repos/example-garden/moss.bed/actions/runs\
         ?branch=feat%2Fa%20b%23c%26d%3De%2Bf%25g%3Fh&per_page=30&exclude_pull_requests=true"
    );
    assert!(runs_path(&slug, "fougère/été").contains("branch=foug%C3%A8re%2F%C3%A9t%C3%A9&"));
    assert!(runs_path(&slug, "v1.0_rc-2~x").contains("branch=v1.0_rc-2~x&"));
}

#[test]
fn the_wait_between_polls_follows_how_the_last_went() {
    let every = Duration::from_secs(60);
    let mut backoff = Backoff::new(every);
    let minutes = |m: u64| Some(Duration::from_secs(m * 60));
    assert_eq!(backoff.after(Ok(()), 0), minutes(1));
    // Unreachable: doubling to ten minutes, back to one on an answer.
    let waits: Vec<_> = (0..6)
        .map(|_| backoff.after(Err(CiError::Unreachable), 0))
        .collect();
    assert_eq!(
        waits,
        [
            minutes(1),
            minutes(2),
            minutes(4),
            minutes(8),
            minutes(10),
            minutes(10)
        ]
    );
    assert_eq!(backoff.after(Ok(()), 0), minutes(1));
    // What GitHub asks for, never sooner than a minute.
    let limited = |retry_after, reset_at| CiError::RateLimited {
        retry_after,
        reset_at,
    };
    assert_eq!(backoff.after(Err(limited(Some(300), None)), 0), minutes(5));
    assert_eq!(backoff.after(Err(limited(Some(5), None)), 0), minutes(1));
    assert_eq!(
        backoff.after(Err(limited(None, Some(1_000_600))), 1_000_000),
        minutes(10)
    );
    backoff.reset();
    let unsaid: Vec<_> = (0..6)
        .map(|_| backoff.after(Err(limited(None, None)), 0))
        .collect();
    assert_eq!(
        unsaid,
        [
            minutes(1),
            minutes(2),
            minutes(4),
            minutes(8),
            minutes(15),
            minutes(15)
        ]
    );
    assert_eq!(backoff.after(Err(CiError::NotFound), 0), minutes(15));
    assert_eq!(backoff.after(Err(CiError::Refused), 0), None);
    // A shorter period scales the same way, as the engine's tests run it.
    let mut quick = Backoff::new(Duration::from_millis(10));
    assert_eq!(
        quick.after(Err(CiError::Server(502)), 0),
        Some(Duration::from_millis(10))
    );
    assert_eq!(
        quick.after(Err(CiError::Server(502)), 0),
        Some(Duration::from_millis(20))
    );
}

#[test]
fn the_token_never_shows_itself() {
    let secret = "github_pat_fictional0123456789";
    let token = Token::new(&format!("  {secret}\n")).unwrap();
    assert_eq!(token.secret(), secret);
    assert!(!format!("{token:?}").contains("fictional"));
    for bad in ["", "   ", "two words", "tab\there", "nul\0"] {
        assert_eq!(Token::new(bad), None, "{bad:?}");
    }
    let keychain = MemoryKeychain::holding(secret);
    assert!(!format!("{:?}", keychain.token()).contains("fictional"));
    for err in [
        CiError::Unreachable,
        CiError::Refused,
        CiError::NotFound,
        CiError::RateLimited {
            retry_after: Some(1),
            reset_at: Some(2),
        },
        CiError::Server(500),
        CiError::Malformed,
    ] {
        assert!(!format!("{err:?}").contains("fictional"));
    }
    assert!(user_agent().starts_with("repo-terrarium/"));
    assert!(!user_agent().contains('@'));
    assert!(!user_agent().contains(' '));
}

#[test]
fn the_real_client_never_follows_a_proxy_or_a_redirect_and_speaks_only_https() {
    let github = GitHub::new();
    let config = github.agent().config();
    assert!(config.proxy().is_none());
    assert_eq!(config.max_redirects(), 0);
    assert!(config.https_only());
    assert!(!config.http_status_as_error());
    // A proxy the environment names is dropped: started from a builder that holds one.
    let named =
        ureq::Agent::config_builder().proxy(Some(ureq::Proxy::new("http://127.0.0.1:9").unwrap()));
    assert!(client::settled_over(named).build().proxy().is_none());
}

/// A server's address, and the requests it was sent.
struct Served {
    requests: Arc<Mutex<Vec<String>>>,
    base: String,
}

/// A server on the loopback interface that answers each request, in turn, with the next of
/// `responses` (status line and headers, then a body), and keeps each request's head.
fn serve(responses: Vec<(String, String)>) -> Served {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let kept = Arc::clone(&requests);
    thread::spawn(move || {
        for (head, body) in responses {
            let Ok((stream, _)) = listener.accept() else {
                return;
            };
            answer(stream, &head, &body, &kept);
        }
    });
    Served { requests, base }
}

fn answer(mut stream: TcpStream, head: &str, body: &str, kept: &Mutex<Vec<String>>) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request = String::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
            break;
        }
        request.push_str(&line);
    }
    kept.lock().unwrap().push(request);
    let response = format!(
        "{head}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let mut rest = Vec::new();
    let _ = stream.set_read_timeout(Some(Duration::from_millis(50)));
    let _ = reader.read_to_end(&mut rest);
}

fn ok(etag: &str, body: &str) -> (String, String) {
    (
        format!("HTTP/1.1 200 OK\r\netag: {etag}\r\ncontent-type: application/json"),
        body.to_owned(),
    )
}

fn status(line: &str, headers: &str) -> (String, String) {
    let head = if headers.is_empty() {
        format!("HTTP/1.1 {line}")
    } else {
        format!("HTTP/1.1 {line}\r\n{headers}")
    };
    (head, String::new())
}

#[test]
fn the_client_asks_with_its_headers_and_an_unchanged_list_costs_a_304() {
    let served = serve(vec![
        ok("\"w1\"", &fixture("fail")),
        status("304 Not Modified", "etag: \"w1\""),
        ok("\"w2\"", &fixture("pass")),
        ok("\"w3\"", &fixture("pass")),
    ]);
    let mut github = GitHub::on_loopback(&served.base);
    let slug = Slug::parse("example-garden/moss-bed").unwrap();
    let token = Token::new("github_pat_fictional").unwrap();
    let first = github.runs(&slug, "feat/moss", &token).unwrap().unwrap();
    assert!(!first.passed);
    let again = github.runs(&slug, "feat/moss", &token).unwrap().unwrap();
    assert_eq!(again, first, "a 304 gives the verdict kept");
    let changed = github.runs(&slug, "feat/moss", &token).unwrap().unwrap();
    assert!(changed.passed);
    // Forgotten when the token changes: no tag is sent.
    github.forget();
    github.runs(&slug, "feat/moss", &token).unwrap();
    let requests: Vec<String> = served
        .requests
        .lock()
        .unwrap()
        .iter()
        .map(|request| request.to_ascii_lowercase())
        .collect();
    assert_eq!(requests.len(), 4);
    assert!(requests[0].starts_with(
        "get /repos/example-garden/moss-bed/actions/runs?branch=feat%2fmoss&per_page=30"
    ));
    assert!(requests[0].contains("authorization: bearer github_pat_fictional\r\n"));
    assert!(requests[0].contains("accept: application/vnd.github+json\r\n"));
    assert!(requests[0].contains("x-github-api-version: 2022-11-28\r\n"));
    assert!(requests[0].contains(&format!("user-agent: {}\r\n", user_agent())));
    assert!(!requests[0].contains("if-none-match"));
    assert!(requests[1].contains("if-none-match: \"w1\"\r\n"));
    assert!(requests[2].contains("if-none-match: \"w1\"\r\n"));
    assert!(!requests[3].contains("if-none-match"));
}

#[test]
fn githubs_refusals_limits_and_failures_say_which() {
    let reset = "x-ratelimit-remaining: 0\r\nx-ratelimit-reset: 1700000600";
    let cases = [
        (status("401 Unauthorized", ""), CiError::Refused),
        (
            status("403 Forbidden", "x-ratelimit-remaining: 41"),
            CiError::Refused,
        ),
        (
            status("403 Forbidden", reset),
            CiError::RateLimited {
                retry_after: None,
                reset_at: Some(1_700_000_600),
            },
        ),
        (
            status("429 Too Many Requests", "retry-after: 120"),
            CiError::RateLimited {
                retry_after: Some(120),
                reset_at: None,
            },
        ),
        (
            (
                "HTTP/1.1 403 Forbidden".to_owned(),
                r#"{"message":"You have exceeded a secondary rate limit."}"#.to_owned(),
            ),
            CiError::RateLimited {
                retry_after: None,
                reset_at: None,
            },
        ),
        (status("404 Not Found", ""), CiError::NotFound),
        (status("502 Bad Gateway", ""), CiError::Server(502)),
        (ok("\"m\"", "<html>not json</html>"), CiError::Malformed),
        (
            status(
                "301 Moved Permanently",
                "location: https://elsewhere.example/runs",
            ),
            CiError::NotFound,
        ),
    ];
    for (response, expected) in cases {
        let served = serve(vec![response]);
        let mut github = GitHub::on_loopback(&served.base);
        let slug = Slug::parse("example-garden/moss-bed").unwrap();
        let token = Token::new("github_pat_fictional").unwrap();
        assert_eq!(github.runs(&slug, "main", &token), Err(expected));
        assert_eq!(served.requests.lock().unwrap().len(), 1, "{expected:?}");
    }
}

#[test]
fn a_renamed_repository_is_followed_once_back_to_the_api() {
    // The move names the same server, as GitHub's names api.github.com.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let kept = Arc::clone(&requests);
    let moved = format!("location: {base}/repositories/42/actions/runs?branch=main");
    let body = fixture("pass");
    thread::spawn(move || {
        let responses = [
            status("301 Moved Permanently", &moved),
            ok("\"r\"", &body),
            status("304 Not Modified", "etag: \"r\""),
        ];
        for (head, body) in responses {
            let Ok((stream, _)) = listener.accept() else {
                return;
            };
            answer(stream, &head, &body, &kept);
        }
    });
    let mut github = GitHub::on_loopback(&base);
    let slug = Slug::parse("example-garden/moss-bed").unwrap();
    let token = Token::new("github_pat_fictional").unwrap();
    assert!(github.runs(&slug, "main", &token).unwrap().unwrap().passed);
    // The next poll asks the new address with its tag, and a 304 keeps the verdict.
    assert!(github.runs(&slug, "main", &token).unwrap().unwrap().passed);
    let requests = requests.lock().unwrap().clone();
    let moved_to = requests[1].to_ascii_lowercase();
    assert!(moved_to.starts_with("get /repositories/42/actions/runs"));
    assert!(moved_to.contains("authorization: bearer"));
    let again = requests[2].to_ascii_lowercase();
    assert!(
        again.starts_with("get /repositories/42/actions/runs"),
        "{again}"
    );
    assert!(again.contains("if-none-match: \"r\"\r\n"));
}

#[test]
fn a_closed_port_is_unreachable() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let mut github = GitHub::on_loopback(&base);
    let slug = Slug::parse("example-garden/moss-bed").unwrap();
    let token = Token::new("github_pat_fictional").unwrap();
    assert_eq!(
        github.runs(&slug, "main", &token),
        Err(CiError::Unreachable)
    );
}

#[test]
fn the_memory_keychain_saves_removes_and_counts_its_reads() {
    let keychain = MemoryKeychain::default();
    assert_eq!(keychain.token(), Ok(None));
    keychain.save(&Token::new("t0k3n").unwrap()).unwrap();
    assert_eq!(keychain.held().as_deref(), Some("t0k3n"));
    keychain.remove().unwrap();
    keychain.remove().unwrap();
    assert_eq!(keychain.token(), Ok(None));
    assert_eq!(keychain.reads(), 2);
}
