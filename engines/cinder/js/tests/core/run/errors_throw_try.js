// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// throw, try/catch and the error constructors.
// needs: errors
// expect: RangeError
// expect: bad
// expect: true
export function main() {
  let name = "";
  let message = "";
  let isError = false;
  try {
    throw new RangeError("bad");
  } catch (e) {
    name = e.name;
    message = e.message;
    isError = e instanceof Error;
  }
  return [name, message, isError];
}
