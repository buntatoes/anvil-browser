// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
function f() {
  try { throw 1; } catch (e) { return () => e; }
  finally { let z = 1; }
}
try {} catch { var q = 1; }
