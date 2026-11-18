// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Targeted integration tests for ECMAScript standard library fidelity:
//! arbitrary-precision `BigInt`, native `ArrayBuffer`/`TypedArray`/`DataView`,
//! lazy `Continuation` generators, async `Promise` combinators, Gregorian `Date`,
//! UTF-16 `String`/`RegExp`, `Function` constructor, `encodeURI`, and `JSON`.

use cinder::compiler::{install, run_script};
use cinder::limits::Limits;
use cinder::vm::{BasicHost, Vm};

fn eval_out(src: &str) -> String {
    let mut vm = Vm::new(Limits::default()).expect("vm");
    install(&mut vm).expect("install");
    let mut host = BasicHost::default();
    let _ = run_script(&mut vm, &mut host, src).expect("run");
    vm.drain_jobs(&mut host).expect("drain");
    host.console
        .iter()
        .map(|(_, line)| line.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn arbitrary_precision_bigint_and_bit_ops_work() {
    let out = eval_out(
        r#"
        const big = 1n << 100n;
        const snowflake = (192045678901234567n + 1n) * 3n;
        console.log(big.toString(16));
        console.log(snowflake.toString());
        console.log(BigInt.asUintN(64, -1n).toString(16));
        console.log(BigInt.asIntN(8, 255n).toString());
        "#,
    );
    assert_eq!(
        out,
        "10000000000000000000000000\n576137036703703704\nffffffffffffffff\n-1"
    );
}

#[test]
fn native_typed_arrays_and_dataview_work() {
    let out = eval_out(
        r#"
        const ab = new ArrayBuffer(16);
        const u8 = new Uint8Array(ab);
        u8.set([0x40, 0x09, 0x21, 0xfb], 0);
        const dv = new DataView(ab);
        dv.setFloat32(4, 3.5, true);
        const f32 = new Float32Array(ab, 4, 1);
        dv.setBigUint64(8, 0x0102030405060708n, false);
        const bi = dv.getBigUint64(8, false);
        const c = new Uint8ClampedArray([ -5, 300, 12.6, 2.5, 3.5 ]);
        console.log(f32[0], bi.toString(16), [...c].join(','), ArrayBuffer.isView(u8), Object.prototype.toString.call(u8));
        "#,
    );
    assert_eq!(
        out,
        "3.5 102030405060708 0,255,13,2,4 true [object Uint8Array]"
    );
}

#[test]
fn lazy_sync_generators_unlimited_yield_sent_and_delegate_work() {
    let out = eval_out(
        r#"
        function* naturals() {
            let i = 0;
            while (true) {
                yield i++;
            }
        }
        const g = naturals();
        let last = -1;
        for (let i = 0; i < 1500; i++) {
            last = g.next().value;
        }
        function* echo() {
            const a = yield 'first';
            const b = yield ('got:' + a);
            return b;
        }
        const e = echo();
        const r1 = e.next().value;
        const r2 = e.next(42).value;
        const r3 = e.next('done');
        function* outer() {
            const innerRet = yield* (function*() { yield 10; yield 20; return 'ok'; })();
            yield innerRet;
        }
        console.log(last, r1, r2, r3.value, r3.done, [...outer()].join(','));
        "#,
    );
    assert_eq!(out, "1499 first got:42 done true 10,20,ok");
}

#[test]
fn async_promise_combinators_settle_pending_promises() {
    let out = eval_out(
        r#"
        let resolveSlow, resolveFast, rejectAny;
        const slow = new Promise(r => { resolveSlow = r; });
        const fast = new Promise(r => { resolveFast = r; });
        const fail = new Promise((_, rej) => { rejectAny = rej; });
        Promise.race([slow, fast]).then(v => console.log('race:' + v));
        Promise.allSettled([fail, fast]).then(res => console.log('allSettled:' + res[0].status + ',' + res[1].value));
        Promise.any([fail, slow]).then(v => console.log('any:' + v));
        resolveFast('B');
        rejectAny('E');
        resolveSlow('A');
        "#,
    );
    assert_eq!(out, "race:B\nallSettled:rejected,B\nany:A");
}

#[test]
fn date_uri_utf16_json_function_and_math_random_work() {
    let out = eval_out(
        r#"
        const d = new Date(Date.UTC(2026, 10, 17, 15, 30, 45, 123));
        const iso = d.toISOString();
        const parsed = new Date('2026-02-01T00:00:00Z').getUTCMonth();
        const addFn = new Function('a', 'b', 'return a + b');
        const uri = encodeURI('https://example.com/a b?k=1#x');
        const pair = '\u{1F600}x';
        const m = /x/.exec(pair);
        const rep = 'a1a2'.replaceAll('a', (match, off) => match + off);
        const parsedJson = JSON.parse('{"a":1,"b":2}', (k, v) => k === 'a' ? v * 10 : v);
        const strJson = JSON.stringify(parsedJson, ['b', 'a'], 2);
        const r = Math.random();
        console.log(iso, parsed, addFn(4, 5), uri, decodeURI(uri) === 'https://example.com/a b?k=1#x', pair.indexOf('x'), m.index, rep, r >= 0 && r < 1);
        console.log(strJson);
        "#,
    );
    let expected = concat!(
        "2026-11-17T15:30:45.123Z 1 9 https://example.com/a%20b?k=1#x true 2 2 a01a22 true\n",
        "{\n  \"b\": 2,\n  \"a\": 10\n}"
    );
    assert_eq!(out, expected);
}
