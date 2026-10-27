// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Cookie partitions, blocked cookie hosts, and HTTPS-only.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;

use anvil_net::{
    CookieJar, HttpsOnlyPolicy, NetError, enter_https_only, fetch, http_get_with_cookie,
    https_only_policy, parse_url, site_for_url,
};

fn under(jar: &CookieJar, top: &str) -> Option<String> {
    jar.http_cookie_header_under("https://cdn.tracker.example/pixel", top, Some(top), false)
}

#[test]
fn cookies_are_partitioned_by_the_top_level_site() {
    let mut jar = CookieJar::new();
    let tracker = "https://cdn.tracker.example/pixel";
    assert!(jar.store_set_cookie_under(
        tracker,
        "https://www.news.test/story",
        "id=news; Secure; SameSite=None"
    ));
    assert!(jar.store_set_cookie_under(
        tracker,
        "https://shop.test/",
        "id=shop; Secure; SameSite=None"
    ));
    assert_eq!(jar.len(), 2);
    assert_eq!(jar.cookies()[0].partition, "news.test");

    assert_eq!(
        under(&jar, "https://news.test/other").as_deref(),
        Some("id=news")
    );
    assert_eq!(
        under(&jar, "https://shop.test/cart").as_deref(),
        Some("id=shop")
    );
    assert_eq!(under(&jar, "https://blog.test/"), None);
    // Loading the tracker itself only sees its own partition.
    assert_eq!(jar.http_cookie_header(tracker, None, true), None);

    // A first-party cookie keeps working on its own site.
    assert!(jar.store_set_cookie("https://shop.test/login", "session=abc; Path=/"));
    assert_eq!(
        jar.http_cookie_header("https://shop.test/account", None, true)
            .as_deref(),
        Some("session=abc")
    );
    assert_eq!(
        jar.http_cookie_header_under(
            "https://shop.test/api",
            "https://news.test/",
            Some("https://news.test/"),
            false
        ),
        None
    );
    assert_eq!(
        jar.script_cookie_string("https://shop.test/account"),
        "session=abc"
    );

    // Deleting with Max-Age only touches the matching partition.
    assert!(jar.store_set_cookie_under(
        tracker,
        "https://news.test/",
        "id=x; Secure; SameSite=None; Max-Age=0"
    ));
    assert_eq!(under(&jar, "https://news.test/"), None);
    assert_eq!(
        under(&jar, "https://shop.test/").as_deref(),
        Some("id=shop")
    );
}

#[test]
fn partitions_survive_a_snapshot_and_old_snapshots_get_their_own_site() {
    let mut jar = CookieJar::new();
    jar.store_set_cookie_under(
        "https://a.widget.test/",
        "https://host.test/",
        "w=1; Secure; SameSite=None",
    );
    let restored = CookieJar::from_json(&jar.to_json()).unwrap();
    assert_eq!(restored, jar);

    let old = r#"{"cookies":[{"name":"k","value":"v","domain":"www.old.test","host_only":true,"path":"/","secure":false,"http_only":false,"same_site":"Lax"}]}"#;
    let restored = CookieJar::from_json(old).unwrap();
    assert_eq!(restored.cookies()[0].partition, "old.test");
    assert_eq!(
        restored
            .http_cookie_header("http://www.old.test/", None, true)
            .as_deref(),
        Some("k=v")
    );
    assert_eq!(
        site_for_url("https://deep.sub.example.org/x").as_deref(),
        Some("example.org")
    );
}

#[test]
fn blocked_hosts_neither_store_nor_send_cookies() {
    let mut jar = CookieJar::new();
    assert!(jar.store_set_cookie("https://www.ads.test/", "a=1"));
    assert!(jar.store_set_cookie("https://fine.test/", "b=2"));
    jar.set_blocked_hosts(&["ads.test".to_string()]);
    assert_eq!(
        jar.len(),
        1,
        "stored cookies for a blocked host are removed"
    );
    assert!(jar.is_host_blocked("x.ads.test"));
    assert!(!jar.is_host_blocked("notads.test"));
    assert!(!jar.store_set_cookie("https://x.ads.test/", "c=3"));
    assert_eq!(
        jar.http_cookie_header("https://ads.test/", None, true),
        None
    );
    assert_eq!(
        jar.http_cookie_header("https://fine.test/", None, true)
            .as_deref(),
        Some("b=2")
    );
    jar.set_blocked_hosts(&[]);
    assert!(jar.store_set_cookie("https://ads.test/", "c=3"));
    assert_eq!(jar.remove_host("ads.test"), 1);
    assert_eq!(jar.len(), 1);
}

#[test]
fn https_only_policy_upgrades_http_except_for_chosen_hosts() {
    let policy = HttpsOnlyPolicy::new(true, &["Intranet.Test".to_string()]);
    assert_eq!(
        policy.upgrade("http://example.test/a?b=c").as_deref(),
        Some("https://example.test/a?b=c")
    );
    assert_eq!(
        policy.upgrade("http://example.test:8080/").as_deref(),
        Some("https://example.test:8080/")
    );
    assert_eq!(policy.upgrade("https://example.test/"), None);
    assert_eq!(policy.upgrade("http://intranet.test/"), None);
    assert!(policy.allows_http("intranet.test"));
    assert!(policy.allows_http("wiki.intranet.test"));
    assert!(!policy.allows_http("notintranet.test"));
    assert!(!policy.allows_http("example.test"));
    assert_eq!(
        HttpsOnlyPolicy::default().upgrade("http://example.test/"),
        None
    );

    assert!(!https_only_policy().enabled);
    {
        let _scope = enter_https_only(policy.clone());
        assert_eq!(https_only_policy(), policy);
        {
            let _inner = enter_https_only(HttpsOnlyPolicy::default());
            assert!(!https_only_policy().enabled);
        }
        assert!(https_only_policy().enabled);
    }
    assert!(!https_only_policy().enabled);
}

/// Plain HTTP server that records the first bytes of every connection.
fn recording_server() -> (u16, Arc<Mutex<Vec<Vec<u8>>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut buf = [0u8; 2048];
            let n = stream.read(&mut buf).unwrap_or(0);
            log.lock().unwrap().push(buf[..n].to_vec());
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nplain",
            );
        }
    });
    (port, seen)
}

#[test]
fn https_only_never_falls_back_to_plain_http() {
    let (port, seen) = recording_server();
    let url = format!("http://127.0.0.1:{port}/page");

    {
        let _scope = enter_https_only(HttpsOnlyPolicy::new(true, &[]));
        match fetch(&url) {
            Err(NetError::HttpsOnly(original)) => assert_eq!(original, url),
            other => panic!("expected an HTTPS-only error, got {other:?}"),
        }
        // A direct request is refused before it connects.
        let parsed = parse_url(&url).unwrap();
        assert!(matches!(
            http_get_with_cookie(&parsed, None),
            Err(NetError::HttpsOnly(_))
        ));
    }
    {
        let log = seen.lock().unwrap();
        assert_eq!(log.len(), 1, "only the TLS attempt reached the server");
        assert!(!log[0].starts_with(b"GET "), "no plain request was sent");
    }

    // With the host allowed, plain http works.
    let _scope = enter_https_only(HttpsOnlyPolicy::new(true, &["127.0.0.1".to_string()]));
    let (final_url, response) = fetch(&url).unwrap();
    assert_eq!(final_url, url);
    assert_eq!(response.body, b"plain");
    assert!(
        seen.lock()
            .unwrap()
            .last()
            .unwrap()
            .starts_with(b"GET /page")
    );
}
