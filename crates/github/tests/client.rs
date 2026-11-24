//! The client against a fake GitHub: a local HTTP server that answers as
//! GitHub's API does for acme/app#1, and records the writes.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;

use diffr_github::{Client, NewComment, Repo, Side, VERDICT_APPROVE};

struct Request {
    method: String,
    path: String,
    query: String,
    host: String,
    body: String,
}

/// Serves requests on a local port until the test ends, one a connection.
fn serve(
    handler: impl Fn(&Request) -> (u16, Vec<(String, String)>, String) + Send + Sync + 'static,
) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let handler = Arc::new(handler);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let handler = handler.clone();
            thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    return;
                }
                let mut parts = line.split_whitespace();
                let method = parts.next().unwrap_or_default().to_string();
                let target = parts.next().unwrap_or_default().to_string();
                let (path, query) = target.split_once('?').unwrap_or((&target, ""));
                let mut len = 0;
                let mut host = String::new();
                loop {
                    let mut h = String::new();
                    if reader.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
                        break;
                    }
                    if let Some((k, v)) = h.split_once(':') {
                        match k.trim().to_lowercase().as_str() {
                            "content-length" => len = v.trim().parse().unwrap_or(0),
                            "host" => host = v.trim().to_string(),
                            _ => {}
                        }
                    }
                }
                let mut body = vec![0; len];
                reader.read_exact(&mut body).unwrap();
                let req = Request {
                    method,
                    path: path.to_string(),
                    query: query.to_string(),
                    host,
                    body: String::from_utf8_lossy(&body).into_owned(),
                };
                let (status, headers, body) = handler(&req);
                let mut out = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
                    body.len()
                );
                for (k, v) in headers {
                    out.push_str(&format!("{k}: {v}\r\n"));
                }
                out.push_str("\r\n");
                out.push_str(&body);
                let _ = stream.write_all(out.as_bytes());
            });
        }
    });
    format!("http://{addr}")
}

