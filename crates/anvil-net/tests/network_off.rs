// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Once networking is off, a request fails without connecting. This runs in
//! its own process because the switch can't be undone.

use std::net::TcpListener;
use std::time::Duration;

#[test]
fn a_process_with_networking_off_never_connects() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!(
        "http://127.0.0.1:{}/",
        listener.local_addr().unwrap().port()
    );

    assert!(!anvil_net::network_is_off());
    anvil_net::turn_off_network();
    assert!(anvil_net::network_is_off());

    let err = anvil_net::fetch(&url).unwrap_err();
    assert!(err.to_string().contains("no network"), "{err}");
    std::thread::sleep(Duration::from_millis(50));
    assert!(listener.accept().is_err(), "nothing may connect");
}
