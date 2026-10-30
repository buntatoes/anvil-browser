// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// ArrayBuffer and typed arrays wrap on store and share the buffer.
// needs: collections
// expect: 1
// expect: 255
// expect: -1
// expect: 248
// expect: 63
// expect: 8
export function main() {
  const u8 = new Uint8Array(2);
  u8[0] = 257;
  u8[1] = -1;
  const i32 = new Int32Array(1);
  i32[0] = 4294967295;
  const buf = new ArrayBuffer(8);
  const f = new Float64Array(buf);
  f[0] = 1.5;
  const bytes = new Uint8Array(buf);
  return [u8[0], u8[1], i32[0], bytes[6], bytes[7], buf.byteLength];
}