#[test]
fn client() {
    let writes: Arc<Mutex<Vec<String>>> = Arc::default();
    let recorded = writes.clone();
    let api = serve(move |r| {
        // Writes, which GraphQL's queries are not.
        if r.method != "GET" && (r.path != "/graphql" || r.body.contains("mutation")) {
            recorded
                .lock()
                .unwrap()
                .push(format!("{} {} {}", r.method, r.path, r.body));
        }
        let ok = |b: &str| (200, vec![], b.to_string());
        let page = r.query.split('&').any(|kv| kv.starts_with("page="));
        match (r.method.as_str(), r.path.as_str()) {
            (_, "/repos/acme/app/pulls/1") => ok(
                r#"{"number":1,"title":"Greet","body":null,"state":"open","user":{"login":"ada"},"base":{"ref":"main","sha":"b"},"head":{"ref":"greet","sha":"h"},"changed_files":2,"created_at":"2026-01-02T03:04:05Z","merged_at":null}"#,
            ),
            (_, "/repos/acme/app/pulls/1/comments") if !page => (
                200,
                vec![(
                    "Link".into(),
                    format!(
                        r#"<http://{}/repos/acme/app/pulls/1/comments?page=2&per_page=100>; rel="next""#,
                        r.host
                    ),
                )],
                r#"[{"id":1,"path":"a.go","side":"RIGHT","line":3,"user":{"login":"bo"},"body":"Why?"}]"#.into(),
            ),
            (_, "/repos/acme/app/pulls/1/comments") => ok(
                r#"[{"id":2,"in_reply_to_id":1,"path":"a.go","side":"RIGHT","line":3,"user":{"login":"ada"},"body":"Because."}]"#,
            ),
            ("GET", "/repos/acme/app/pulls/1/reviews") => {
                ok(r#"[{"id":7,"node_id":"PRR_7","state":"PENDING","user":{"login":"ada"}}]"#)
            }
            (_, "/repos/acme/app/pulls/1/reviews/7/comments") => ok(
                r#"[{"id":5,"path":"b.go","side":"RIGHT","line":1,"user":{"login":"ada"},"body":"Draft"}]"#,
            ),
            (_, "/graphql") if r.body.contains("reviewThreads") => ok(
                r#"{"data":{"repository":{"pullRequest":{"reviewThreads":{"pageInfo":{"hasNextPage":false},"nodes":[{"id":"T1","isResolved":false,"comments":{"nodes":[{"fullDatabaseId":"1"}]}}]}}}}}"#,
            ),
            (_, "/graphql") if r.body.contains("search(") => ok(
                r#"{"data":{"search":{"nodes":[{"number":4,"title":"Add","isDraft":true,"author":{"login":"cy"},"reviewDecision":"APPROVED","commits":{"nodes":[{"commit":{"statusCheckRollup":{"state":"FAILURE"}}}]},"additions":3},{}]}}}"#,
            ),
            (_, "/graphql") => ok(r#"{"data":{}}"#),
            ("GET", "/repos/acme/app/pulls") => ok(
                r#"[{"number":4,"title":"Add","draft":false,"user":{"login":"cy"},"head":{"ref":"add"},"labels":[{"name":"bug","color":"d73a4a"}]}]"#,
            ),
            ("GET", "/user") => ok(r#"{"login":"ada","avatar_url":"https://a/ada.png"}"#),
            (m, _) if m != "GET" => ok("{}"),
            _ => (404, vec![], r#"{"message":"Not Found"}"#.into()),
        }
    });
    let repo = Repo::new("acme", "app");
    let c = Client::new("t").with_api(&api);

    let p = c.pull(&repo, 1).unwrap();
    assert_eq!(p.title, "Greet");
    assert_eq!(p.head.sha, "h");
    assert_eq!(p.status(), "Open");
    assert_eq!(p.body, "");
    assert_eq!(p.created_at.unwrap().year(), 2026);

    let r = c.reviews(&repo, 1).unwrap();
    assert_eq!(r.threads.len(), 2, "{r:?}");
    assert_eq!(r.threads[0].comments.len(), 2);
    assert_eq!(r.threads[0].node_id, "T1");
    assert!(r.threads[1].pending);
    assert!(r.pending.is_some());

    let pulls = c.open_pulls(&repo).unwrap();
    assert_eq!(pulls.len(), 1);
    assert!(pulls[0].draft);
    assert_eq!(pulls[0].review_decision, "approved");
    assert_eq!(pulls[0].checks, "failure");
    assert_eq!(pulls[0].author.login, "cy");
    let rest = Client::new("").with_api(&api).open_pulls(&repo).unwrap();
    assert_eq!(rest.len(), 1);
    assert_eq!(rest[0].head_ref, "add");
    assert_eq!(rest[0].labels[0].name, "bug");
    // Without a token, GraphQL is not asked: a write is refused at once.
    let err = Client::new("")
        .with_api(&api)
        .resolve("T1", true)
        .unwrap_err();
    assert!(err.is_forbidden());

    assert_eq!(c.viewer().unwrap().login, "ada");

    let nc = NewComment {
        body: "Hm".into(),
        commit_id: "h".into(),
        path: "a.go".into(),
        side: Side::Right,
        line: 5,
        start_line: 3,
    };
    c.add_to_review(&repo, 1, None, &nc).unwrap();
    c.add_to_review(&repo, 1, r.pending.as_ref(), &nc).unwrap();
    c.submit_review(&repo, 1, r.pending.as_ref(), VERDICT_APPROVE, "Ship it")
        .unwrap();
    c.reply(&repo, 1, 1, "Sure").unwrap();
    c.edit_comment(&repo, 2, "Because!").unwrap();
    c.delete_comment(&repo, 2).unwrap();
    c.resolve("T1", true).unwrap();
    c.comment(&repo, 1, &nc).unwrap();
    c.discard_review(&repo, 1, r.pending.as_ref().unwrap())
        .unwrap();

    let writes = writes.lock().unwrap().clone();
    assert_eq!(writes.len(), 9, "{writes:#?}");
    let started: serde_json::Value =
        serde_json::from_str(writes[0].splitn(3, ' ').nth(2).unwrap()).unwrap();
    assert!(writes[0].starts_with("POST /repos/acme/app/pulls/1/reviews "));
    assert_eq!(started["commit_id"], "h");
    assert_eq!(started["comments"][0]["start_line"], 3);
    assert_eq!(started["comments"][0]["start_side"], "RIGHT");
    assert!(writes[1].starts_with("POST /graphql "));
    assert!(writes[1].contains(r#""reviewId":"PRR_7""#), "{}", writes[1]);
    assert!(writes[1].contains(r#""startLine":3"#), "{}", writes[1]);
    assert!(writes[2].starts_with("POST /repos/acme/app/pulls/1/reviews/7/events "));
    assert!(writes[2].contains(r#""event":"APPROVE""#));
    assert!(writes[3].starts_with("POST /repos/acme/app/pulls/1/comments/1/replies "));
    assert!(writes[4].starts_with("PATCH /repos/acme/app/pulls/comments/2 "));
    assert!(writes[5].starts_with("DELETE /repos/acme/app/pulls/comments/2"));
    assert!(writes[6].starts_with("POST /graphql ") && writes[6].contains("resolveReviewThread"));
    assert!(writes[7].starts_with("POST /repos/acme/app/pulls/1/comments "));
    assert!(writes[7].contains(r#""commit_id":"h""#));
    assert!(writes[8].starts_with("DELETE /repos/acme/app/pulls/1/reviews/7"));

    let err = c.pull(&repo, 404).unwrap_err();
    assert!(err.to_string().contains("Not found"), "{err}");
    assert_eq!(err.status(), Some(404));
}
